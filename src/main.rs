mod app;
mod apu;
mod bus;
mod cartridge;
mod cpu;
mod ppu;
mod renderer;
#[cfg(test)]
mod tests;

use app::App;
use std::{
    env, fs,
    path::PathBuf,
    process,
    time::{Duration, Instant},
};
use renderer::Renderer;
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
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

    if let Some(rom_path) = args.get(1) {
        if let Err(e) = app.load_rom(&PathBuf::from(rom_path)) {
            eprintln!("error: invalid ROM: {}", e);
            process::exit(1);
        }
    }

    let mut next_frame = Instant::now();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(next_frame);

        match event {
            Event::WindowEvent {
                window_id,
                event: WindowEvent::CloseRequested,
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
                event: WindowEvent::DroppedFile(path),
                ..
            } => {
                if let Err(e) = app.load_rom(&path) {
                    eprintln!("error: cannot load dropped ROM: {}", e);
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
