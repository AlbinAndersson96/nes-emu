mod app;
mod apu;
mod bus;
mod cartridge;
mod cpu;
mod input;
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
use tao::{
    event::{ElementState, Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
    keyboard::{KeyCode, ModifiersState},
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

fn main() {
    maybe_configure_wsl2_gpu();

    let args: Vec<String> = env::args().collect();
    if args.len() > 2 {
        eprintln!("Usage: {} [rom.nes]", args[0]);
        process::exit(1);
    }

    let event_loop = EventLoop::new();
    let renderer = match Renderer::new(&event_loop) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: could not create window: {}", e);
            process::exit(1);
        }
    };

    let mut app = App::new(renderer);

    let key_map = env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("keybindings.toml")))
        .map_or_else(KeyMap::default, |path| KeyMap::load(&path));
    let mut button_state: [u8; 2] = [0, 0];

    if let Some(rom_path) = args.get(1) {
        if let Err(e) = app.load_rom(&PathBuf::from(rom_path)) {
            eprintln!("error: invalid ROM: {}", e);
            process::exit(1);
        }
    }

    let mut next_frame = Instant::now();
    let mut modifiers = ModifiersState::default();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(next_frame);

        match event {
            Event::WindowEvent {
                window_id,
                event: WindowEvent::CloseRequested,
                ..
            } if window_id == app.renderer.window_id() => {
                *control_flow = ControlFlow::Exit;
            }

            Event::WindowEvent {
                event: WindowEvent::Resized(size),
                ..
            } => {
                app.renderer.resize(size.width, size.height);
            }

            Event::WindowEvent {
                event: WindowEvent::ModifiersChanged(state),
                ..
            } => {
                modifiers = state;
            }

            Event::WindowEvent {
                event: WindowEvent::DroppedFile(path),
                ..
            } => {
                if let Err(e) = app.load_rom(&path) {
                    eprintln!("error: cannot load dropped ROM: {}", e);
                }
            }

            Event::WindowEvent {
                event: WindowEvent::KeyboardInput { event, is_synthetic, .. },
                ..
            } => {
                if is_synthetic {
                    // tao synthesizes these on focus-in for keys already held down;
                    // ignoring them preserves reacting only to real press/release.
                    return;
                }

                let is_ctrl_o = event.state == ElementState::Pressed
                    && event.physical_key == KeyCode::KeyO
                    && modifiers.control_key();
                if is_ctrl_o {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("NES ROM", &["nes"])
                        .pick_file()
                    {
                        if let Err(e) = app.load_rom(&path) {
                            eprintln!("error: cannot load ROM: {}", e);
                        }
                    }
                }

                let is_fps_toggle = event.state == ElementState::Pressed
                    && modifiers.control_key()
                    && key_map.is_fps_toggle(event.physical_key);
                if is_fps_toggle {
                    app.toggle_fps_overlay();
                }

                if let Some((port, bit)) = key_map.on_key(event.physical_key) {
                    match event.state {
                        ElementState::Pressed => button_state[port] |= bit,
                        ElementState::Released => button_state[port] &= !bit,
                        _ => {}
                    }
                    app.set_controller_buttons(port, button_state[port]);
                }
            }

            Event::MainEventsCleared => {
                if Instant::now() >= next_frame {
                    next_frame += FRAME_DURATION;
                    app.step_frame();
                    *control_flow = ControlFlow::WaitUntil(next_frame);
                }
            }

            _ => {}
        }
    });
}
