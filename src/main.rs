mod apu;
mod bus;
mod cartridge;
mod cpu;
mod ppu;
mod renderer;
#[cfg(test)]
mod tests;

use std::{
    env, fs, process,
    time::{Duration, Instant},
};
use renderer::Renderer;
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop},
};

const CYCLES_PER_FRAME: u64 = 29_781;
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
    if args.len() != 2 {
        eprintln!("Usage: {} <rom.nes>", args[0]);
        process::exit(1);
    }

    let data = match fs::read(&args[1]) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: cannot read '{}': {}", args[1], e);
            process::exit(1);
        }
    };

    let cartridge = match cartridge::Cartridge::from_ines(&data) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: invalid ROM: {}", e);
            process::exit(1);
        }
    };

    let mut bus = bus::Bus::new();
    bus.insert_cartridge(cartridge);

    let mut cpu = cpu::Cpu::new();
    cpu.reset(&mut bus);

    let event_loop = EventLoop::new();
    let mut renderer = match Renderer::new(&event_loop) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: could not create window: {}", e);
            process::exit(1);
        }
    };

    let mut next_frame = Instant::now();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::WaitUntil(next_frame);

        match event {
            Event::WindowEvent {
                window_id,
                event: WindowEvent::CloseRequested,
            } if window_id == renderer.window_id() => {
                *control_flow = ControlFlow::Exit;
            }

            Event::WindowEvent {
                event: WindowEvent::Resized(size),
                ..
            } => {
                renderer.resize(size.width, size.height);
            }

            Event::MainEventsCleared => {
                if Instant::now() >= next_frame {
                    next_frame += FRAME_DURATION;

                    let mut elapsed = 0u64;
                    while elapsed < CYCLES_PER_FRAME {
                        let c = cpu.step(&mut bus) as u64;
                        let dma = bus.take_oam_dma_stall() as u64;
                        let extra = if dma > 0 { dma + (cpu.cycles & 1) } else { 0 };
                        let total = c + extra;
                        elapsed += total;
                        if bus.tick_ppu(total) { cpu.nmi(); }
                        if bus.tick_apu(total) { cpu.irq(); }
                    }

                    if bus.ppu.frame_ready {
                        bus.ppu.frame_ready = false;
                        renderer.present(&bus.ppu.frame).unwrap();
                    }

                    *control_flow = ControlFlow::WaitUntil(next_frame);
                }
            }

            _ => {}
        }
    });
}
