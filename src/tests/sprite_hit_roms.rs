use std::path::{Path, PathBuf};

use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::cpu::Cpu;
use crate::renderer::nes_to_rgba;
use crate::system::SystemClock;

// Run each ROM for 5 seconds of NES time (~300 frames at 60 fps). The ROM
// prints its result as text on screen (not a single digit like the other PPU
// suite) then loops forever while beeping; text is written before beeping
// starts, so this margin only needs to cover the actual test logic.
const FRAMES: u32 = 300;

struct RomOutput {
    frame: Box<[u8; 256 * 240]>,
    /// Text scanned off the nametable after the ROM reports its result —
    /// contains "PASSED" or "FAILED #<code>" (see decode_screen_text).
    screen_text: String,
}

const ROM_DIR: &str = "tests/roms/sprite_hit_tests_2005.10.05";

/// Nametable tiles are loaded 1:1 with ASCII for the result text (see
/// runtime/console.a: print_char writes the character byte straight to
/// $2007), while every other tile used by the tests themselves is a small
/// graphics index below $20. Filtering to the printable ASCII range pulls
/// out just the result text, wherever the runtime's console cursor left it.
fn decode_screen_text(bus: &Bus) -> String {
    let mut text = String::new();
    for row in 0..30 {
        for col in 0..32 {
            let tile = bus.ppu.nt0_tile(row, col);
            if (0x20..=0x7e).contains(&tile) {
                text.push(tile as char);
            }
        }
        text.push('\n');
    }
    text
}

fn run_sprite_hit_rom(filename: &str) -> RomOutput {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(ROM_DIR)
        .join(filename);
    let data = std::fs::read(&path).unwrap_or_else(|_| panic!("ROM not found: {}", path.display()));
    let cartridge =
        Cartridge::from_ines(&data).unwrap_or_else(|e| panic!("failed to parse {filename}: {e}"));
    let mut bus = Bus::new();
    bus.insert_cartridge(cartridge);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    // The real 6502's reset sequence takes 7 cycles before the first
    // instruction fetch; the PPU (3x the CPU clock) has already run 21 dots
    // by then. These ROMs use cycle-tuned delays for hit-timing precision
    // tests, so the CPU/PPU phase must start matched to real hardware (see
    // roms.rs's run_rom, which does the same for cpu_interrupts_v2).
    let _ = bus.tick_ppu(7);
    let _ = bus.tick_apu(8);

    // Step via the shared SystemClock: these ROMs poll $2002 in cycle-tuned
    // sync loops, and a run loop that doesn't consume the $2002-read PPU
    // pre-advance (bus.take_ppu_preadvance) drifts the PPU 3 CPU cycles per
    // read, silently breaking blargg's sync-routine phase guarantees.
    let mut clock = SystemClock::new();
    let mut frames_done = 0u32;
    loop {
        clock.step(&mut cpu, &mut bus);

        if bus.ppu.frame_ready {
            bus.ppu.frame_ready = false;
            frames_done += 1;
            if frames_done >= FRAMES {
                let mut frame = Box::new([0u8; 256 * 240]);
                frame.copy_from_slice(bus.ppu.frame.as_ref());
                return RomOutput {
                    frame,
                    screen_text: decode_screen_text(&bus),
                };
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
    let file =
        std::fs::File::open(path).unwrap_or_else(|e| panic!("cannot open {}: {e}", path.display()));
    let dec = png::Decoder::new(file);
    let mut reader = dec.read_info().unwrap();
    let mut buf = vec![0u8; reader.output_buffer_size()];
    reader.next_frame(&mut buf).unwrap();
    buf
}

/// Run `filename`, check the on-screen "PASSED"/"FAILED #<code>" text, save
/// an output screenshot, and compare against the golden image if one exists.
///
/// `meanings` maps result codes 2.. to their descriptions (index 0 = code 2,
/// index 1 = code 3, etc.) — taken directly from readme.txt.
///
/// To bless a new golden, copy:
///   tests/screenshots/ppu/output/<name>.png
///   → tests/screenshots/ppu/golden/<name>.png
fn verify(name: &str, filename: &str, meanings: &[&str]) {
    let out = run_sprite_hit_rom(filename);

    let mut rgba = vec![0u8; 256 * 240 * 4];
    nes_to_rgba(&out.frame, &mut rgba);

    let base = screenshot_base();
    let out_path = base.join("output").join(format!("{name}.png"));
    write_png(&out_path, &rgba);

    // Check the result text first so the failure message is descriptive.
    if !out.screen_text.contains("PASSED") {
        let code = out
            .screen_text
            .split("FAILED #")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|digits| digits.parse::<usize>().ok());
        let meaning = code
            .and_then(|c| meanings.get(c.saturating_sub(2)))
            .copied()
            .unwrap_or("unknown result");
        panic!(
            "{name}: ROM reported failure — {meaning}\n  screen:\n{}\n  screenshot: {}",
            out.screen_text,
            out_path.display(),
        );
    }

    // Result is a pass — compare against golden if available.
    let golden_path = base.join("golden").join(format!("{name}.png"));
    if golden_path.exists() {
        let golden = read_png(&golden_path);
        assert_eq!(
            rgba,
            golden,
            "{name}: screenshot differs from golden despite passing.\n  \
             Actual: {}\n  Golden: {}",
            out_path.display(),
            golden_path.display(),
        );
    } else {
        eprintln!(
            "[ppu/{name}] passed but no golden yet.\n  \
             Verify {out} then bless it:\n  cp {out} {gold}",
            out = out_path.display(),
            gold = golden_path.display(),
        );
    }
}

#[test]
fn sprite_hit_basics() {
    verify(
        "sprite_hit_basics",
        "01.basics.nes",
        &[
            "Sprite hit isn't working at all",
            "Should hit even when completely behind background",
            "Should miss when background rendering is off",
            "Should miss when sprite rendering is off",
            "Should miss when all rendering is off",
            "All-transparent sprite should miss",
            "Only low two palette index bits are relevant",
            "Any non-zero palette index should hit with any other",
            "Should miss when background is all transparent",
            "Should always miss other sprites",
        ],
    );
}

#[test]
fn sprite_hit_alignment() {
    verify(
        "sprite_hit_alignment",
        "02.alignment.nes",
        &[
            "Basic sprite-background alignment is way off",
            "Sprite should miss left side of bg tile",
            "Sprite should hit left side of bg tile",
            "Sprite should miss right side of bg tile",
            "Sprite should hit right side of bg tile",
            "Sprite should miss top of bg tile",
            "Sprite should hit top of bg tile",
            "Sprite should miss bottom of bg tile",
            "Sprite should hit bottom of bg tile",
        ],
    );
}

#[test]
fn sprite_hit_corners() {
    verify(
        "sprite_hit_corners",
        "03.corners.nes",
        &[
            "Lower-right pixel should hit",
            "Lower-left pixel should hit",
            "Upper-right pixel should hit",
            "Upper-left pixel should hit",
        ],
    );
}

#[test]
fn sprite_hit_flip() {
    verify(
        "sprite_hit_flip",
        "04.flip.nes",
        &[
            "Horizontal flipping doesn't work",
            "Vertical flipping doesn't work",
            "Horizontal + Vertical flipping doesn't work",
        ],
    );
}

#[test]
fn sprite_hit_left_clip() {
    verify(
        "sprite_hit_left_clip",
        "05.left_clip.nes",
        &[
            "Should miss when entirely in left-edge clipping",
            "Left-edge clipping occurs when $2001 is not $1e",
            "Left-edge clipping is off when $2001 = $1e",
            "Left-edge clipping blocks all hits only when X = 0",
            "Should miss; sprite pixel covered by left-edge clip",
            "Should hit; sprite pixel outside left-edge clip",
            "Should hit; sprite pixel outside left-edge clip",
        ],
    );
}

#[test]
fn sprite_hit_right_edge() {
    verify(
        "sprite_hit_right_edge",
        "06.right_edge.nes",
        &[
            "Should always miss when X = 255",
            "Should hit; sprite has pixels < 255",
            "Should miss; sprite pixel is at 255",
            "Should hit; sprite pixel is at 254",
            "Should also hit; sprite pixel is at 254",
        ],
    );
}

#[test]
fn sprite_hit_screen_bottom() {
    verify(
        "sprite_hit_screen_bottom",
        "07.screen_bottom.nes",
        &[
            "Should always miss when Y >= 239",
            "Can hit when Y < 239",
            "Should always miss when Y = 255",
            "Should hit; sprite pixel is at 238",
            "Should miss; sprite pixel is at 239",
            "Should hit; sprite pixel is at 238",
        ],
    );
}

#[test]
fn sprite_hit_double_height() {
    verify(
        "sprite_hit_double_height",
        "08.double_height.nes",
        &[
            "Lower sprite tile should miss bottom of bg tile",
            "Lower sprite tile should hit bottom of bg tile",
            "Lower sprite tile should miss top of bg tile",
            "Lower sprite tile should hit top of bg tile",
        ],
    );
}

#[test]
fn sprite_hit_timing_basics() {
    verify(
        "sprite_hit_timing_basics",
        "09.timing_basics.nes",
        &[
            "Upper-left corner too soon",
            "Upper-left corner too late",
            "Upper-right corner too soon",
            "Upper-right corner too late",
            "Lower-left corner too soon",
            "Lower-left corner too late",
            "Cleared at end of VBL too soon",
            "Cleared at end of VBL too late",
        ],
    );
}

#[test]
fn sprite_hit_timing_order() {
    verify(
        "sprite_hit_timing_order",
        "10.timing_order.nes",
        &[
            "Upper-left corner too soon",
            "Upper-left corner too late",
            "Upper-right corner too soon",
            "Upper-right corner too late",
            "Lower-left corner too soon",
            "Lower-left corner too late",
            "Lower-right corner too soon",
            "Lower-right corner too late",
        ],
    );
}

#[test]
fn sprite_hit_edge_timing() {
    verify(
        "sprite_hit_edge_timing",
        "11.edge_timing.nes",
        &[
            "Hit time shouldn't be based on pixels under left clip",
            "Hit time shouldn't be based on pixels at X=255",
            "Hit time shouldn't be based on pixels off right edge",
        ],
    );
}
