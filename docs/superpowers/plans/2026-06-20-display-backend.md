# Display Backend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a windowed display to the NES emulator using `pixels` 0.13 + `winit` 0.29 so the PPU framebuffer is blitted to screen at ~60 Hz.

**Architecture:** A new `src/renderer.rs` module owns the `Window` and `Pixels` surface. `main.rs`'s bare `loop {}` is replaced with a `winit` event loop that runs ~29,781 CPU cycles per frame and calls `renderer.present()` when `bus.ppu.frame_ready` is set. Frame pacing uses `ControlFlow::WaitUntil` with a fixed 16.67 ms deadline.

**Tech Stack:** Rust 2024 edition, `winit 0.29`, `pixels 0.13`

## Global Constraints

- `winit = "0.29"` — do not upgrade to 0.30 (incompatible API)
- `pixels = "0.13"` — paired with winit 0.29
- Window logical size: 512×480 (2× NES resolution 256×240)
- NES texture size passed to `Pixels::new`: 256×240
- Frame cycle budget: 29,781 CPU cycles per frame
- Frame duration: 16,666,667 ns (~60.0 Hz)
- Pixel format written to `pixels.frame_mut()`: RGBA, alpha always 255

---

### Task 1: Add dependencies

**Files:**
- Modify: `Cargo.toml`

**Interfaces:**
- Produces: `winit` and `pixels` crates available to all source files

- [ ] **Step 1: Add crates to Cargo.toml**

Replace the `[dependencies]` section:

```toml
[dependencies]
winit = "0.29"
pixels = "0.13"
```

- [ ] **Step 2: Verify resolution**

```bash
cargo build
```

Expected: downloads and compiles `winit 0.29.x` and `pixels 0.13.x`, no errors.

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "deps: add winit 0.29 and pixels 0.13"
```

---

### Task 2: `src/renderer.rs` — palette, conversion helper, Renderer struct

**Files:**
- Create: `src/renderer.rs`

**Interfaces:**
- Consumes: nothing (standalone module)
- Produces:
  - `pub const NES_PALETTE: [(u8, u8, u8); 64]`
  - `pub(crate) fn nes_to_rgba(frame: &[u8; 256 * 240], buf: &mut [u8])`
  - `pub struct Renderer`
  - `impl Renderer { pub fn new(event_loop: &EventLoop<()>) -> Result<Self, Box<dyn std::error::Error>> }`
  - `impl Renderer { pub fn present(&mut self, frame: &[u8; 256 * 240]) -> Result<(), pixels::Error> }`
  - `impl Renderer { pub fn resize(&mut self, width: u32, height: u32) }`
  - `impl Renderer { pub fn window_id(&self) -> winit::window::WindowId }`

- [ ] **Step 1: Write failing tests in a skeleton file**

Create `src/renderer.rs` with a stub and tests only:

```rust
pub(crate) fn nes_to_rgba(_frame: &[u8; 256 * 240], _buf: &mut [u8]) {
    unimplemented!()
}

pub const NES_PALETTE: [(u8, u8, u8); 64] = [
    (0x54, 0x54, 0x54), (0x00, 0x1E, 0x74), (0x08, 0x10, 0x90), (0x30, 0x00, 0x88),
    (0x44, 0x00, 0x64), (0x5C, 0x00, 0x30), (0x54, 0x04, 0x00), (0x3C, 0x18, 0x00),
    (0x20, 0x2A, 0x00), (0x08, 0x3A, 0x00), (0x00, 0x40, 0x00), (0x00, 0x3C, 0x00),
    (0x00, 0x32, 0x3C), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00),
    (0x98, 0x96, 0x98), (0x08, 0x4C, 0xC4), (0x30, 0x32, 0xEC), (0x5C, 0x1E, 0xE4),
    (0x88, 0x14, 0xB0), (0xA0, 0x14, 0x64), (0x98, 0x22, 0x20), (0x78, 0x3C, 0x00),
    (0x54, 0x5A, 0x00), (0x28, 0x72, 0x00), (0x08, 0x7C, 0x00), (0x00, 0x76, 0x28),
    (0x00, 0x66, 0x78), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00),
    (0xEC, 0xEE, 0xEC), (0x4C, 0x9A, 0xEC), (0x78, 0x7C, 0xEC), (0xB0, 0x62, 0xEC),
    (0xE4, 0x54, 0xEC), (0xEC, 0x58, 0xA4), (0xEC, 0x6A, 0x64), (0xD4, 0x88, 0x20),
    (0xA0, 0xAA, 0x00), (0x74, 0xC4, 0x00), (0x4C, 0xD0, 0x20), (0x38, 0xCC, 0x6C),
    (0x38, 0xB4, 0xCC), (0x3C, 0x3C, 0x3C), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00),
    (0xEC, 0xEE, 0xEC), (0xA8, 0xCC, 0xEC), (0xBC, 0xBC, 0xEC), (0xD4, 0xB2, 0xEC),
    (0xEC, 0xAE, 0xEC), (0xEC, 0xAE, 0xD4), (0xEC, 0xB4, 0xB0), (0xE4, 0xC4, 0x90),
    (0xCC, 0xD2, 0x78), (0xB4, 0xDE, 0x78), (0xA8, 0xE2, 0x90), (0x98, 0xE2, 0xB4),
    (0xA0, 0xD6, 0xE4), (0xA0, 0xA2, 0xA0), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_index_0_maps_correctly() {
        let frame = [0u8; 256 * 240];
        let mut buf = [0u8; 256 * 240 * 4];
        nes_to_rgba(&frame, &mut buf);
        let (r, g, b) = NES_PALETTE[0];
        assert_eq!(&buf[0..4], &[r, g, b, 255]);
    }

    #[test]
    fn palette_index_masked_to_6_bits() {
        let mut frame = [0u8; 256 * 240];
        frame[0] = 0x41; // 0x41 & 0x3F == 0x01
        let mut buf = [0u8; 256 * 240 * 4];
        nes_to_rgba(&frame, &mut buf);
        let (r, g, b) = NES_PALETTE[0x01];
        assert_eq!(&buf[0..4], &[r, g, b, 255]);
    }

    #[test]
    fn alpha_channel_always_255() {
        let frame = [0x3Fu8; 256 * 240];
        let mut buf = [0u8; 256 * 240 * 4];
        nes_to_rgba(&frame, &mut buf);
        for chunk in buf.chunks(4) {
            assert_eq!(chunk[3], 255, "alpha must be 255 for all pixels");
        }
    }

    #[test]
    fn last_pixel_is_written() {
        let mut frame = [0u8; 256 * 240];
        let last = 256 * 240 - 1;
        frame[last] = 0x30;
        let mut buf = [0u8; 256 * 240 * 4];
        nes_to_rgba(&frame, &mut buf);
        let (r, g, b) = NES_PALETTE[0x30];
        assert_eq!(&buf[last * 4..last * 4 + 4], &[r, g, b, 255]);
    }
}
```

Also add `mod renderer;` to `src/main.rs` (the module declaration must exist for `cargo test` to compile it):

In `src/main.rs`, add this line after `mod ppu;`:
```rust
mod renderer;
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test renderer::tests
```

Expected: panics with `not yet implemented` from the `unimplemented!()` stub.

- [ ] **Step 3: Implement `nes_to_rgba` and the full `Renderer` struct**

Replace the entire contents of `src/renderer.rs` with:

```rust
use pixels::{Pixels, SurfaceTexture};
use winit::{
    dpi::LogicalSize,
    event_loop::EventLoop,
    window::{Window, WindowBuilder, WindowId},
};

pub const NES_PALETTE: [(u8, u8, u8); 64] = [
    (0x54, 0x54, 0x54), (0x00, 0x1E, 0x74), (0x08, 0x10, 0x90), (0x30, 0x00, 0x88),
    (0x44, 0x00, 0x64), (0x5C, 0x00, 0x30), (0x54, 0x04, 0x00), (0x3C, 0x18, 0x00),
    (0x20, 0x2A, 0x00), (0x08, 0x3A, 0x00), (0x00, 0x40, 0x00), (0x00, 0x3C, 0x00),
    (0x00, 0x32, 0x3C), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00),
    (0x98, 0x96, 0x98), (0x08, 0x4C, 0xC4), (0x30, 0x32, 0xEC), (0x5C, 0x1E, 0xE4),
    (0x88, 0x14, 0xB0), (0xA0, 0x14, 0x64), (0x98, 0x22, 0x20), (0x78, 0x3C, 0x00),
    (0x54, 0x5A, 0x00), (0x28, 0x72, 0x00), (0x08, 0x7C, 0x00), (0x00, 0x76, 0x28),
    (0x00, 0x66, 0x78), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00),
    (0xEC, 0xEE, 0xEC), (0x4C, 0x9A, 0xEC), (0x78, 0x7C, 0xEC), (0xB0, 0x62, 0xEC),
    (0xE4, 0x54, 0xEC), (0xEC, 0x58, 0xA4), (0xEC, 0x6A, 0x64), (0xD4, 0x88, 0x20),
    (0xA0, 0xAA, 0x00), (0x74, 0xC4, 0x00), (0x4C, 0xD0, 0x20), (0x38, 0xCC, 0x6C),
    (0x38, 0xB4, 0xCC), (0x3C, 0x3C, 0x3C), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00),
    (0xEC, 0xEE, 0xEC), (0xA8, 0xCC, 0xEC), (0xBC, 0xBC, 0xEC), (0xD4, 0xB2, 0xEC),
    (0xEC, 0xAE, 0xEC), (0xEC, 0xAE, 0xD4), (0xEC, 0xB4, 0xB0), (0xE4, 0xC4, 0x90),
    (0xCC, 0xD2, 0x78), (0xB4, 0xDE, 0x78), (0xA8, 0xE2, 0x90), (0x98, 0xE2, 0xB4),
    (0xA0, 0xD6, 0xE4), (0xA0, 0xA2, 0xA0), (0x00, 0x00, 0x00), (0x00, 0x00, 0x00),
];

pub(crate) fn nes_to_rgba(frame: &[u8; 256 * 240], buf: &mut [u8]) {
    for (i, &idx) in frame.iter().enumerate() {
        let (r, g, b) = NES_PALETTE[(idx & 0x3F) as usize];
        buf[i * 4]     = r;
        buf[i * 4 + 1] = g;
        buf[i * 4 + 2] = b;
        buf[i * 4 + 3] = 255;
    }
}

pub struct Renderer {
    window: Window,
    pixels: Pixels,
}

impl Renderer {
    pub fn new(event_loop: &EventLoop<()>) -> Result<Self, Box<dyn std::error::Error>> {
        let window = WindowBuilder::new()
            .with_title("nes-emu")
            .with_inner_size(LogicalSize::new(512u32, 480u32))
            .build(event_loop)?;
        let size = window.inner_size();
        let surface_texture = SurfaceTexture::new(size.width, size.height, &window);
        let pixels = Pixels::new(256, 240, surface_texture)?;
        Ok(Self { window, pixels })
    }

    pub fn present(&mut self, frame: &[u8; 256 * 240]) -> Result<(), pixels::Error> {
        nes_to_rgba(frame, self.pixels.frame_mut());
        self.pixels.render()
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let _ = self.pixels.resize_surface(width, height);
    }

    pub fn window_id(&self) -> WindowId {
        self.window.id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palette_index_0_maps_correctly() {
        let frame = [0u8; 256 * 240];
        let mut buf = [0u8; 256 * 240 * 4];
        nes_to_rgba(&frame, &mut buf);
        let (r, g, b) = NES_PALETTE[0];
        assert_eq!(&buf[0..4], &[r, g, b, 255]);
    }

    #[test]
    fn palette_index_masked_to_6_bits() {
        let mut frame = [0u8; 256 * 240];
        frame[0] = 0x41; // 0x41 & 0x3F == 0x01
        let mut buf = [0u8; 256 * 240 * 4];
        nes_to_rgba(&frame, &mut buf);
        let (r, g, b) = NES_PALETTE[0x01];
        assert_eq!(&buf[0..4], &[r, g, b, 255]);
    }

    #[test]
    fn alpha_channel_always_255() {
        let frame = [0x3Fu8; 256 * 240];
        let mut buf = [0u8; 256 * 240 * 4];
        nes_to_rgba(&frame, &mut buf);
        for chunk in buf.chunks(4) {
            assert_eq!(chunk[3], 255, "alpha must be 255 for all pixels");
        }
    }

    #[test]
    fn last_pixel_is_written() {
        let mut frame = [0u8; 256 * 240];
        let last = 256 * 240 - 1;
        frame[last] = 0x30;
        let mut buf = [0u8; 256 * 240 * 4];
        nes_to_rgba(&frame, &mut buf);
        let (r, g, b) = NES_PALETTE[0x30];
        assert_eq!(&buf[last * 4..last * 4 + 4], &[r, g, b, 255]);
    }
}
```

- [ ] **Step 4: Run tests**

```bash
cargo test renderer::tests
```

Expected:
```
test renderer::tests::alpha_channel_always_255 ... ok
test renderer::tests::last_pixel_is_written ... ok
test renderer::tests::palette_index_0_maps_correctly ... ok
test renderer::tests::palette_index_masked_to_6_bits ... ok
```

- [ ] **Step 5: Commit**

```bash
git add src/renderer.rs
git commit -m "feat: add Renderer with NES palette and RGBA conversion"
```

---

### Task 3: Wire `main.rs` to the winit event loop

**Files:**
- Modify: `src/main.rs`

**Interfaces:**
- Consumes:
  - `renderer::Renderer::new(event_loop: &EventLoop<()>) -> Result<Self, Box<dyn std::error::Error>>`
  - `renderer::Renderer::present(&mut self, frame: &[u8; 256 * 240]) -> Result<(), pixels::Error>`
  - `renderer::Renderer::resize(&mut self, width: u32, height: u32)`
  - `renderer::Renderer::window_id(&self) -> winit::window::WindowId`
  - `bus::Bus::ppu.frame: Box<[u8; 256 * 240]>` — already `pub`
  - `bus::Bus::ppu.frame_ready: bool` — already `pub`
  - `cpu::Cpu::cycles: u64` — already `pub`
- Produces: a running windowed NES emulator at ~60 Hz

- [ ] **Step 1: Replace `src/main.rs`**

```rust
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

fn main() {
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
```

- [ ] **Step 2: Verify all existing tests still pass**

```bash
cargo test
```

Expected: all previously passing tests continue to pass. The event loop change does not affect test-only code paths.

- [ ] **Step 3: Build and run with a ROM**

```bash
cargo build --release
./target/release/nes-emu path/to/game.nes
```

Expected: a 512×480 window opens showing the game's output at ~60 Hz. The window closes cleanly when the X button is clicked. Resizing the window does not crash.

- [ ] **Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat: wire winit event loop and pixels display backend"
```
