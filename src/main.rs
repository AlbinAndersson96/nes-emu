mod app;
mod apu;
mod bus;
mod cartridge;
mod cpu;
mod input;
mod menu;
mod ppu;
mod renderer;
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

const FRAME_DURATION: Duration = Duration::from_nanos(16_666_667);

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

struct WinitApp {
    app: Option<App>,
    key_map: KeyMap,
    button_state: [u8; 2],
    modifiers: ModifiersState,
    next_frame: Instant,
    pending_rom_path: Option<PathBuf>,
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
        self.app = Some(app);
        self.next_frame = Instant::now();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
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
                event, is_synthetic, ..
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

                if let Some((port, bit)) = self.key_map.on_key(code) {
                    match event.state {
                        ElementState::Pressed => self.button_state[port] |= bit,
                        ElementState::Released => self.button_state[port] &= !bit,
                    }
                    app.set_controller_buttons(port, self.button_state[port]);
                }
            }

            WindowEvent::RedrawRequested if app.renderer.redraw(menu::draw) => {
                load_rom_via_dialog(app);
            }

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
    if args.len() > 2 {
        eprintln!("Usage: {} [rom.nes]", args[0]);
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
    };

    let event_loop = EventLoop::new().expect("failed to create event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop
        .run_app(&mut winit_app)
        .expect("event loop terminated with an error");
}
