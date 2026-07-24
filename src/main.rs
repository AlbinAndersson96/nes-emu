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

/// Resolve which `keybindings.toml` to load, in priority order:
///
/// 1. `$NES_EMU_KEYBINDINGS`, if set — an explicit path override.
/// 2. Debug builds: the source file `assets/keybindings.toml` in the crate,
///    read directly so edits take effect on the next `cargo run` (the copy
///    `build.rs` seeds into `target/<profile>/` is for release/installed use,
///    and is never overwritten once it exists).
/// 3. Release builds: `keybindings.toml` next to the executable (the seeded,
///    user-customizable copy).
///
/// A missing file at the resolved path is not fatal — `KeyMap::load` warns and
/// falls back to the built-in defaults.
fn keybindings_path() -> PathBuf {
    if let Some(path) = env::var_os("NES_EMU_KEYBINDINGS") {
        return PathBuf::from(path);
    }
    if cfg!(debug_assertions) {
        return PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/keybindings.toml");
    }
    env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("keybindings.toml")))
        .unwrap_or_else(|| PathBuf::from("keybindings.toml"))
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

fn save_state_via_dialog(app: &mut App) {
    let mut dialog = rfd::FileDialog::new().add_filter("Save state", &["state"]);
    // Pre-fill the dialog with the default `<rom>.state` location/name.
    if let Some(default) = app.default_state_path() {
        if let Some(dir) = default.parent() {
            dialog = dialog.set_directory(dir);
        }
        if let Some(name) = default.file_name().and_then(|n| n.to_str()) {
            dialog = dialog.set_file_name(name);
        }
    }
    if let Some(path) = dialog.save_file()
        && let Err(e) = app.save_state_to(&path)
    {
        eprintln!("error: cannot save state: {}", e);
    }
}

fn load_state_via_dialog(app: &mut App) {
    let mut dialog = rfd::FileDialog::new().add_filter("Save state", &["state"]);
    if let Some(default) = app.default_state_path()
        && let Some(dir) = default.parent()
    {
        dialog = dialog.set_directory(dir);
    }
    if let Some(path) = dialog.pick_file()
        && let Err(e) = app.load_state_from(&path)
    {
        eprintln!("error: cannot load state: {}", e);
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

                // Save states: the configured slot keys (default 0-9) select
                // the active slot; the save/load keys (default F5/F9) quick-save
                // and quick-load that slot's file next to the ROM (slot 0 =
                // `<rom>.state`, slot N = `<rom>.stateN`). All are configurable
                // via keybindings.toml's `[app]` section.
                if event.state == ElementState::Pressed {
                    if let Some(slot) = self.key_map.slot_for_key(code) {
                        app.select_slot(slot);
                        eprintln!("save-state slot {slot} selected");
                    }
                    if self.key_map.is_save_state(code) {
                        match app.save_state() {
                            Ok(()) => eprintln!("save state written to slot {}", app.active_slot()),
                            Err(e) => eprintln!("save state failed: {e}"),
                        }
                    }
                    if self.key_map.is_load_state(code) {
                        match app.load_state() {
                            Ok(()) => {
                                eprintln!("save state loaded from slot {}", app.active_slot())
                            }
                            Err(e) => eprintln!("load state failed: {e}"),
                        }
                    }

                    // Emulation speed: step through the slow-motion /
                    // fast-forward list, or jump back to 1x. The pacing loop in
                    // `about_to_wait` picks up the new multiplier on its next
                    // wakeup.
                    if self.key_map.is_speed_up(code) {
                        app.speed_up();
                        eprintln!("emulation speed {}", app.speed_label());
                    }
                    if self.key_map.is_slow_down(code) {
                        app.slow_down();
                        eprintln!("emulation speed {}", app.speed_label());
                    }
                    if self.key_map.is_normal_speed(code) {
                        app.reset_speed();
                        eprintln!("emulation speed {}", app.speed_label());
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

            WindowEvent::RedrawRequested => {
                let rom_loaded = app.is_rom_loaded();
                match app.renderer.redraw(|ui| menu::draw(ui, rom_loaded)) {
                    menu::MenuAction::LoadRom => load_rom_via_dialog(app),
                    menu::MenuAction::SaveState => save_state_via_dialog(app),
                    menu::MenuAction::LoadState => load_state_via_dialog(app),
                    menu::MenuAction::PlayReplay => load_replay_via_dialog(app),
                    menu::MenuAction::None => {}
                }
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(app) = self.app.as_mut() else {
            return;
        };
        // The wall-clock time one NES frame should take at the current speed:
        // slow motion stretches the interval (multiplier < 1), fast forward
        // shrinks it (multiplier > 1). At 1x this is exactly FRAME_DURATION.
        let interval = FRAME_DURATION.div_f64(app.speed_multiplier());
        let now = Instant::now();
        if now >= self.next_frame {
            app.step_frame();
            self.next_frame += interval;
            // If we've fallen behind — the host can't keep up at this speed, or
            // fast forward just shrank the interval past the old target — resync
            // to `now` so a backlog can't accumulate into a runaway catch-up.
            if self.next_frame <= now {
                self.next_frame = now + interval;
            }
        } else if self.next_frame > now + interval {
            // Speed just increased while we were already waiting: the previously
            // scheduled target is now too far out. Pull it in so the change
            // takes effect promptly instead of after the old (longer) wait.
            self.next_frame = now + interval;
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

    let key_map = KeyMap::load(&keybindings_path());

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
