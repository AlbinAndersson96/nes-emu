# Display Backend Design

**Date:** 2026-06-20  
**Status:** Approved

## Goal

Wire the existing PPU framebuffer (`bus.ppu.frame`, `bus.ppu.frame_ready`) to a real window using `pixels` 0.13 and `winit` 0.29. The result is a running NES emulator that renders at ~60 Hz in a 512×480 window (2× NES resolution).

## Scope

- Window creation and pixel surface setup
- NES master palette (2C02 NTSC) → RGBA conversion
- Frame blitting when `frame_ready` is set
- ~60 Hz frame pacing via `ControlFlow::WaitUntil`
- Window resize handling

**Out of scope:** keyboard/controller input, audio output.

## Dependencies

```toml
[dependencies]
winit = "0.29"
pixels = "0.13"
```

## Architecture

### `src/renderer.rs` (new file)

```rust
pub struct Renderer {
    window: winit::window::Window,
    pixels: pixels::Pixels,
}
```

**`Renderer::new(event_loop: &EventLoop<()>) -> Result<Self, Box<dyn Error>>`**
- Creates a `WindowBuilder` with inner size 512×480 and title "nes-emu"
- Builds a `SurfaceTexture` from the window (256×240 logical pixel grid)
- Constructs `Pixels::new(256, 240, surface_texture)`
- Returns `Renderer { window, pixels }`

**`Renderer::present(&mut self, frame: &[u8; 256 * 240]) -> Result<(), pixels::Error>`**
- Gets `pixels.frame_mut()` — a `&mut [u8]` of 256×240×4 RGBA bytes
- For each pixel `i`: looks up `NES_PALETTE[frame[i] as usize]` → `(r, g, b)`, writes `[r, g, b, 255]`
- Calls `pixels.render()`

**`Renderer::resize(&mut self, width: u32, height: u32)`**
- Calls `pixels.resize_surface(width, height)`

**`Renderer::window_id(&self) -> winit::window::WindowId`**
- Returns `self.window.id()` for event matching

**`const NES_PALETTE: [(u8, u8, u8); 64]`**
- Standard 2C02 NTSC palette, defined as a `const` in the same file

### `src/main.rs` (modified)

```
mod renderer;
use renderer::Renderer;
use winit::{event::*, event_loop::{ControlFlow, EventLoop}};
use std::time::{Duration, Instant};
```

The existing `loop {}` is replaced by `event_loop.run(...)`:

```
const CYCLES_PER_FRAME: u64 = 29_781;
const FRAME_DURATION: Duration = Duration::from_nanos(16_666_667);

let event_loop = EventLoop::new();
let mut renderer = Renderer::new(&event_loop)?;
// ... existing bus/cpu setup ...
let mut next_frame = Instant::now();

event_loop.run(move |event, _, control_flow| {
    *control_flow = ControlFlow::WaitUntil(next_frame);

    match event {
        Event::WindowEvent { window_id, event: WindowEvent::CloseRequested, .. }
            if window_id == renderer.window_id() =>
        {
            *control_flow = ControlFlow::Exit;
        }

        Event::WindowEvent { event: WindowEvent::Resized(size), .. } => {
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
```

## Frame pacing

`next_frame` is a fixed-step deadline. On each `MainEventsCleared`:
- If wall clock has not reached `next_frame`, the event loop sleeps via `WaitUntil` — no busy-wait.
- If wall clock has passed `next_frame`, the frame runs immediately and `next_frame` advances by one `FRAME_DURATION`. If multiple frames are behind, each subsequent `MainEventsCleared` runs another frame until caught up (no accumulating lag spiral because the deadline is fixed, not re-derived from `now`).

## Data flow

```
cpu.step() → bus.tick_ppu() → ppu.frame[] + ppu.frame_ready
                                        ↓
                            renderer.present(&ppu.frame)
                                        ↓
                        NES_PALETTE lookup → RGBA buffer
                                        ↓
                              pixels.render() → window
```
