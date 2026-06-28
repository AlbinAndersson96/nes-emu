use std::path::{Path, PathBuf};

use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::cpu::Cpu;
use crate::renderer::nes_to_rgba;

// Run each ROM for 5 seconds of NES time (~300 frames at 60 fps).
const FRAMES: u32 = 300;

struct RomOutput {
    frame: Box<[u8; 256 * 240]>,
    /// Blargg result code read from the nametable ($01 = pass).
    result_code: u8,
}

fn run_ppu_rom(filename: &str) -> RomOutput {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/roms")
        .join(filename);
    let data = std::fs::read(&path)
        .unwrap_or_else(|_| panic!("ROM not found: {}", path.display()));
    let cartridge = Cartridge::from_ines(&data)
        .unwrap_or_else(|e| panic!("failed to parse {filename}: {e}"));
    let mut bus = Bus::new();
    bus.insert_cartridge(cartridge);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);

    let mut frames_done = 0u32;
    loop {
        let cycles_before = cpu.cycles;
        if bus.dma_active() {
            bus.tick_dma();
            if bus.tick_ppu(1) { cpu.nmi(); }
            if bus.tick_apu(1) { cpu.irq(); }
        } else {
            cpu.tick(&mut bus);
            let delta = cpu.cycles - cycles_before;
            if bus.tick_ppu(delta) { cpu.nmi(); }
            if bus.tick_apu(delta) { cpu.irq(); }
        }

        if bus.ppu.frame_ready {
            bus.ppu.frame_ready = false;
            frames_done += 1;
            if frames_done >= FRAMES {
                // The ROMs display "$XX" at nametable-0 tile (row=5, col=2..4).
                // Tiles are ASCII-mapped; subtract 0x30 to get the decimal digit.
                let digit_tile = bus.ppu.nt0_tile(5, 4);
                let result_code = digit_tile.wrapping_sub(0x30);

                let mut frame = Box::new([0u8; 256 * 240]);
                frame.copy_from_slice(bus.ppu.frame.as_ref());
                return RomOutput { frame, result_code };
            }
        }
    }
}

fn screenshot_base() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/screenshots/ppu")
}

fn write_png(path: &Path, rgba: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let file = std::fs::File::create(path)
        .unwrap_or_else(|e| panic!("cannot create {}: {e}", path.display()));
    let mut enc = png::Encoder::new(file, 256, 240);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(rgba).unwrap();
}

fn read_png(path: &Path) -> Vec<u8> {
    let file = std::fs::File::open(path)
        .unwrap_or_else(|e| panic!("cannot open {}: {e}", path.display()));
    let dec = png::Decoder::new(file);
    let mut reader = dec.read_info().unwrap();
    let mut buf = vec![0u8; reader.output_buffer_size()];
    reader.next_frame(&mut buf).unwrap();
    buf
}

/// Run `filename`, check the on-screen result code, save an output screenshot,
/// and compare against the golden image if one exists.
///
/// `meanings` maps result codes 2.. to their descriptions (index 0 = code 2,
/// index 1 = code 3, etc.) — taken directly from the README.
///
/// To bless a new golden, copy:
///   tests/screenshots/ppu/output/<name>.png
///   → tests/screenshots/ppu/golden/<name>.png
fn verify(name: &str, filename: &str, meanings: &[&str]) {
    let out = run_ppu_rom(filename);

    let mut rgba = vec![0u8; 256 * 240 * 4];
    nes_to_rgba(&out.frame, &mut rgba);

    let base = screenshot_base();
    let out_path = base.join("output").join(format!("{name}.png"));
    write_png(&out_path, &rgba);

    // Check result code first so the failure message is descriptive.
    if out.result_code != 1 {
        let meaning = meanings
            .get(out.result_code.saturating_sub(2) as usize)
            .copied()
            .unwrap_or("unknown result code");
        panic!(
            "{name}: ROM reports result code {} — {meaning}\n  screenshot: {}",
            out.result_code,
            out_path.display(),
        );
    }

    // Result is 1 (pass) — compare against golden if available.
    let golden_path = base.join("golden").join(format!("{name}.png"));
    if golden_path.exists() {
        let golden = read_png(&golden_path);
        assert_eq!(
            rgba, golden,
            "{name}: screenshot differs from golden despite result code 1.\n  \
             Actual: {}\n  Golden: {}",
            out_path.display(),
            golden_path.display(),
        );
    } else {
        eprintln!(
            "[ppu/{name}] result code 1 (pass) but no golden yet.\n  \
             Verify {out} then bless it:\n  cp {out} {gold}",
            out = out_path.display(),
            gold = golden_path.display(),
        );
    }
}

#[test]
fn palette_ram() {
    verify("palette_ram", "ppu/palette_ram.nes", &[
        "Palette read shouldn't be buffered like other VRAM",
        "Palette write/read doesn't work",
        "Palette should be mirrored within $3f00-$3fff",
        "Write to $10 should be mirrored at $00",
        "Write to $00 should be mirrored at $10",
    ]);
}

#[test]
fn power_up_palette() {
    verify("power_up_palette", "ppu/power_up_palette.nes", &[
        "Palette differs from table",
    ]);
}

#[test]
fn sprite_ram() {
    verify("sprite_ram", "ppu/sprite_ram.nes", &[
        "Basic read/write doesn't work",
        "Address should increment on $2004 write",
        "Address should not increment on $2004 read",
        "Third sprite bytes should be masked with $e3 on read",
        "$4014 DMA copy doesn't work at all",
        "$4014 DMA copy should start at value in $2003 and wrap",
        "$4014 DMA copy should leave value in $2003 intact",
    ]);
}

#[test]
fn vbl_clear_time() {
    verify("vbl_clear_time", "ppu/vbl_clear_time.nes", &[
        "VBL flag cleared too soon",
        "VBL flag cleared too late",
    ]);
}

#[test]
fn vram_access() {
    verify("vram_access", "ppu/vram_access.nes", &[
        "VRAM reads should be delayed in a buffer",
        "Basic write/read doesn't work",
        "Read buffer shouldn't be affected by VRAM write",
        "Read buffer shouldn't be affected by palette write",
        "Palette read should also read VRAM into read buffer",
        "\"Shadow\" VRAM read unaffected by palette transparent color mirroring",
    ]);
}
