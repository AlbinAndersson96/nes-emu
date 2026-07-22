//! Automated headless runner for the AccuracyCoin all-in-one accuracy ROM
//! (`tests/roms/AccuracyCoin/AccuracyCoin.nes`).
//!
//! The ROM is menu-driven: with the cursor at the top of a page, pressing
//! Start runs every test in the ROM and stores each test's verdict at a fixed
//! RAM address ($0400-$0492; pass = $01, fail = (ErrorCode << 2) | $02,
//! never-ran = $00). This harness boots the ROM, injects a Start press
//! through the controller latch, waits for the run-all pass to finish
//! (`RunningAllTests` at $35 goes 0 -> 1 -> 0), and dumps every result.
//!
//! `#[ignore]`d because it takes minutes of emulated time and its verdicts
//! are tracked in `docs/accuracycoin_outcome.md` rather than asserted:
//! run with `cargo test accuracycoin -- --ignored --nocapture`.

use std::path::PathBuf;

use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::cpu::Cpu;
use crate::system::SystemClock;

/// Write directly to fd 2, bypassing Rust's test harness capture.
fn print_raw(msg: &str) {
    use std::io::Write;
    use std::os::unix::io::FromRawFd;
    // SAFETY: fd 2 (stderr) is always open. mem::forget prevents the
    // temporary File from closing it.
    let mut f = unsafe { std::fs::File::from_raw_fd(2) };
    let _ = writeln!(f, "{}", msg);
    std::mem::forget(f);
}

/// (address, name) for every `result_*` variable in AccuracyCoin.asm,
/// in address order. Extracted from the assembly source's definition block.
const RESULTS: &[(u16, &str)] = &[
    (0x0400, "Unimplemented"),
    (0x0401, "CPUInstr"),
    (0x0403, "RAMMirror"),
    (0x0404, "PPURegMirror"),
    (0x0405, "ROMnotWritable"),
    (0x0406, "DummyReads"),
    (0x0407, "DummyWrites"),
    (0x0408, "OpenBus"),
    (0x0409, "UnOp_SLO_03"),
    (0x040A, "UnOp_SLO_07"),
    (0x040B, "UnOp_SLO_0F"),
    (0x040C, "UnOp_SLO_13"),
    (0x040D, "UnOp_SLO_17"),
    (0x040E, "UnOp_SLO_1B"),
    (0x040F, "UnOp_SLO_1F"),
    (0x0410, "UnOp_ANC_0B"),
    (0x0411, "UnOp_ANC_2B"),
    (0x0412, "UnOp_ASR_4B"),
    (0x0413, "UnOp_ARR_6B"),
    (0x0414, "UnOp_ANE_8B"),
    (0x0415, "UnOp_LXA_AB"),
    (0x0416, "UnOp_AXS_CB"),
    (0x0417, "UnOp_SBC_EB"),
    (0x0419, "UnOp_RLA_23"),
    (0x041A, "UnOp_RLA_27"),
    (0x041B, "UnOp_RLA_2F"),
    (0x041C, "UnOp_RLA_33"),
    (0x041D, "UnOp_RLA_37"),
    (0x041E, "UnOp_RLA_3B"),
    (0x041F, "UnOp_RLA_3F"),
    (0x0420, "UnOp_SRE_43"),
    (0x047F, "UnOp_SRE_47"),
    (0x0422, "UnOp_SRE_4F"),
    (0x0423, "UnOp_SRE_53"),
    (0x0424, "UnOp_SRE_57"),
    (0x0425, "UnOp_SRE_5B"),
    (0x0426, "UnOp_SRE_5F"),
    (0x0427, "UnOp_RRA_63"),
    (0x0428, "UnOp_RRA_67"),
    (0x0429, "UnOp_RRA_6F"),
    (0x042A, "UnOp_RRA_73"),
    (0x042B, "UnOp_RRA_77"),
    (0x042C, "UnOp_RRA_7B"),
    (0x042D, "UnOp_RRA_7F"),
    (0x042E, "UnOp_SAX_83"),
    (0x042F, "UnOp_SAX_87"),
    (0x0430, "UnOp_SAX_8F"),
    (0x0431, "UnOp_SAX_97"),
    (0x0432, "UnOp_LAX_A3"),
    (0x0433, "UnOp_LAX_A7"),
    (0x0434, "UnOp_LAX_AF"),
    (0x0435, "UnOp_LAX_B3"),
    (0x0436, "UnOp_LAX_B7"),
    (0x0437, "UnOp_LAX_BF"),
    (0x0438, "UnOp_DCP_C3"),
    (0x0439, "UnOp_DCP_C7"),
    (0x043A, "UnOp_DCP_CF"),
    (0x043B, "UnOp_DCP_D3"),
    (0x043C, "UnOp_DCP_D7"),
    (0x043D, "UnOp_DCP_DB"),
    (0x043E, "UnOp_DCP_DF"),
    (0x043F, "UnOp_ISC_E3"),
    (0x0440, "UnOp_ISC_E7"),
    (0x0441, "UnOp_ISC_EF"),
    (0x0442, "UnOp_ISC_F3"),
    (0x0443, "UnOp_ISC_F7"),
    (0x0444, "UnOp_ISC_FB"),
    (0x0445, "UnOp_ISC_FF"),
    (0x0446, "UnOp_SHA_93"),
    (0x0447, "UnOp_SHA_9F"),
    (0x0448, "UnOp_SHS_9B"),
    (0x0449, "UnOp_SHY_9C"),
    (0x044A, "UnOp_SHX_9E"),
    (0x044B, "UnOp_LAE_BB"),
    (0x044C, "DMA_Plus_2007R"),
    (0x044D, "ProgramCounter_Wraparound"),
    (0x044E, "PPUOpenBus"),
    (0x044F, "DMA_Plus_2007W"),
    (0x0450, "VBlank_Beginning"),
    (0x0451, "VBlank_End"),
    (0x0452, "NMI_Control"),
    (0x0453, "NMI_Timing"),
    (0x0454, "NMI_Suppression"),
    (0x0455, "NMI_VBL_End"),
    (0x0456, "NMI_Disabled_VBL_Start"),
    (0x0457, "Sprite0Hit_Behavior"),
    (0x0458, "ArbitrarySpriteZero"),
    (0x0459, "SprOverflow_Behavior"),
    (0x045A, "MisalignedOAM_Behavior"),
    (0x045B, "Address2004_Behavior"),
    (0x045C, "APURegActivation"),
    (0x045D, "DMA_Plus_4015R"),
    (0x045E, "DMA_Plus_4016R"),
    (0x045F, "ControllerStrobing"),
    (0x0460, "InstructionTiming"),
    (0x0461, "IFlagLatency"),
    (0x0462, "NmiAndBrk"),
    (0x0463, "NmiAndIrq"),
    (0x0464, "RMW2007"),
    (0x0465, "APULengthCounter"),
    (0x0466, "APULengthTable"),
    (0x0467, "FrameCounterIRQ"),
    (0x0468, "FrameCounter4Step"),
    (0x0469, "FrameCounter5Step"),
    (0x046A, "DeltaModulationChannel"),
    (0x046B, "DMABusConflict"),
    (0x046C, "DMA_Plus_OpenBus"),
    (0x046D, "ImpliedDummyRead"),
    (0x046E, "AddrMode_AbsIndex"),
    (0x046F, "AddrMode_ZPgIndex"),
    (0x0470, "AddrMode_Indirect"),
    (0x0471, "AddrMode_IndIndeX"),
    (0x0472, "AddrMode_IndIndeY"),
    (0x0473, "AddrMode_Relative"),
    (0x0474, "DecimalFlag"),
    (0x0475, "BFlag"),
    (0x0476, "PPUReadBuffer"),
    (0x0477, "DMCDMAPlusOAMDMA"),
    (0x0478, "ImplicitDMAAbort"),
    (0x0479, "ExplicitDMAAbort"),
    (0x047A, "ControllerClocking"),
    (0x047B, "OAM_Corruption"),
    (0x047C, "JSREdgeCases"),
    (0x047D, "AllNOPs"),
    (0x047E, "PaletteRAMQuirks"),
    (0x0480, "INC4014"),
    (0x0481, "AttributesAsTiles"),
    (0x0482, "tRegisterQuirks"),
    (0x0483, "StaleBGShiftRegisters"),
    (0x0484, "Scanline0Sprites"),
    (0x0485, "CHRROMIsNotWritable"),
    (0x0486, "RenderingFlagBehavior"),
    (0x0487, "BGSerialIn"),
    (0x0488, "DMA_Plus_2002R"),
    (0x0489, "SuddenlyResizeSprite"),
    (0x048A, "Rendering2007Read"),
    (0x048B, "BranchDummyRead"),
    (0x048C, "2004_Stress"),
    (0x048D, "2002FlagClearTiming"),
    (0x048E, "2007_Stress"),
    (0x048F, "StaleSpriteShiftRegs"),
    (0x0490, "InternalDataBus"),
    (0x0491, "ALERead"),
    (0x0492, "HybridAddresses"),
];

const RUNNING_ALL_TESTS: u16 = 0x35; // zero-page flag set during the run-all pass
const POST_ALL_TEST_TALLY: u16 = 0x37; // count of tests run so far

/// `$2004 Stress` result address (verdict written when the test completes).
const RESULT_2004_STRESS: u16 = 0x048C;
/// Base of the ROM's per-dot `$2004 Stress` capture buffer: byte `$500 + dot`
/// holds the value `$2004` returned on that PPU dot of the target scanline
/// (17 sampled dots x 21 sub-offsets tile the 341-dot line — see
/// `TEST_2004_StressLoop` in AccuracyCoin.asm).
const STRESS_2004_BUF: u16 = 0x0500;
const STRESS_2004_LEN: usize = 341;

/// AccuracyCoin's `Test_2004_Stress_AnswerKey1` (OAM = $FF,$FE,...,$00; the
/// <8-in-range / OAMADDR-overflow case). Extracted verbatim from
/// `tests/roms/AccuracyCoin/AccuracyCoin.asm`.
#[rustfmt::skip]
const STRESS_2004_KEY1: [u8; STRESS_2004_LEN] = [
    0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFB, 0xFB, 0xF7, 0xF7, 0xF3, 0xF3, 0xEF, 0xEF, 0xEB, 0xEB, 0xE7, 0xE7, 0xE3,
    0xE3, 0xDF, 0xDF, 0xDB, 0xDB, 0xD7, 0xD7, 0xD3, 0xD3, 0xCF, 0xCF, 0xCB, 0xCB, 0xC7, 0xC7, 0xC3,
    0xC3, 0xBF, 0xBF, 0xBB, 0xBB, 0xB7, 0xB7, 0xB3, 0xB3, 0xAF, 0xAF, 0xAB, 0xAB, 0xA7, 0xA7, 0xA3,
    0xA3, 0x9F, 0x9F, 0x9B, 0x9B, 0x97, 0x97, 0x93, 0x93, 0x8F, 0x8F, 0x8B, 0x8B, 0x87, 0x87, 0x83,
    0x83, 0x7F, 0x7F, 0x7E, 0x7E, 0x61, 0x61, 0x7C, 0x7C, 0x7B, 0x7B, 0x7A, 0x7A, 0x61, 0x61, 0x78,
    0x78, 0x77, 0x77, 0x73, 0x73, 0x6F, 0x6F, 0x6B, 0x6B, 0x67, 0x67, 0x63, 0x63, 0x5F, 0x5F, 0x5B,
    0x5B, 0x57, 0x57, 0x53, 0x53, 0x4F, 0x4F, 0x4B, 0x4B, 0x47, 0x47, 0x43, 0x43, 0x3F, 0x3F, 0x3B,
    0x3B, 0x37, 0x37, 0x33, 0x33, 0x2F, 0x2F, 0x2B, 0x2B, 0x27, 0x27, 0x23, 0x23, 0x1F, 0x1F, 0x1B,
    0x1B, 0x17, 0x17, 0x13, 0x13, 0x0F, 0x0F, 0x0B, 0x0B, 0x07, 0x07, 0x03, 0x03, 0xFF, 0x03, 0xFB,
    0x03, 0xF7, 0x03, 0xF3, 0x03, 0xEF, 0x03, 0xEB, 0x03, 0xE7, 0x03, 0xE3, 0x03, 0xDF, 0x03, 0xDB,
    0x03, 0xD7, 0x03, 0xD3, 0x03, 0xCF, 0x03, 0xCB, 0x03, 0xC7, 0x03, 0xC3, 0x03, 0xBF, 0x03, 0xBB,
    0x03, 0xB7, 0x03, 0xB3, 0x03, 0xAF, 0x03, 0xAB, 0x03, 0xA7, 0x03, 0xA3, 0x03, 0x9F, 0x03, 0x9B,
    0x03, 0x7F, 0x7E, 0x61, 0x7C, 0x7C, 0x7C, 0x7C, 0x7C, 0x7B, 0x7A, 0x61, 0x78, 0x78, 0x78, 0x78,
    0x78, 0x03, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0x7F, 0x7F, 0x7F, 0x7F, 0x7F, 0x7F, 0x7F, 0x7F, 0x7F, 0x7F, 0x7F, 0x7F, 0x7F, 0x7F, 0x7F,
    0x7F, 0x7F, 0x7F, 0x7F, 0x7F,
];

/// AccuracyCoin's `Test_2004_Stress_AnswerKey2` (the interesting OAM in
/// `Test_2004_Stress_OAM`; the 8-in-range / overflow case). Extracted verbatim
/// from `tests/roms/AccuracyCoin/AccuracyCoin.asm`.
#[rustfmt::skip]
const STRESS_2004_KEY2: [u8; STRESS_2004_LEN] = [
    0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    0xFF, 0x80, 0x80, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x7F, 0x7F, 0x01, 0x01, 0x20, 0x20, 0xEE,
    0xEE, 0x7E, 0x7E, 0x02, 0x02, 0x40, 0x40, 0xDD, 0xDD, 0x7D, 0x7D, 0x03, 0x03, 0x60, 0x60, 0xCC,
    0xCC, 0x7C, 0x7C, 0x04, 0x04, 0x80, 0x80, 0xBB, 0xBB, 0x7B, 0x7B, 0x05, 0x05, 0xA0, 0xA0, 0xAA,
    0xAA, 0x7A, 0x7A, 0x06, 0x06, 0xC0, 0xC0, 0x99, 0x99, 0x79, 0x79, 0x07, 0x07, 0xE0, 0xE0, 0x88,
    0x88, 0x00, 0x80, 0x05, 0x80, 0x02, 0x80, 0x0F, 0x80, 0x10, 0x80, 0x15, 0x80, 0x02, 0x80, 0x1F,
    0x80, 0x20, 0x80, 0x25, 0x80, 0x22, 0x80, 0x2F, 0x80, 0x30, 0x80, 0x35, 0x80, 0x22, 0x80, 0x3F,
    0x80, 0x40, 0x80, 0x45, 0x80, 0x42, 0x80, 0x4F, 0x80, 0x50, 0x80, 0x55, 0x80, 0x42, 0x80, 0x5F,
    0x80, 0x60, 0x80, 0x65, 0x80, 0x62, 0x80, 0x6F, 0x80, 0x70, 0x80, 0x75, 0x80, 0x62, 0x80, 0x7F,
    0x80, 0x80, 0x80, 0x81, 0x80, 0x82, 0x80, 0x80, 0x80, 0x84, 0x80, 0x88, 0x80, 0x8C, 0x80, 0x90,
    0x80, 0x94, 0x80, 0x98, 0x80, 0x9C, 0x80, 0xA0, 0x80, 0xA4, 0x80, 0xA8, 0x80, 0xAC, 0x80, 0xB0,
    0x80, 0xB4, 0x80, 0xB8, 0x80, 0xBC, 0x80, 0xC0, 0x80, 0xC4, 0x80, 0xC8, 0x80, 0xCC, 0x80, 0xD0,
    0x80, 0xD4, 0x80, 0xD8, 0x80, 0xDC, 0x80, 0x80, 0x80, 0x7F, 0x80, 0x7E, 0x80, 0x7D, 0x80, 0x7C,
    0x80, 0x80, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F, 0x01, 0x20, 0xEE, 0xEE, 0xEE, 0xEE,
    0xEE, 0x7E, 0x02, 0x40, 0xDD, 0xDD, 0xDD, 0xDD, 0xDD, 0x7D, 0x03, 0x60, 0xCC, 0xCC, 0xCC, 0xCC,
    0xCC, 0x7C, 0x04, 0x80, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0x7B, 0x05, 0xA0, 0xAA, 0xAA, 0xAA, 0xAA,
    0xAA, 0x7A, 0x06, 0xC0, 0x99, 0x99, 0x99, 0x99, 0x99, 0x79, 0x07, 0xE0, 0x88, 0x88, 0x88, 0x88,
    0x88, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80,
    0x80, 0x80, 0x80, 0x80, 0x80,
];

const CYCLES_PER_FRAME: u64 = 29781;
const BOOT_FRAMES: u64 = 120;
const START_HOLD_FRAMES: u64 = 4;
// Generous overall cap: ~10 minutes of NES time.
const MAX_CYCLES: u64 = 1_073_000_000;

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

/// Count how many positions differ when the captured buffer is treated as
/// `shift` dots late relative to `key`: `key[i]` vs `buf[i + shift]` over the
/// overlap `[0, LEN - shift)`. Plain equality (the ROM's own bit-flip
/// tolerance is only for the shifted, analog-corrupt alignment — not needed to
/// find the best shift). Returns (mismatch_count, overlap_len).
fn stress_2004_mismatches(buf: &[u8], key: &[u8; STRESS_2004_LEN], shift: usize) -> (usize, usize) {
    let overlap = STRESS_2004_LEN - shift;
    let mismatches = (0..overlap).filter(|&i| key[i] != buf[i + shift]).count();
    (mismatches, overlap)
}

/// Diagnostic dump of the `$2004 Stress` per-dot capture buffer against both
/// answer keys. Gated behind `TRACE_2004_STRESS`; called once when the test's
/// verdict is first written (buffer still intact, before the next test reuses
/// `$500`). This is the Phase-0 oracle: it reveals which sub-test was reached,
/// the clock-alignment shift, and the exact mismatching dots.
fn dump_2004_stress(buf: &[u8]) -> String {
    let mut out = String::new();
    out.push_str("\n===== $2004 Stress capture ($500..$654, byte = value read on that dot) =====\n");

    // Raw buffer, 16 bytes/row labelled by starting dot.
    for row in 0..((STRESS_2004_LEN + 15) / 16) {
        let base = row * 16;
        let mut line = format!("dot {base:3}: ");
        for i in base..(base + 16).min(STRESS_2004_LEN) {
            line.push_str(&format!("{:02X} ", buf[i]));
        }
        out.push_str(&line);
        out.push('\n');
    }

    // Per-key, per-shift mismatch counts (shift = buffer dots late).
    out.push_str("\nMismatch counts by shift (buffer treated as N dots late):\n");
    let mut best: Option<(&str, &[u8; STRESS_2004_LEN], usize, usize)> = None;
    for (name, key) in [("KEY1", &STRESS_2004_KEY1), ("KEY2", &STRESS_2004_KEY2)] {
        let mut line = format!("  {name}: ");
        for shift in 0..=4 {
            let (m, n) = stress_2004_mismatches(buf, key, shift);
            line.push_str(&format!("+{shift}={m}/{n}  "));
            if best.map_or(true, |(_, _, _, bm)| m < bm) {
                best = Some((name, key, shift, m));
            }
        }
        out.push_str(&line);
        out.push('\n');
    }

    // For the best fit, list the mismatching dots (capped).
    if let Some((name, key, shift, m)) = best {
        out.push_str(&format!(
            "\nBest fit: {name} at shift +{shift} ({m} mismatches). First mismatches (dot: got != want):\n"
        ));
        let overlap = STRESS_2004_LEN - shift;
        let mut shown = 0;
        for i in 0..overlap {
            if key[i] != buf[i + shift] {
                out.push_str(&format!(
                    "  dot {:3}: {:02X} != {:02X}\n",
                    i + shift,
                    buf[i + shift],
                    key[i]
                ));
                shown += 1;
                if shown >= 48 {
                    out.push_str("  ... (truncated)\n");
                    break;
                }
            }
        }
    }
    out.push_str("===== end $2004 Stress capture =====\n");
    out
}

fn dump_results(bus: &Bus) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "0012 DMASync_PreTest              raw ${:02X} (1 = open-bus sync, 2 = fallback sync)\n",
        bus.peek(0x12)
    ));
    for &(addr, name) in RESULTS {
        let v = bus.peek(addr);
        let verdict = match v {
            0 => "NOT-RUN".to_string(),
            1 => "pass".to_string(),
            // Success with multiple acceptable behaviors: 1 | (variant << 2)
            // (e.g. DMA + $4016 Read returns $05 for NES double-read
            // behavior, $09 for Famicom bus-conflict behavior).
            v if v & 0x03 == 0x01 => format!("pass (variant {})", v >> 2),
            v if v & 0x02 != 0 => format!("FAIL code {}", v >> 2),
            v => format!("UNEXPECTED ${v:02X}"),
        };
        out.push_str(&format!("{addr:04X} {name:28} {verdict}\n"));
    }
    out
}

#[test]
#[ignore]
fn accuracycoin_run_all() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/roms/AccuracyCoin/AccuracyCoin.nes");
    let data = std::fs::read(&path).unwrap_or_else(|_| panic!("ROM not found: {}", path.display()));
    let cartridge = Cartridge::from_ines(&data).expect("failed to parse AccuracyCoin.nes");
    let mut bus = Bus::new();
    bus.insert_cartridge(cartridge);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    let _ = bus.tick_ppu(7);
    let _ = bus.tick_apu(8);

    let mut clock = SystemClock::new();
    let mut started = false;
    let mut last_tally = 0xFFu8;
    let mut apureg_seen = false;
    // Phase-0 oracle for the `$2004 Stress` work: when TRACE_2004_STRESS is set,
    // snapshot the ROM's per-dot capture buffer ($500..$654) the instant the
    // test's verdict is written (buffer still intact) and diff against both keys.
    let trace_2004_stress = std::env::var_os("TRACE_2004_STRESS").is_some();
    let mut stress_2004_dumped = false;
    // `RunningAllTests` ($35) is a zero-page byte the ROM sets to 1 at run-all
    // start and clears only at true completion. But the `Implied Dummy Reads`
    // test legitimately zeroes ALL of zero page ($00-$FF via `STA $00,X`) as
    // scratch, transiently clearing $35 mid-run before restoring ZP from
    // $0700-$07FF. Breaking on the first $35==0 would end the run early inside
    // that test; require $35 to stay 0 for a sustained window to tell true
    // completion from the transient clobber.
    let mut zero_since: Option<u64> = None;
    const RUNNING_ZERO_STABLE_CYCLES: u64 = 4_000_000;

    loop {
        clock.step(&mut cpu, &mut bus);
        let cycles = cpu.cycles;

        // Controller script: after boot, hold Start for a few frames, then
        // release. Start = bit 3 in the latch's NES serial order (A, B,
        // Select, Start, ...).
        let frame = cycles / CYCLES_PER_FRAME;
        let buttons = if (BOOT_FRAMES..BOOT_FRAMES + START_HOLD_FRAMES).contains(&frame) {
            0x08
        } else {
            0x00
        };
        bus.set_controller_state(0, buttons);

        if trace_2004_stress && !stress_2004_dumped && bus.peek(RESULT_2004_STRESS) != 0 {
            stress_2004_dumped = true;
            let buf: Vec<u8> = (0..STRESS_2004_LEN)
                .map(|i| bus.peek(STRESS_2004_BUF + i as u16))
                .collect();
            print_raw(&format!(
                "[accuracycoin] $2004 Stress verdict ${:02X} at cycle {cycles}",
                bus.peek(RESULT_2004_STRESS)
            ));
            print_raw(&dump_2004_stress(&buf));
        }

        let running = bus.peek(RUNNING_ALL_TESTS);
        if !started && running != 0 {
            started = true;
            print_raw(&format!("[accuracycoin] run-all started at cycle {cycles}"));
        }
        if started && bus.peek(0x045C) != 0 && !apureg_seen {
            apureg_seen = true;
            print_raw(&format!(
                "[accuracycoin] APURegActivation result ${:02X}; pre-check counter $50={} (cycle {cycles})",
                bus.peek(0x045C),
                bus.peek(0x50)
            ));
        }
        if started {
            let tally = bus.peek(POST_ALL_TEST_TALLY);
            if tally != last_tally {
                last_tally = tally;
                print_raw(&format!(
                    "[accuracycoin] running test #{tally} (cycle {cycles})"
                ));
            }
            if running == 0 {
                if zero_since.is_none() {
                    zero_since = Some(cycles);
                }
                if cycles - zero_since.unwrap() >= RUNNING_ZERO_STABLE_CYCLES {
                    print_raw(&format!(
                        "[accuracycoin] run-all finished at cycle {cycles}"
                    ));
                    break;
                }
            } else {
                zero_since = None;
            }
        }

        if cycles > MAX_CYCLES {
            print_raw(&format!(
                "[accuracycoin] TIMED OUT at cycle {cycles} (started={started}, tally={last_tally}); screen:\n{}",
                decode_screen_text(&bus)
            ));
            break;
        }
    }

    print_raw(&dump_results(&bus));
}
