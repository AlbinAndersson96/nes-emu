mod app;
mod apu;
mod bus;
mod cartridge;
mod cpu;
mod input;
mod menu;
mod ppu;
mod renderer;
mod replay;
mod savestate;
mod system;
#[cfg(test)]
mod tests;

use app::App;
use input::KeyMap;
use renderer::Renderer;
use std::{
    env, fs,
    path::PathBuf,
    process,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::{ElementState, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, ModifiersState},
    window::WindowId,
};

// NTSC NES refreshes at 60.0988 Hz, not the round 60 Hz — one frame is
// 16_639_267 ns (1 / 60.0988 s). Pacing at a flat 16_666_667 ns (60.000 Hz)
// ran the machine ~0.16% slow: the effective CPU clock became 29780.5 × 60.000
// = 1.7869 MHz instead of the real 1.789773 MHz. See the NTSC column of
// https://www.nesdev.org/wiki/Cycle_reference_chart.
const FRAME_DURATION: Duration = Duration::from_nanos(16_639_267);

fn maybe_configure_wsl2_gpu() {
    let version = match fs::read_to_string("/proc/version") {
        Ok(v) => v,
        Err(_) => return,
    };
    if !version.to_lowercase().contains("microsoft") {
        return;
    }
    // SAFETY: called before any threads are spawned
    unsafe {
        if env::var("WGPU_BACKEND").is_err() {
            env::set_var("WGPU_BACKEND", "vulkan");
        }
        if env::var("VK_ICD_FILENAMES").is_err() {
            let lvp = "/usr/share/vulkan/icd.d/lvp_icd.json";
            if std::path::Path::new(lvp).exists() {
                env::set_var("VK_ICD_FILENAMES", lvp);
            }
        }
        env::remove_var("WAYLAND_DISPLAY");
    }
}

fn load_rom_via_dialog(app: &mut App) {
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("NES ROM", &["nes"])
        .pick_file()
        && let Err(e) = app.load_rom(&path)
    {
        eprintln!("error: cannot load ROM: {}", e);
    }
}

fn load_replay_via_dialog(app: &mut App) {
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("FM2 replay", &["fm2"])
        .pick_file()
        && let Err(e) = app.load_replay(&path)
    {
        eprintln!("error: cannot load replay: {}", e);
    }
}

struct WinitApp {
    app: Option<App>,
    key_map: KeyMap,
    button_state: [u8; 2],
    modifiers: ModifiersState,
    next_frame: Instant,
    pending_rom_path: Option<PathBuf>,
    pending_replay_path: Option<PathBuf>,
}

impl ApplicationHandler for WinitApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.app.is_some() {
            return;
        }
        let renderer = match Renderer::new(event_loop) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("error: could not create window: {}", e);
                process::exit(1);
            }
        };
        let mut app = App::new(renderer);
        if let Some(rom_path) = self.pending_rom_path.take()
            && let Err(e) = app.load_rom(&rom_path)
        {
            eprintln!("error: invalid ROM: {}", e);
            process::exit(1);
        }
        if let Some(replay_path) = self.pending_replay_path.take()
            && let Err(e) = app.load_replay(&replay_path)
        {
            eprintln!("error: cannot load replay: {}", e);
        }
        self.app = Some(app);
        self.next_frame = Instant::now();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        let Some(app) = self.app.as_mut() else {
            return;
        };
        if window_id != app.renderer.window_id() {
            return;
        }

        let response = app.renderer.on_window_event(&event);
        if response.consumed && !matches!(event, WindowEvent::KeyboardInput { .. }) {
            return;
        }

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }

            WindowEvent::Resized(size) => {
                app.renderer.resize(size.width, size.height);
            }

            WindowEvent::ModifiersChanged(state) => {
                self.modifiers = state.state();
            }

            WindowEvent::DroppedFile(path) => {
                if let Err(e) = app.load_rom(&path) {
                    eprintln!("error: cannot load dropped ROM: {}", e);
                }
            }

            WindowEvent::KeyboardInput {
                event,
                is_synthetic,
                ..
            } => {
                if is_synthetic {
                    return;
                }
                let winit::keyboard::PhysicalKey::Code(code) = event.physical_key else {
                    return;
                };

                let is_ctrl_o = event.state == ElementState::Pressed
                    && code == KeyCode::KeyO
                    && self.modifiers.control_key();
                if is_ctrl_o {
                    load_rom_via_dialog(app);
                }

                let is_fps_toggle = event.state == ElementState::Pressed
                    && self.modifiers.control_key()
                    && self.key_map.is_fps_toggle(code);
                if is_fps_toggle {
                    app.toggle_fps_overlay();
                }

                // Save states: F5 quick-saves, F9 quick-loads (to/from
                // `<rom>.state` next to the ROM).
                if event.state == ElementState::Pressed {
                    match code {
                        KeyCode::F5 => match app.save_state() {
                            Ok(()) => eprintln!("save state written"),
                            Err(e) => eprintln!("save state failed: {e}"),
                        },
                        KeyCode::F9 => match app.load_state() {
                            Ok(()) => eprintln!("save state loaded"),
                            Err(e) => eprintln!("load state failed: {e}"),
                        },
                        _ => {}
                    }
                }

                if let Some((port, bit)) = self.key_map.on_key(code) {
                    match event.state {
                        ElementState::Pressed => self.button_state[port] |= bit,
                        ElementState::Released => self.button_state[port] &= !bit,
                    }
                    app.set_controller_buttons(port, self.button_state[port]);
                }
            }

            WindowEvent::RedrawRequested => match app.renderer.redraw(menu::draw) {
                menu::MenuAction::LoadRom => load_rom_via_dialog(app),
                menu::MenuAction::PlayReplay => load_replay_via_dialog(app),
                menu::MenuAction::None => {}
            },

            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(app) = self.app.as_mut() else {
            return;
        };
        if Instant::now() >= self.next_frame {
            self.next_frame += FRAME_DURATION;
            app.step_frame();
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(self.next_frame));
    }
}

fn main() {
    maybe_configure_wsl2_gpu();

    let args: Vec<String> = env::args().collect();
    if args.len() > 3 {
        eprintln!("Usage: {} [rom.nes] [replay.fm2]", args[0]);
        process::exit(1);
    }

    let key_map = env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("keybindings.toml")))
        .map_or_else(KeyMap::default, |path| KeyMap::load(&path));

    let mut winit_app = WinitApp {
        app: None,
        key_map,
        button_state: [0, 0],
        modifiers: ModifiersState::default(),
        next_frame: Instant::now(),
        pending_rom_path: args.get(1).map(PathBuf::from),
        pending_replay_path: args.get(2).map(PathBuf::from),
    };

    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop
        .run_app(&mut winit_app)
        .expect("event loop terminated with an error");
}
