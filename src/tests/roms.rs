use std::path::Path;

use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::cpu::Bus as CpuBus;
use crate::cpu::Cpu;
use crate::system::SystemClock;

// Signature written by the test ROM to $6001-$6003 once output is valid.
const SIG: [u8; 3] = [0xDE, 0xB0, 0x61];

// Generous upper bound: ~56 seconds of NES time at 1.789 MHz.
const MAX_CYCLES: u64 = 100_000_000;

/// Write directly to fd 2, bypassing Rust's test harness capture.
/// Both io::stdout() and io::stderr() are intercepted by the harness;
/// a raw file handle to fd 2 is not.
fn print_raw(msg: &str) {
    use std::io::Write;
    use std::os::unix::io::FromRawFd;
    // SAFETY: fd 2 (stderr) is always open. mem::forget prevents the
    // temporary File from closing it.
    let mut f = unsafe { std::fs::File::from_raw_fd(2) };
    let _ = writeln!(f, "{}", msg);
    std::mem::forget(f);
}

fn load_rom(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/roms")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|_| {
        panic!(
            "ROM not found: {} — place it in tests/roms/<suite-name>/",
            path.display()
        )
    })
}

fn read_output(bus: &mut Bus) -> String {
    let mut out = String::new();
    let mut addr = 0x6004u16;
    loop {
        let b = bus.read(addr);
        if b == 0 {
            break;
        }
        out.push(b as char);
        addr = addr.wrapping_add(1);
    }
    out
}

fn run_until_complete_trace(filename: &str, bus: &mut Bus, cpu: &mut Cpu, trace_nmi: bool) {
    let mut total_cycles: u64 = 0;
    let mut nmi_count: u32 = 0;
    let mut last_nmi_cycle: u64 = 0;
    // All interrupt-delivery rules (deferred NMI edges, per-cycle APU ticking
    // with last-cycle IRQ flagging, DMA interrupt deferral) live in the shared
    // SystemClock — the same stepping code main.rs runs. See
    // docs/cpu_interrupts.md for how each rule was derived and verified.
    let mut clock = SystemClock::new();

    // Status $81 means "needs the reset button pressed, but delayed by at
    // least 100 msec from now" (see tests/roms/cpu_reset/readme.txt) — about
    // 190,000 CPU cycles at 1.789773 MHz, rounded up for safety margin.
    // Tracks the cycle at which $81 was first observed in the current phase
    // so a warm reset only fires once that delay has elapsed; the tracker is
    // cleared whenever status isn't $81 (including right after a reset
    // fires), so a later $81 phase gets its own fresh wait.
    const RESET_DELAY_CYCLES: u64 = 190_000;
    const MAX_RESETS: u32 = 8;
    let mut reset_request_since: Option<u64> = None;
    let mut reset_count: u32 = 0;

    loop {
        let sig_valid =
            bus.read(0x6001) == SIG[0] && bus.read(0x6002) == SIG[1] && bus.read(0x6003) == SIG[2];

        if sig_valid {
            let status = bus.read(0x6000);
            if status == 0x81 {
                let since = *reset_request_since.get_or_insert(total_cycles);
                // Only fire the reset at an instruction boundary (empty
                // micro-op queue) — `clock.step()` below ticks exactly one
                // micro-op at a time, so this loop can observe $81 mid
                // instruction; calling `warm_reset` there would change PC
                // out from under an in-flight micro-op sequence and corrupt
                // execution. The 100ms+ delay window gives ample slack to
                // wait the handful of extra cycles until the current
                // instruction retires.
                let (_, _, queue_len) = cpu.debug_nmi_state();
                if total_cycles.saturating_sub(since) >= RESET_DELAY_CYCLES && queue_len == 0 {
                    reset_count += 1;
                    if reset_count > MAX_RESETS {
                        let text = read_output(bus);
                        print_raw(text.trim());
                        panic!(
                            "{filename}: still requesting reset (status=$81) after {} warm resets",
                            MAX_RESETS
                        );
                    }
                    cpu.warm_reset(bus);
                    let _ = bus.tick_ppu(7);
                    let _ = bus.tick_apu(7);
                    cpu.cycles += 7;
                    total_cycles += 7;
                    reset_request_since = None;
                    continue;
                }
            } else {
                reset_request_since = None;
                if status < 0x80 {
                    let text = read_output(bus);
                    print_raw(&format!(
                        "[{filename}] status={:#04x} text={}",
                        status,
                        text.trim()
                    ));
                    assert_eq!(
                        status,
                        0,
                        "{filename}: ROM reported failure with code {:#04x}\n{}",
                        status,
                        text.trim()
                    );
                    return;
                }
            }
        }

        if total_cycles >= MAX_CYCLES {
            let text = read_output(bus);
            print_raw(text.trim());
            panic!("{filename}: timed out after {} cycles", total_cycles);
        }

        let result = clock.step(cpu, bus);
        if result.nmi && trace_nmi && nmi_count < 30 {
            let gap = cpu.cycles - last_nmi_cycle;
            eprintln!(
                "NMI#{:02} at cycle={} pc={:#06x} gap={}",
                nmi_count, cpu.cycles, cpu.pc, gap
            );
            last_nmi_cycle = cpu.cycles;
            nmi_count += 1;
        }
        total_cycles += result.cycles;
    }
}

fn run_rom(filename: &str) {
    run_rom_impl(filename, false);
}

fn run_rom_impl(filename: &str, trace_nmi: bool) {
    let data = load_rom(filename);
    let cartridge = Cartridge::from_ines(&data)
        .unwrap_or_else(|e| panic!("failed to parse {}: {}", filename, e));
    let mut bus = Bus::new();
    bus.insert_cartridge(cartridge);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    // Pre-advance PPU by 7 cycles (the real 6502 reset takes 7 cycles on
    // hardware). APU advances by 8 to match cpu.cycles starting at 8.
    let _ = bus.tick_ppu(7);
    let _ = bus.tick_apu(8);
    run_until_complete_trace(filename, &mut bus, &mut cpu, trace_nmi);
}

macro_rules! rom_test {
    ($name:ident, $file:expr) => {
        #[test]
        fn $name() {
            run_rom($file);
        }
    };
}

// All ROM test files below were written by Shay Green <gblargg@gmail.com>.

// instr_test-v5/rom_singles — one ROM per addressing mode / instruction group
rom_test!(basics, "instr_test-v5/rom_singles/01-basics.nes");
rom_test!(implied, "instr_test-v5/rom_singles/02-implied.nes");
rom_test!(immediate, "instr_test-v5/rom_singles/03-immediate.nes");
rom_test!(zero_page, "instr_test-v5/rom_singles/04-zero_page.nes");
rom_test!(zp_xy, "instr_test-v5/rom_singles/05-zp_xy.nes");
rom_test!(absolute, "instr_test-v5/rom_singles/06-absolute.nes");
rom_test!(abs_xy, "instr_test-v5/rom_singles/07-abs_xy.nes");
rom_test!(ind_x, "instr_test-v5/rom_singles/08-ind_x.nes");
rom_test!(ind_y, "instr_test-v5/rom_singles/09-ind_y.nes");
rom_test!(branches, "instr_test-v5/rom_singles/10-branches.nes");
rom_test!(stack, "instr_test-v5/rom_singles/11-stack.nes");
rom_test!(jmp_jsr, "instr_test-v5/rom_singles/12-jmp_jsr.nes");
rom_test!(rts, "instr_test-v5/rom_singles/13-rts.nes");
rom_test!(rti, "instr_test-v5/rom_singles/14-rti.nes");
rom_test!(brk, "instr_test-v5/rom_singles/15-brk.nes");
rom_test!(special, "instr_test-v5/rom_singles/16-special.nes");

// instr_test-v5 — full suite (Mapper 1 / MMC1, 256 KB PRG-ROM)
rom_test!(official_only, "instr_test-v5/official_only.nes");

// cpu_interrupts_v2 — interrupt timing and sequencing
rom_test!(
    cpu_interrupts_v2_cli_latency,
    "cpu_interrupts_v2/rom_singles/1-cli_latency.nes"
);
rom_test!(
    cpu_interrupts_v2_nmi_and_brk,
    "cpu_interrupts_v2/rom_singles/2-nmi_and_brk.nes"
);
rom_test!(
    cpu_interrupts_v2_nmi_and_irq,
    "cpu_interrupts_v2/rom_singles/3-nmi_and_irq.nes"
);
rom_test!(
    cpu_interrupts_v2_irq_and_dma,
    "cpu_interrupts_v2/rom_singles/4-irq_and_dma.nes"
);
rom_test!(
    cpu_interrupts_v2_branch_delays_irq,
    "cpu_interrupts_v2/rom_singles/5-branch_delays_irq.nes"
);
rom_test!(
    cpu_interrupts_v2_all,
    "cpu_interrupts_v2/cpu_interrupts.nes"
);

// instr_misc — instruction behaviour edge cases
rom_test!(
    instr_misc_abs_x_wrap,
    "instr_misc/rom_singles/01-abs_x_wrap.nes"
);
rom_test!(
    instr_misc_branch_wrap,
    "instr_misc/rom_singles/02-branch_wrap.nes"
);
rom_test!(
    instr_misc_dummy_reads,
    "instr_misc/rom_singles/03-dummy_reads.nes"
);
rom_test!(
    instr_misc_dummy_reads_apu,
    "instr_misc/rom_singles/04-dummy_reads_apu.nes"
);
rom_test!(instr_misc_all, "instr_misc/instr_misc.nes");

// instr_timing — cycle-accurate instruction timing (Mapper 1 / MMC1)
rom_test!(instr_timing, "instr_timing/instr_timing.nes");

// --- Additional $6000-protocol suites; all assert the ROM's status like the
// suites above. Known failing ones are listed under "Failing $6000-protocol
// tests" in CLAUDE.md's Known gaps. ---

// instr_test-v3 — older instr_test vintage (Mapper 1 / MMC1 for the combined ROMs)
rom_test!(
    instr_test_v3_implied,
    "instr_test-v3/rom_singles/01-implied.nes"
);
rom_test!(
    instr_test_v3_immediate,
    "instr_test-v3/rom_singles/02-immediate.nes"
);
rom_test!(
    instr_test_v3_zero_page,
    "instr_test-v3/rom_singles/03-zero_page.nes"
);
rom_test!(
    instr_test_v3_zp_xy,
    "instr_test-v3/rom_singles/04-zp_xy.nes"
);
rom_test!(
    instr_test_v3_absolute,
    "instr_test-v3/rom_singles/05-absolute.nes"
);
rom_test!(
    instr_test_v3_abs_xy,
    "instr_test-v3/rom_singles/06-abs_xy.nes"
);
rom_test!(
    instr_test_v3_ind_x,
    "instr_test-v3/rom_singles/07-ind_x.nes"
);
rom_test!(
    instr_test_v3_ind_y,
    "instr_test-v3/rom_singles/08-ind_y.nes"
);
rom_test!(
    instr_test_v3_branches,
    "instr_test-v3/rom_singles/09-branches.nes"
);
rom_test!(
    instr_test_v3_stack,
    "instr_test-v3/rom_singles/10-stack.nes"
);
rom_test!(
    instr_test_v3_jmp_jsr,
    "instr_test-v3/rom_singles/11-jmp_jsr.nes"
);
rom_test!(instr_test_v3_rts, "instr_test-v3/rom_singles/12-rts.nes");
rom_test!(instr_test_v3_rti, "instr_test-v3/rom_singles/13-rti.nes");
rom_test!(instr_test_v3_brk, "instr_test-v3/rom_singles/14-brk.nes");
rom_test!(
    instr_test_v3_special,
    "instr_test-v3/rom_singles/15-special.nes"
);
rom_test!(
    instr_test_v3_official_only,
    "instr_test-v3/official_only.nes"
);
rom_test!(instr_test_v3_all_instrs, "instr_test-v3/all_instrs.nes");

// nes_instr_test — another instr_test vintage, rom_singles only (no combined ROM)
rom_test!(
    nes_instr_test_implied,
    "nes_instr_test/rom_singles/01-implied.nes"
);
rom_test!(
    nes_instr_test_immediate,
    "nes_instr_test/rom_singles/02-immediate.nes"
);
rom_test!(
    nes_instr_test_zero_page,
    "nes_instr_test/rom_singles/03-zero_page.nes"
);
rom_test!(
    nes_instr_test_zp_xy,
    "nes_instr_test/rom_singles/04-zp_xy.nes"
);
rom_test!(
    nes_instr_test_absolute,
    "nes_instr_test/rom_singles/05-absolute.nes"
);
rom_test!(
    nes_instr_test_abs_xy,
    "nes_instr_test/rom_singles/06-abs_xy.nes"
);
rom_test!(
    nes_instr_test_ind_x,
    "nes_instr_test/rom_singles/07-ind_x.nes"
);
rom_test!(
    nes_instr_test_ind_y,
    "nes_instr_test/rom_singles/08-ind_y.nes"
);
rom_test!(
    nes_instr_test_branches,
    "nes_instr_test/rom_singles/09-branches.nes"
);
rom_test!(
    nes_instr_test_stack,
    "nes_instr_test/rom_singles/10-stack.nes"
);
rom_test!(
    nes_instr_test_special,
    "nes_instr_test/rom_singles/11-special.nes"
);

// cpu_dummy_writes — RMW double-write behavior (OAM and PPU-memory variants)
rom_test!(
    cpu_dummy_writes_oam,
    "cpu_dummy_writes/cpu_dummy_writes_oam.nes"
);
rom_test!(
    cpu_dummy_writes_ppumem,
    "cpu_dummy_writes/cpu_dummy_writes_ppumem.nes"
);

// cpu_exec_space — CPU execution from I/O address space
rom_test!(
    cpu_exec_space_apu,
    "cpu_exec_space/test_cpu_exec_space_apu.nes"
);
rom_test!(
    cpu_exec_space_ppuio,
    "cpu_exec_space/test_cpu_exec_space_ppuio.nes"
);

// cpu_reset — register/RAM state across a reset
rom_test!(cpu_reset_ram_after_reset, "cpu_reset/ram_after_reset.nes");
rom_test!(cpu_reset_registers, "cpu_reset/registers.nes");

// oam_read / oam_stress — OAM read/DMA edge cases
rom_test!(oam_read, "oam_read/oam_read.nes");
rom_test!(oam_stress, "oam_stress/oam_stress.nes");

// ppu_open_bus — PPU register open-bus behavior
rom_test!(ppu_open_bus, "ppu_open_bus/ppu_open_bus.nes");

// ppu_vbl_nmi — VBL/NMI timing (Mapper 1 for the combined ROM)
rom_test!(
    ppu_vbl_nmi_vbl_basics,
    "ppu_vbl_nmi/rom_singles/01-vbl_basics.nes"
);
rom_test!(
    ppu_vbl_nmi_vbl_set_time,
    "ppu_vbl_nmi/rom_singles/02-vbl_set_time.nes"
);
rom_test!(
    ppu_vbl_nmi_vbl_clear_time,
    "ppu_vbl_nmi/rom_singles/03-vbl_clear_time.nes"
);
rom_test!(
    ppu_vbl_nmi_nmi_control,
    "ppu_vbl_nmi/rom_singles/04-nmi_control.nes"
);
rom_test!(
    ppu_vbl_nmi_nmi_timing,
    "ppu_vbl_nmi/rom_singles/05-nmi_timing.nes"
);
rom_test!(
    ppu_vbl_nmi_suppression,
    "ppu_vbl_nmi/rom_singles/06-suppression.nes"
);
rom_test!(
    ppu_vbl_nmi_nmi_on_timing,
    "ppu_vbl_nmi/rom_singles/07-nmi_on_timing.nes"
);
rom_test!(
    ppu_vbl_nmi_nmi_off_timing,
    "ppu_vbl_nmi/rom_singles/08-nmi_off_timing.nes"
);
rom_test!(
    ppu_vbl_nmi_even_odd_frames,
    "ppu_vbl_nmi/rom_singles/09-even_odd_frames.nes"
);
rom_test!(
    ppu_vbl_nmi_even_odd_timing,
    "ppu_vbl_nmi/rom_singles/10-even_odd_timing.nes"
);
rom_test!(ppu_vbl_nmi_all, "ppu_vbl_nmi/ppu_vbl_nmi.nes");

// Diagnostic: run test 2 with NMI cycle tracing. Not in CI; run manually with:
//   cargo test nmi_and_brk_trace -- --nocapture 2>&1 | head -40
#[test]
#[ignore]
fn nmi_and_brk_trace() {
    run_rom_impl("cpu_interrupts_v2/rom_singles/2-nmi_and_brk.nes", true);
}

// Diagnostic: trace every CRC update call ($E5AE) with the byte being fed.
//   cargo test nmi_brk_crc_trace -- --nocapture 2>&1 | grep CRC | head -60
#[test]
#[ignore]
fn nmi_brk_crc_trace() {
    let data = load_rom("cpu_interrupts_v2/rom_singles/2-nmi_and_brk.nes");
    let cartridge = crate::cartridge::Cartridge::from_ines(&data).unwrap();
    let mut bus = Bus::new();
    bus.insert_cartridge(cartridge);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    let _ = bus.tick_ppu(8);
    let _ = bus.tick_apu(8);

    let mut total_cycles: u64 = 0;
    let mut prev_pc = 0u16;
    let mut crc_count = 0u32;

    loop {
        let sig_valid =
            bus.read(0x6001) == SIG[0] && bus.read(0x6002) == SIG[1] && bus.read(0x6003) == SIG[2];
        if sig_valid {
            let status = bus.read(0x6000);
            if status < 0x80 {
                return; // pass — just collecting trace
            }
        }
        if total_cycles >= MAX_CYCLES {
            panic!("timeout");
        }

        let cycles_before = cpu.cycles;
        if bus.dma_active() {
            bus.tick_dma();
            if bus.tick_ppu(1) {
                cpu.nmi();
            }
            if bus.tick_apu(1) {
                cpu.irq();
            }
            total_cycles += 1;
        } else {
            let pc_now = cpu.pc;
            // $E5B2 is PHA inside the CRC update ($E5AE); A holds the byte being CRC'd
            if pc_now == 0xE5B2 && prev_pc != 0xE5B2 {
                if crc_count < 60 {
                    eprintln!("CRC[{:2}] byte={:#04x}", crc_count, cpu.a);
                }
                crc_count += 1;
            }
            prev_pc = pc_now;

            cpu.tick(&mut bus);
            let delta = cpu.cycles - cycles_before;
            let (extra, extra_nmi) = bus.take_ppu_preadvance();
            let mut got_nmi = extra_nmi;
            let remaining = delta.saturating_sub(extra as u64);
            for _ in 0..remaining {
                if bus.tick_ppu(1) {
                    got_nmi = true;
                }
            }
            if got_nmi {
                cpu.nmi();
            }
            if bus.tick_apu(delta) {
                cpu.irq();
            }
            total_cycles += delta;
        }
    }
}

// Diagnostic: disassemble ROM bytes at known addresses to identify instructions.
// Run with:
//   cargo test nmi_brk_disasm -- --nocapture 2>&1
#[test]
#[ignore]
fn nmi_brk_disasm() {
    let data = load_rom("cpu_interrupts_v2/rom_singles/2-nmi_and_brk.nes");
    let cartridge = crate::cartridge::Cartridge::from_ines(&data).unwrap();
    let mut bus = Bus::new();
    bus.insert_cartridge(cartridge);
    // Dump ROM bytes around the sync loop and row-test setup
    for &base in &[
        0xE200u16, 0xE220u16, 0xE240u16, 0xE260u16, 0xE280u16, 0xE2A0u16, 0xE2C0u16, 0xE2E0u16,
        0xE300u16, 0xE320u16, 0xE340u16, 0xE440u16, 0xE460u16, 0xE480u16,
    ] {
        eprint!("${:04X}:", base);
        for offset in 0..32u16 {
            eprint!(" {:02X}", bus.read(base + offset));
        }
        eprintln!();
    }
}

// Diagnostic: run test 2 and print per-row ($1F, $1D) results plus cycle offsets.
// Run with:
//   cargo test nmi_brk_row_trace -- --nocapture 2>&1 | grep -E 'Row|BRK'
#[test]
#[ignore]
fn nmi_brk_row_trace() {
    let data = load_rom("cpu_interrupts_v2/rom_singles/2-nmi_and_brk.nes");
    let cartridge = crate::cartridge::Cartridge::from_ines(&data).unwrap();
    let mut bus = Bus::new();
    bus.insert_cartridge(cartridge);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    let _ = bus.tick_ppu(8);
    let _ = bus.tick_apu(8);

    let mut total_cycles: u64 = 0;
    let mut row = 0u32;
    let mut prev_pc = 0u16;

    // Per-row cycle tracking.
    const BRK_ADDR: u16 = 0xE34B;
    // STA $2000 (NMI enable) is at $E32B; the T1 opcode-fetch tick is what we capture.
    const STA2000_ADDR: u16 = 0xE32B;
    // Return points of JSR calls between STA$2000 and BRK:
    const AFTER_E442_1: u16 = 0xE332; // after JSR $E442(timing_offset) → PHP
    const AFTER_E458: u16 = 0xE339; // after JSR $E458($73) → LDA #$D7
    const AFTER_E442_2: u16 = 0xE33E; // after JSR $E442($D7) → PLA
    let mut last_nmi_cycle: u64 = 0;
    let mut last_brk_t1_cycle: u64 = 0;
    let mut last_sta2000_t1_cycle: u64 = 0;
    let mut cycle_after_e442_1: u64 = 0;
    let mut cycle_after_e458: u64 = 0;
    let mut cycle_after_e442_2: u64 = 0;
    let mut deferred_nmi = false;

    loop {
        let sig_valid =
            bus.read(0x6001) == SIG[0] && bus.read(0x6002) == SIG[1] && bus.read(0x6003) == SIG[2];
        if sig_valid {
            let status = bus.read(0x6000);
            if status < 0x80 {
                let text = read_output(&mut bus);
                print_raw(text.trim());
                assert_eq!(status, 0, "test failed with code {:#04x}", status);
                return;
            }
        }
        if total_cycles >= MAX_CYCLES {
            panic!("timed out after {} cycles", total_cycles);
        }

        let cycles_before = cpu.cycles;
        let pc_now = cpu.pc;
        if bus.dma_active() {
            bus.tick_dma();
            if bus.tick_ppu(1) {
                cpu.nmi();
            }
            if bus.tick_apu(1) {
                cpu.irq();
            }
            total_cycles += 1;
        } else {
            // Capture PC before tick to detect when $E350 is reached (LDA $1F)
            if pc_now == 0xE350 && prev_pc != 0xE350 {
                let nmi_col = bus.read(0x1F);
                let brk_col = bus.read(0x1D);
                let nmi_minus_brk = last_nmi_cycle as i64 - last_brk_t1_cycle as i64;
                let sta_to_nmi = last_nmi_cycle as i64 - last_sta2000_t1_cycle as i64;
                let sta_to_brk = last_brk_t1_cycle as i64 - last_sta2000_t1_cycle as i64;
                let d1 = cycle_after_e442_1 as i64 - last_sta2000_t1_cycle as i64;
                let d2 = cycle_after_e458 as i64 - last_sta2000_t1_cycle as i64;
                let d3 = cycle_after_e442_2 as i64 - last_sta2000_t1_cycle as i64;
                eprintln!(
                    "Row {:2}: NMI_col={:#04x} BRK_col={:#04x}  nmi-brk={:+}  sta_to_nmi={}  sta_to_brk={}  e442_1={}  e458={}  e442_2={}",
                    row, nmi_col, brk_col, nmi_minus_brk, sta_to_nmi, sta_to_brk, d1, d2, d3
                );
                row += 1;
            }
            prev_pc = pc_now;

            cpu.tick(&mut bus);
            let delta = cpu.cycles - cycles_before;

            // Detect BRK T1 (queue-empty opcode fetch at BRK_ADDR, delta=1).
            if pc_now == BRK_ADDR && delta == 1 {
                last_brk_t1_cycle = cpu.cycles;
            }
            // Detect STA $2000 T1 (queue-empty fetch at STA2000_ADDR, delta=1).
            if pc_now == STA2000_ADDR && delta == 1 {
                last_sta2000_t1_cycle = cpu.cycles;
            }
            // Detect return points of JSR calls (opcode fetch, delta=1).
            if pc_now == AFTER_E442_1 && delta == 1 {
                cycle_after_e442_1 = cpu.cycles;
            }
            if pc_now == AFTER_E458 && delta == 1 {
                cycle_after_e458 = cpu.cycles;
            }
            if pc_now == AFTER_E442_2 && delta == 1 {
                cycle_after_e442_2 = cpu.cycles;
            }

            let (extra, extra_nmi) = bus.take_ppu_preadvance();
            let mut got_nmi = extra_nmi;
            let remaining = delta.saturating_sub(extra as u64);
            let mut new_deferred = false;
            for i in 0..remaining {
                if bus.tick_ppu(1) {
                    if i + 1 == remaining {
                        new_deferred = true;
                    } else {
                        got_nmi = true;
                    }
                }
            }
            if deferred_nmi {
                got_nmi = true;
            }
            deferred_nmi = new_deferred;
            if got_nmi {
                last_nmi_cycle = cpu.cycles;
                cpu.nmi();
            }
            if bus.tick_apu(delta) {
                cpu.irq();
            }
            total_cycles += delta;
        }
    }
}

// Diagnostic: per-tick trace of pending_nmi/nmi_pending/queue_len across the
// BRK-vs-NMI race for a specific set of rows, to find exactly which BRK
// T-state our hijack decision differs from real hardware. Real hardware's
// expected transition from non-hijack (BRK_col=0x36 non-hijack via separate
// IRQ-style push) to hijack (BRK_col=0x00, NMI vectored directly) happens
// between readme rows 2 and 3; ours happens one row later, between rows 3
// and 4. Traces rows 2, 3, 4 to see the difference at the boundary.
//   cargo test nmi_brk_micro_trace -- --nocapture --ignored 2>&1 | less
#[test]
#[ignore]
fn nmi_brk_micro_trace() {
    let data = load_rom("cpu_interrupts_v2/rom_singles/2-nmi_and_brk.nes");
    let cartridge = crate::cartridge::Cartridge::from_ines(&data).unwrap();
    let mut bus = Bus::new();
    bus.insert_cartridge(cartridge);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    let _ = bus.tick_ppu(8);
    let _ = bus.tick_apu(8);

    let mut total_cycles: u64 = 0;
    let mut row = 0u32;
    let mut prev_pc = 0u16;
    let mut deferred_nmi = false;

    const STA2000_ADDR: u16 = 0xE32B;
    const TRACE_ROWS: [u32; 3] = [7, 8, 9];

    loop {
        let sig_valid =
            bus.read(0x6001) == SIG[0] && bus.read(0x6002) == SIG[1] && bus.read(0x6003) == SIG[2];
        if sig_valid {
            let status = bus.read(0x6000);
            if status < 0x80 {
                return; // just collecting trace
            }
        }
        if total_cycles >= MAX_CYCLES {
            panic!("timeout");
        }

        let cycles_before = cpu.cycles;
        let pc_now = cpu.pc;
        let trace_this_row = TRACE_ROWS.contains(&row);

        if bus.dma_active() {
            bus.tick_dma();
            if bus.tick_ppu(1) {
                cpu.nmi();
            }
            if bus.tick_apu(1) {
                cpu.irq();
            }
            total_cycles += 1;
        } else {
            if pc_now == 0xE350 && prev_pc != 0xE350 {
                if trace_this_row {
                    eprintln!("--- end row {} ---", row);
                }
                row += 1;
                if TRACE_ROWS.contains(&row) {
                    eprintln!("=== begin row {} ===", row);
                }
            }
            if pc_now == STA2000_ADDR && prev_pc != STA2000_ADDR && trace_this_row {
                eprintln!(
                    "  [row {}] STA $2000 (NMI armed) at cycle={}",
                    row, cpu.cycles
                );
            }
            prev_pc = pc_now;

            let (pn_before, np_before, ql_before) = cpu.debug_nmi_state();
            cpu.tick(&mut bus);
            let delta = cpu.cycles - cycles_before;
            let (pn_after, np_after, ql_after) = cpu.debug_nmi_state();

            let (extra, extra_nmi) = bus.take_ppu_preadvance();
            let mut got_nmi = extra_nmi;
            let remaining = delta.saturating_sub(extra as u64);
            let mut new_deferred = false;
            for i in 0..remaining {
                if bus.tick_ppu(1) {
                    if i + 1 == remaining {
                        new_deferred = true;
                    } else {
                        got_nmi = true;
                    }
                }
            }
            let delivered_deferred = deferred_nmi;
            if deferred_nmi {
                got_nmi = true;
            }
            deferred_nmi = new_deferred;
            if got_nmi {
                cpu.nmi();
            }
            if bus.tick_apu(delta) {
                cpu.irq();
            }
            total_cycles += delta;

            if trace_this_row {
                eprintln!(
                    "  [row {}] pc={:#06x} delta={} cycles={} ql:{}->{} pending_nmi:{}->{} nmi_pending:{}->{}{}{}",
                    row,
                    pc_now,
                    delta,
                    cpu.cycles,
                    ql_before,
                    ql_after,
                    pn_before,
                    pn_after,
                    np_before,
                    np_after,
                    if got_nmi {
                        "  <== NMI EDGE (cpu.nmi() called)"
                    } else {
                        ""
                    },
                    if new_deferred {
                        "  [defer-armed]"
                    } else if delivered_deferred {
                        "  [deferred-delivered]"
                    } else {
                        ""
                    }
                );
            }
        }
    }
}

// Diagnostic: generic runtime tracer for test 3 (nmi_and_irq). Unlike test 2's
// tracers, this doesn't assume any particular ROM code layout — it watches for
// changes to the $1D/$1F result bytes (the same convention test 2 used) and
// logs every NMI/IRQ delivery with PC and cycle, to let the row/loop structure
// be inferred empirically rather than from manual disassembly.
//   cargo test nmi_irq_generic_trace -- --nocapture --ignored 2>&1 | less
#[test]
#[ignore]
fn nmi_irq_generic_trace() {
    let data = load_rom("cpu_interrupts_v2/rom_singles/3-nmi_and_irq.nes");
    let cartridge = crate::cartridge::Cartridge::from_ines(&data).unwrap();
    let mut bus = Bus::new();
    bus.insert_cartridge(cartridge);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    let _ = bus.tick_ppu(8);
    let _ = bus.tick_apu(8);

    let mut total_cycles: u64 = 0;
    let mut deferred_nmi = false;
    let mut last_1f = 0xFFu16; // sentinel, zp values are u8 so 0xFF..=0xFF won't collide falsely for long
    let mut last_1d = 0xFFu16;
    let mut event_count = 0u32;

    loop {
        let sig_valid =
            bus.read(0x6001) == SIG[0] && bus.read(0x6002) == SIG[1] && bus.read(0x6003) == SIG[2];
        if sig_valid {
            let status = bus.read(0x6000);
            if status < 0x80 {
                return; // just collecting trace
            }
        }
        if total_cycles >= MAX_CYCLES {
            panic!("timeout");
        }

        let v1f = bus.read(0x1F) as u16;
        let v1d = bus.read(0x1D) as u16;
        if (v1f != last_1f || v1d != last_1d) && event_count < 200 {
            eprintln!(
                "RESULT change at cycle={} pc={:#06x}: $1F={:#04x} $1D={:#04x}",
                cpu.cycles, cpu.pc, v1f, v1d
            );
            last_1f = v1f;
            last_1d = v1d;
            event_count += 1;
        }

        let cycles_before = cpu.cycles;
        if bus.dma_active() {
            bus.tick_dma();
            if bus.tick_ppu(1) {
                cpu.nmi();
            }
            if bus.tick_apu(1) {
                cpu.irq();
            }
            total_cycles += 1;
        } else {
            cpu.tick(&mut bus);
            let delta = cpu.cycles - cycles_before;
            let (extra, extra_nmi) = bus.take_ppu_preadvance();
            let mut got_nmi = extra_nmi;
            let remaining = delta.saturating_sub(extra as u64);
            let mut new_deferred = false;
            for i in 0..remaining {
                if bus.tick_ppu(1) {
                    if i + 1 == remaining {
                        new_deferred = true;
                    } else {
                        got_nmi = true;
                    }
                }
            }
            if deferred_nmi {
                got_nmi = true;
            }
            deferred_nmi = new_deferred;
            if got_nmi {
                if event_count < 200 {
                    eprintln!("NMI EDGE at cycle={} pc={:#06x}", cpu.cycles, cpu.pc);
                }
                cpu.nmi();
            }
            if bus.tick_apu(delta) {
                cpu.irq();
            }
            total_cycles += delta;
        }
    }
}

// Diagnostic: per-row cycle-position trace for test 3 (nmi_and_irq), analogous
// to test 2's nmi_brk_row_trace. Row structure (from manual disassembly):
//   $E323: EOR #$FF; CLC; ADC #$0D   -- A = 12-row
//   $E328: JSR $E200                 -- sync to VBlank start, NMI/IRQ disabled
//   $E32B: JSR $E442                 -- delay(A=12-row), THE row-varying knob
//   ... fixed-length delays (JSR $E458/$E442 with constant args) ...
//   $E351: STA $2000,#$80            -- arm NMI (mid-VBlank instant-fire quirk)
//   $E354: LDA $4015; $E357: CLI     -- ack + enable IRQ
//   $E35E: CLV; $E35F: SEC; $E360: LDA #$01 ("LDA #1"); $E362: CLC; $E363: NOP
//   $E364: LDX $1F; $E366: LDY $1D   -- capture results
// Traces cycle position of $E32B (delay-call start) and $E360 (LDA #1) plus
// every NMI edge, for the first few rows, to find where our cycle counts
// diverge from the expected 1-cycle-per-row shift.
//   cargo test nmi_irq_row_trace -- --nocapture --ignored 2>&1 | grep Row
#[test]
#[ignore]
fn nmi_irq_row_trace() {
    let data = load_rom("cpu_interrupts_v2/rom_singles/3-nmi_and_irq.nes");
    let cartridge = crate::cartridge::Cartridge::from_ines(&data).unwrap();
    let mut bus = Bus::new();
    bus.insert_cartridge(cartridge);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    let _ = bus.tick_ppu(8);
    let _ = bus.tick_apu(8);

    let mut total_cycles: u64 = 0;
    let mut deferred_nmi = false;
    let mut prev_pc = 0u16;
    let mut row = -1i32; // becomes 0 on first LDX $1F hit
    let mut last_delay_call_cycle: u64 = 0;
    let mut last_lda1_cycle: u64 = 0;
    let mut last_nmi_cycle: u64 = 0;
    let mut have_nmi_this_row = false;

    const DELAY_CALL: u16 = 0xE32B;
    const LDA1: u16 = 0xE360;
    const CAPTURE: u16 = 0xE364; // LDX $1F

    loop {
        let sig_valid =
            bus.read(0x6001) == SIG[0] && bus.read(0x6002) == SIG[1] && bus.read(0x6003) == SIG[2];
        if sig_valid {
            let status = bus.read(0x6000);
            if status < 0x80 {
                return;
            }
        }
        if total_cycles >= MAX_CYCLES {
            panic!("timeout");
        }

        let cycles_before = cpu.cycles;
        let pc_now = cpu.pc;

        if pc_now == CAPTURE && prev_pc != CAPTURE {
            if row >= 0 {
                let nmi_col = bus.read(0x1F);
                let irq_col = bus.read(0x1D);
                eprintln!(
                    "Row {:2}: NMI_col={:#04x} IRQ_col={:#04x}  delay_call_cyc={} lda1_cyc={} lda1-delay={}  nmi_cyc={} nmi-lda1={:+} had_nmi={}",
                    row,
                    nmi_col,
                    irq_col,
                    last_delay_call_cycle,
                    last_lda1_cycle,
                    last_lda1_cycle as i64 - last_delay_call_cycle as i64,
                    last_nmi_cycle,
                    last_nmi_cycle as i64 - last_lda1_cycle as i64,
                    have_nmi_this_row
                );
            }
            row += 1;
            have_nmi_this_row = false;
        }
        if pc_now == DELAY_CALL && prev_pc != DELAY_CALL {
            last_delay_call_cycle = cpu.cycles;
        }
        if pc_now == LDA1 && prev_pc != LDA1 {
            last_lda1_cycle = cpu.cycles;
        }
        prev_pc = pc_now;

        if bus.dma_active() {
            bus.tick_dma();
            if bus.tick_ppu(1) {
                cpu.nmi();
            }
            if bus.tick_apu(1) {
                cpu.irq();
            }
            total_cycles += 1;
        } else {
            cpu.tick(&mut bus);
            let delta = cpu.cycles - cycles_before;
            let (extra, extra_nmi) = bus.take_ppu_preadvance();
            let mut got_nmi = extra_nmi;
            let remaining = delta.saturating_sub(extra as u64);
            let mut new_deferred = false;
            for i in 0..remaining {
                if bus.tick_ppu(1) {
                    if i + 1 == remaining {
                        new_deferred = true;
                    } else {
                        got_nmi = true;
                    }
                }
            }
            if deferred_nmi {
                got_nmi = true;
            }
            deferred_nmi = new_deferred;
            if got_nmi {
                last_nmi_cycle = cpu.cycles;
                have_nmi_this_row = true;
                cpu.nmi();
            }
            if bus.tick_apu(delta) {
                cpu.irq();
            }
            total_cycles += delta;
        }
    }
}

// Diagnostic: full stage-by-stage cycle breakdown of $E200's VBlank-sync
// routine plus the surrounding row setup, for the first 3 rows of test 3.
// Marks: E200 entry, first-loop exit ($E20A poll), fixed-delay-1 return
// ($E21B), second-loop entry ($E224/$E227), second-loop exit ($E230), E200
// RTS ($E233), the row-varying delay call ($E32B), fixed-delay-2/3 returns,
// STA $2000 arm ($E351), LDA $4015 ack ($E354), CLI ($E357), LDA #1 ($E360).
//   cargo test nmi_irq_stage_trace -- --nocapture --ignored 2>&1 | grep row
#[test]
#[ignore]
fn nmi_irq_stage_trace() {
    let data = load_rom("cpu_interrupts_v2/rom_singles/3-nmi_and_irq.nes");
    let cartridge = crate::cartridge::Cartridge::from_ines(&data).unwrap();
    let mut bus = Bus::new();
    bus.insert_cartridge(cartridge);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    let _ = bus.tick_ppu(8);
    let _ = bus.tick_apu(8);

    let mut total_cycles: u64 = 0;
    let mut deferred_nmi = false;
    let mut prev_pc = 0u16;
    let mut row = -1i32;
    let mut logged_irq_this_row = false;

    const MARKS: &[(u16, &str)] = &[
        (0xE200, "E200_entry"),
        (0xE20F, "loop1_exit"),
        (0xE21B, "fixdelay1_ret"),
        (0xE224, "loop2_start"),
        (0xE232, "loop2_exit"),
        (0xE234, "E200_ret"),
        (0xE32B, "delay_call"),
        (0xE32E, "delay_call_ret"),
        (0xE33A, "fixdelay2_ret"),
        (0xE34D, "fixdelay3_ret"),
        (0xE351, "sta2000_arm"),
        (0xE354, "lda4015_ack"),
        (0xE357, "cli"),
        (0xE360, "lda1"),
        (0xE364, "capture"),
    ];

    loop {
        let sig_valid =
            bus.read(0x6001) == SIG[0] && bus.read(0x6002) == SIG[1] && bus.read(0x6003) == SIG[2];
        if sig_valid {
            let status = bus.read(0x6000);
            if status < 0x80 {
                return;
            }
        }
        if total_cycles >= MAX_CYCLES {
            panic!("timeout");
        }

        let cycles_before = cpu.cycles;
        let pc_now = cpu.pc;
        let fine_trace = (4..=6).contains(&row) && (0xE310..=0xE365).contains(&pc_now);

        if (0..12).contains(&row) {
            for (addr, label) in MARKS {
                if pc_now == *addr && prev_pc != *addr {
                    eprintln!("row {} {} cycle={}", row, label, cpu.cycles);
                }
            }
        }
        if pc_now == 0xE364 && prev_pc != 0xE364 {
            row += 1;
            logged_irq_this_row = false;
        }
        prev_pc = pc_now;

        if bus.dma_active() {
            bus.tick_dma();
            if bus.tick_ppu(1) {
                cpu.nmi();
            }
            if bus.tick_apu(1) {
                cpu.irq();
            }
            total_cycles += 1;
        } else {
            let (pn_before, np_before, ql_before) = cpu.debug_nmi_state();
            cpu.tick(&mut bus);
            let delta = cpu.cycles - cycles_before;
            if fine_trace {
                let (pn_after, np_after, ql_after) = cpu.debug_nmi_state();
                eprintln!(
                    "  fine pc={:#06x} delta={} cycles={} ql:{}->{} pending_nmi:{}->{} nmi_pending:{}->{}",
                    pc_now,
                    delta,
                    cpu.cycles,
                    ql_before,
                    ql_after,
                    pn_before,
                    pn_after,
                    np_before,
                    np_after
                );
            }
            let (extra, extra_nmi) = bus.take_ppu_preadvance();
            let mut got_nmi = extra_nmi;
            let remaining = delta.saturating_sub(extra as u64);
            let mut new_deferred = false;
            for i in 0..remaining {
                if bus.tick_ppu(1) {
                    if i + 1 == remaining {
                        new_deferred = true;
                    } else {
                        got_nmi = true;
                    }
                }
            }
            if deferred_nmi {
                got_nmi = true;
            }
            deferred_nmi = new_deferred;
            if got_nmi {
                if (0..12).contains(&row) {
                    eprintln!(
                        "row {} NMI_EDGE cycle={} pc={:#06x}",
                        row, cpu.cycles, pc_now
                    );
                }
                cpu.nmi();
            }
            if bus.tick_apu(delta) {
                if (0..12).contains(&row) && !logged_irq_this_row {
                    eprintln!(
                        "row {} IRQ_EDGE(first) cycle={} pc={:#06x}",
                        row, cpu.cycles, pc_now
                    );
                    logged_irq_this_row = true;
                }
                cpu.irq();
            }
            total_cycles += delta;
        }
    }
}

// Diagnostic: isolate `$E442` (fine cycle-exact delay) and `$E458` (coarse
// delay-loop) from all PPU/NMI/interrupt machinery entirely, to measure their
// exact CPU-cycle cost against a value computed independently by hand. Uses
// TestBus (flat 64 KB, no side effects, no PPU) — this is a pure CPU
// instruction-cycle-counting question, nothing else. Bytes copied verbatim
// from `3-nmi_and_irq.nes` at $E440-$E48A (identical in `2-nmi_and_brk.nes`
// too — shared blargg framework code, confirmed by independent disassembly
// of both ROMs this session).
//   cargo test isolate_delay_routines -- --nocapture --ignored
#[test]
#[ignore]
fn isolate_delay_routines() {
    use crate::tests::TestBus;

    // $E440-$E48A verbatim.
    const DELAY_CODE: &[u8] = &[
        0xE9, 0x07, 0xC9, 0x07, 0xB0, 0xFA, 0x4A, 0xB0, 0x00, 0xF0, 0x05, 0x4A, 0xF0, 0x04, 0x90,
        0x02, // E440
        0xD0, 0x00, 0x60, 0xC9, 0x00, 0xD0, 0x01, 0x60, 0x48, 0xA9, 0xD7, 0x20, 0x42, 0xE4, 0x68,
        0x18, // E450
        0x69, 0xFF, 0xD0, 0xF4, 0x60, 0xC9, 0x00, 0xD0, 0x01, 0x60, 0x48, 0xA9, 0xCA, 0x20, 0x42,
        0xE4, // E460
        0xA9, 0xFF, 0x20, 0x58, 0xE4, 0x68, 0x18, 0x69, 0xFF, 0xD0, 0xEF, 0x60, // E470
    ];

    // Measures total cycles for: LDA #a_val ; JSR target ; <lands here>
    // Subtract 8 (LDA=2, JSR=6) to get the routine's own cost including its RTS.
    fn measure(target: u16, a_val: u8) -> u64 {
        let mut bus = TestBus::new();
        for (i, &b) in DELAY_CODE.iter().enumerate() {
            bus.mem[0xE440 + i] = b;
        }
        let harness = 0x0300u16;
        bus.mem[harness as usize] = 0xA9; // LDA #imm
        bus.mem[harness as usize + 1] = a_val;
        bus.mem[harness as usize + 2] = 0x20; // JSR abs
        bus.mem[harness as usize + 3] = (target & 0xFF) as u8;
        bus.mem[harness as usize + 4] = (target >> 8) as u8;
        let halt = harness + 5;
        bus.mem[halt as usize] = 0x4C; // JMP abs (self, infinite loop)
        bus.mem[halt as usize + 1] = (halt & 0xFF) as u8;
        bus.mem[halt as usize + 2] = (halt >> 8) as u8;

        let mut cpu = Cpu::new();
        cpu.reset(&mut bus);
        cpu.pc = harness;
        cpu.sp = 0xFD;
        let start_cycles = cpu.cycles;
        // Break BEFORE dispatching the halt address as a fresh instruction —
        // checking after tick() would miss it, since fetch() already advances
        // PC past `halt` on the very tick that lands there.
        loop {
            let (_, _, queue_len) = cpu.debug_nmi_state();
            if cpu.pc == halt && queue_len == 0 {
                break;
            }
            cpu.tick(&mut bus);
        }
        cpu.cycles - start_cycles
    }

    // $E442 entry: "delay A cycles" (from JSR to RTS, per the modulo-7 loop).
    for &a in &[0u8, 1, 6, 7, 8, 14, 15, 0x15, 0x18, 0x37, 0xB8, 0xD7] {
        let total = measure(0xE442, a);
        eprintln!(
            "E442(A={:#04x}={:3}): total={} (routine-only, total-8) = {}",
            a,
            a,
            total,
            total.saturating_sub(8)
        );
    }
    eprintln!("---");
    // $E458 entry: coarse loop calling E442(A=$D7) repeatedly, A times.
    for &a in &[1u8, 2, 3, 0x73, 0x74] {
        let total = measure(0xE458, a);
        eprintln!(
            "E458(A={:#04x}={:3}): total={} (routine-only, total-8) = {}",
            a,
            a,
            total,
            total.saturating_sub(8)
        );
    }
}

// Diagnostic: sweep the VBlank double-poll loop pattern from test 3's $E200
// (`BIT $2002; loop: BIT $2002; BPL loop`) across a range of starting phases
// relative to VBlank onset, to find any non-monotonic anomaly in the exit
// cycle (a 1-cycle shift in start phase should shift the exit cycle by
// exactly 0 or 1 — any jump of 2+, or any flat/backward step, pinpoints a
// bug in the $2002-read pre-advance mechanism's interaction with repeated
// polling). Uses the real `Bus` (has a PPU) but no cartridge — PPU dot/
// scanline progression doesn't depend on cartridge presence.
//   cargo test sweep_vbl_poll_loop -- --nocapture --ignored
#[test]
#[ignore]
fn sweep_vbl_poll_loop() {
    // BIT $2002 (E207-equiv); loop: BIT $2002 (E20A-equiv); BPL loop (E20D-equiv); halt.
    const CODE: &[u8] = &[
        0x2C, 0x02, 0x20, 0x2C, 0x02, 0x20, 0x10, 0xFB, 0x4C, 0x08, 0x03,
    ];
    let base = 0x0300u16;
    let halt = base + 8;

    // VBlank sets at scanline 241, dot 1: dot offset 241*341+1 = 82082 from (0,0).
    const VBL_DOT: u64 = 241 * 341 + 1;

    fn run_from(pre_cycles: u64, code: &[u8], base: u16, halt: u16) -> u64 {
        let mut bus = Bus::new();
        for (i, &b) in code.iter().enumerate() {
            bus.write((base as usize + i) as u16, b);
        }
        let mut cpu = Cpu::new();
        cpu.reset(&mut bus);
        cpu.pc = base;
        cpu.sp = 0xFD;
        // Advance PPU (and APU, to keep them in step, though APU is irrelevant
        // here) by pre_cycles BEFORE starting the poll, to control starting phase.
        let _ = bus.tick_ppu(pre_cycles);
        let start_cycles = cpu.cycles;
        let mut deferred_nmi = false;
        loop {
            let (_, _, queue_len) = cpu.debug_nmi_state();
            if cpu.pc == halt && queue_len == 0 {
                break;
            }
            let cycles_before = cpu.cycles;
            cpu.tick(&mut bus);
            let delta = cpu.cycles - cycles_before;
            let (extra, extra_nmi) = bus.take_ppu_preadvance();
            let mut got_nmi = extra_nmi;
            let remaining = delta.saturating_sub(extra as u64);
            let mut new_deferred = false;
            for i in 0..remaining {
                if bus.tick_ppu(1) {
                    if i + 1 == remaining {
                        new_deferred = true;
                    } else {
                        got_nmi = true;
                    }
                }
            }
            if deferred_nmi {
                got_nmi = true;
            }
            deferred_nmi = new_deferred;
            let _ = got_nmi; // NMI is masked off ($2000 untouched here); irrelevant to this loop.
        }
        cpu.cycles - start_cycles
    }

    // Sweep starting phase across a range that straddles VBlank onset.
    // pre_cycles chosen so 3*pre_cycles lands from ~30 dots before VBL_DOT
    // to ~10 dots after, i.e. covers the exact boundary several times over.
    let center = VBL_DOT / 3; // CPU cycles to land near the boundary
    // The invariant that must hold: absolute_exit = pre_cycles + exit_delta is
    // the ABSOLUTE cycle (from PPU dot 0) at which the poll observes VBlank.
    // For any pre_cycles starting strictly before that moment, absolute_exit
    // should be the SAME constant (the first read that can observe it) —
    // it should NOT vary with pre_cycles until pre_cycles itself passes that
    // point, after which absolute_exit should track pre_cycles + a small
    // constant (poll exits almost immediately since VBlank is already set).
    for offset in -15i64..=10 {
        let pre_cycles = (center as i64 + offset) as u64;
        let exit_delta = run_from(pre_cycles, CODE, base, halt);
        let absolute_exit = pre_cycles + exit_delta;
        let start_dot = pre_cycles * 3;
        eprintln!(
            "pre_cycles={:6} start_dot={:6} (vbl_dot={}) exit_delta={:4} absolute_exit={:6}",
            pre_cycles, start_dot, VBL_DOT, exit_delta, absolute_exit
        );
    }
}

// Diagnostic: isolate a SINGLE non-looping `BIT $2002` read (no loop
// periodicity to confound the picture) and sweep the exact pre-tick cycle
// count to find the precise boundary at which the read starts observing
// VBlank as set (N flag from BIT reflects $2002 bit 7). This directly tests
// the $2002-read pre-advance mechanism's cycle-exactness in isolation.
//   cargo test sweep_single_2002_read -- --nocapture --ignored
#[test]
#[ignore]
fn sweep_single_2002_read() {
    // BIT $2002 ; halt (JMP self)
    const CODE: &[u8] = &[0x2C, 0x02, 0x20, 0x4C, 0x03, 0x03];
    let base = 0x0300u16;
    let halt = base + 3;
    const VBL_DOT: u64 = 241 * 341 + 1;

    fn run_from_correct(pre_cycles: u64, code: &[u8], base: u16, halt: u16) -> (bool, u64) {
        let mut bus = Bus::new();
        for (i, &b) in code.iter().enumerate() {
            bus.write((base as usize + i) as u16, b);
        }
        let mut cpu = Cpu::new();
        cpu.reset(&mut bus);
        cpu.pc = base;
        cpu.sp = 0xFD;
        let _ = bus.tick_ppu(pre_cycles);
        let start = cpu.cycles;
        loop {
            let (_, _, queue_len) = cpu.debug_nmi_state();
            if cpu.pc == halt && queue_len == 0 {
                return (cpu.p & 0x80 != 0, cpu.cycles - start);
            }
            let cycles_before = cpu.cycles;
            cpu.tick(&mut bus);
            let delta = cpu.cycles - cycles_before;
            let (extra, _) = bus.take_ppu_preadvance();
            let remaining = delta.saturating_sub(extra as u64);
            for _ in 0..remaining {
                bus.tick_ppu(1);
            }
        }
    }

    let center = VBL_DOT / 3;
    for offset in -5i64..=5 {
        let pre_cycles = (center as i64 + offset) as u64;
        let (n_flag, cycles_used) = run_from_correct(pre_cycles, CODE, base, halt);
        let start_dot = pre_cycles * 3;
        eprintln!(
            "pre_cycles={:6} start_dot={:6} (vbl_dot={}) N_flag(vblank_observed)={} cycles_used={}",
            pre_cycles, start_dot, VBL_DOT, n_flag, cycles_used
        );
    }

    eprintln!("--- dot-exact sweep (bypassing tick_ppu's 3-dot granularity) ---");
    fn run_from_dot(start_dot: u64, code: &[u8], base: u16, halt: u16) -> bool {
        let mut bus = Bus::new();
        for (i, &b) in code.iter().enumerate() {
            bus.write((base as usize + i) as u16, b);
        }
        let mut cpu = Cpu::new();
        cpu.reset(&mut bus);
        cpu.pc = base;
        cpu.sp = 0xFD;
        let scanline = (start_dot / 341) as u16;
        let dot = (start_dot % 341) as u16;
        bus.ppu.debug_set_position(dot, scanline);
        loop {
            let (_, _, queue_len) = cpu.debug_nmi_state();
            if cpu.pc == halt && queue_len == 0 {
                return cpu.p & 0x80 != 0;
            }
            let cycles_before = cpu.cycles;
            cpu.tick(&mut bus);
            let delta = cpu.cycles - cycles_before;
            let (extra, _) = bus.take_ppu_preadvance();
            let remaining = delta.saturating_sub(extra as u64);
            for _ in 0..remaining {
                bus.tick_ppu(1);
            }
        }
    }
    for d in (VBL_DOT - 15)..=(VBL_DOT + 5) {
        let n_flag = run_from_dot(d, CODE, base, halt);
        eprintln!("start_dot={:6} (vbl_dot={}) N_flag={}", d, VBL_DOT, n_flag);
    }
}

// Diagnostic: test sync_vbl's DOCUMENTED CONTRACT directly, using the real
// ROM's actual $E200 bytes (via a real cartridge, so E440-E48A's delay
// routines are automatically present too — no hand-transcription). Source
// (tests/roms/cpu/cpu_interrupts_v2/source/common/sync_vbl.s) documents:
//   "Reading PPUSTATUS 29768 clocks or later after return will have bit 7
//    set. Reading PPUSTATUS immediately will have bit 7 clear."
// sync_vbl itself (E200-E233) is a sophisticated multi-iteration precision
// convergence loop (NOT the simple single-poll-loop earlier tests modeled) —
// testing its CONTRACT directly sidesteps needing to hand-verify every
// iteration of that convergence mechanism.
//   cargo test verify_sync_vbl_contract -- --nocapture --ignored
#[test]
#[ignore]
fn verify_sync_vbl_contract() {
    let data = load_rom("cpu_interrupts_v2/rom_singles/3-nmi_and_irq.nes");

    fn run_sync_vbl(data: &[u8], pre_cycles: u64) -> (bool, bool, u64) {
        let cartridge = crate::cartridge::Cartridge::from_ines(data).unwrap();
        let mut bus = Bus::new();
        bus.insert_cartridge(cartridge);
        let mut cpu = Cpu::new();
        cpu.reset(&mut bus);
        // Harness in RAM: JSR $E200 ; halt (JMP self). Gives sync_vbl's RTS a
        // real return address to pop, so we can detect completion cleanly.
        let harness = 0x0300u16;
        bus.write(harness, 0x20); // JSR abs
        bus.write(harness + 1, 0x00);
        bus.write(harness + 2, 0xE2);
        let halt = harness + 3;
        bus.write(halt, 0x4C); // JMP abs (self)
        bus.write(halt + 1, (halt & 0xFF) as u8);
        bus.write(halt + 2, (halt >> 8) as u8);
        cpu.pc = harness;
        cpu.sp = 0xFD;
        let _ = bus.tick_ppu(pre_cycles);
        let mut deferred_nmi = false;
        let mut iterations = 0u64;
        loop {
            let (_, _, queue_len) = cpu.debug_nmi_state();
            if cpu.pc == halt && queue_len == 0 {
                break;
            }
            iterations += 1;
            if iterations > 50_000_000 {
                panic!("sync_vbl never returned (pre_cycles={})", pre_cycles);
            }
            let cycles_before = cpu.cycles;
            cpu.tick(&mut bus);
            let delta = cpu.cycles - cycles_before;
            let (extra, extra_nmi) = bus.take_ppu_preadvance();
            let mut got_nmi = extra_nmi;
            let remaining = delta.saturating_sub(extra as u64);
            let mut new_deferred = false;
            for i in 0..remaining {
                if bus.tick_ppu(1) {
                    if i + 1 == remaining {
                        new_deferred = true;
                    } else {
                        got_nmi = true;
                    }
                }
            }
            if deferred_nmi {
                got_nmi = true;
            }
            deferred_nmi = new_deferred;
            let _ = got_nmi; // NMI masked off throughout sync_vbl; irrelevant here.
        }
        let ret_cycles = cpu.cycles;
        let immediate = bus.read(0x2002) & 0x80 != 0;
        let _ = bus.tick_ppu(29768);
        let later = bus.read(0x2002) & 0x80 != 0;
        (!immediate, later, ret_cycles)
    }

    for &pre_cycles in &[
        0u64, 100, 1000, 10000, 27390, 27395, 82175, 82182, 82183, 150000,
    ] {
        let (immediate_clear, later_set, ret_cycles) = run_sync_vbl(&data, pre_cycles);
        eprintln!(
            "pre_cycles={:8} immediate_clear={} later_set={} ret_cycles={} contract_ok={}",
            pre_cycles,
            immediate_clear,
            later_set,
            ret_cycles,
            immediate_clear && later_set
        );
    }
}

// Diagnostic: isolate the APU's frame-IRQ absolute timing, completely
// separate from CPU/PPU. Writes $4017=$00 (mode 0, IRQ enabled) then ticks
// one CPU cycle at a time, recording the first cycle (relative to the write)
// at which `tick()` reports the IRQ line asserted. Compares against the
// hand-derived expectation from `src/apu/mod.rs`'s own MODE0 table:
// frame_reset_delay(7) + first IRQ step (29828) = 29835 cycles after the
// write's APPLICATION (the delay is anchored to the start of the writing
// instruction — see the $4017 handler's derivation comment). This directly
// tests whether the APU's own internal timing table fires at the expected
// absolute cycle, with zero CPU/PPU involvement.
//   cargo test isolate_apu_frame_irq_timing -- --nocapture --ignored
#[test]
#[ignore]
fn isolate_apu_frame_irq_timing() {
    use crate::apu::Apu;
    let mut apu = Apu::new();
    apu.write(0x4017, 0x00);
    let mut first_irq_cycle: Option<u64> = None;
    for cycle in 1u64..=40_000 {
        if apu.tick(1) {
            first_irq_cycle = Some(cycle);
            break;
        }
    }
    eprintln!(
        "First IRQ asserted at cycle={:?} after $4017=$00 write (expected: 7 + 29828 = 29835)",
        first_irq_cycle
    );
    assert_eq!(first_irq_cycle, Some(29_835));
}

// Diagnostic: fine-grained NMI-vs-IRQ arbitration trace for a chosen row of
// test 3, with an optional IRQ-defer experiment (see debug log) reapplied
// locally for comparison (not in the shared run loop — investigation-only).
// Found: row 0 is byte-correct under the plain baseline (IRQ preempts LDA #1,
// begins its own service sequence, NMI arrives mid-sequence and hijacks the
// vector fetch at T6 — the NMI handler runs but with IRQ's own pushed P,
// giving $1F=$23/$1D=$00, matching the readme exactly). The IRQ-defer
// experiment breaks this (IRQ becomes pending 1 cycle too late, letting
// LDA #1 slip through unpreempted) — confirmed wrong, not a fix.
//   cargo test nmi_irq_row0_arbitration_trace -- --nocapture --ignored
#[test]
#[ignore]
fn nmi_irq_row0_arbitration_trace() {
    fn run(data: &[u8], defer_irq: bool, trace_row: i32) {
        let cartridge = crate::cartridge::Cartridge::from_ines(data).unwrap();
        let mut bus = Bus::new();
        bus.insert_cartridge(cartridge);
        let mut cpu = Cpu::new();
        cpu.reset(&mut bus);
        let _ = bus.tick_ppu(8);
        let _ = bus.tick_apu(8);

        let mut total_cycles: u64 = 0;
        let mut deferred_nmi = false;
        let mut deferred_irq = false;
        let mut irq_line_high = false;
        let mut prev_pc = 0u16;
        let mut row = -1i32;

        eprintln!("=== defer_irq={} trace_row={} ===", defer_irq, trace_row);

        loop {
            let sig_valid = bus.read(0x6001) == SIG[0]
                && bus.read(0x6002) == SIG[1]
                && bus.read(0x6003) == SIG[2];
            if sig_valid {
                let status = bus.read(0x6000);
                if status < 0x80 {
                    return;
                }
            }
            if total_cycles >= MAX_CYCLES {
                panic!("timeout");
            }

            let cycles_before = cpu.cycles;
            let pc_now = cpu.pc;
            let fine_trace = row == trace_row && (0xE357..=0xE366).contains(&pc_now);

            if pc_now == 0xE364 && prev_pc != 0xE364 {
                if row == trace_row {
                    eprintln!(
                        "row {} result: $1F={:#04x} $1D={:#04x}",
                        row,
                        bus.read(0x1F),
                        bus.read(0x1D)
                    );
                    return;
                }
                row += 1;
            }
            prev_pc = pc_now;

            if bus.dma_active() {
                bus.tick_dma();
                if bus.tick_ppu(1) {
                    cpu.nmi();
                }
                if bus.tick_apu(1) {
                    cpu.irq();
                }
                total_cycles += 1;
            } else {
                let (pn_before, np_before, ql_before) = cpu.debug_nmi_state();
                let (irqp_before, inhibit_before, flagi_before) = cpu.debug_irq_state();
                cpu.tick(&mut bus);
                let delta = cpu.cycles - cycles_before;

                let (extra, extra_nmi) = bus.take_ppu_preadvance();
                let mut got_nmi = extra_nmi;
                let remaining = delta.saturating_sub(extra as u64);
                let mut new_deferred = false;
                for i in 0..remaining {
                    if bus.tick_ppu(1) {
                        if i + 1 == remaining {
                            new_deferred = true;
                        } else {
                            got_nmi = true;
                        }
                    }
                }
                if deferred_nmi {
                    got_nmi = true;
                }
                deferred_nmi = new_deferred;
                if got_nmi {
                    cpu.nmi();
                }

                let mut got_irq;
                if defer_irq {
                    got_irq = false;
                    let mut new_deferred_irq = false;
                    for i in 0..delta {
                        let was_high = irq_line_high;
                        let now_high = bus.tick_apu(1);
                        irq_line_high = now_high;
                        if now_high {
                            if !was_high && i + 1 == delta {
                                new_deferred_irq = true;
                            } else {
                                got_irq = true;
                            }
                        }
                    }
                    if deferred_irq {
                        got_irq = true;
                    }
                    deferred_irq = new_deferred_irq;
                } else {
                    got_irq = bus.tick_apu(delta);
                }
                if got_irq {
                    cpu.irq();
                }

                if fine_trace {
                    let (pn_after, np_after, ql_after) = cpu.debug_nmi_state();
                    let (irqp_after, inhibit_after, flagi_after) = cpu.debug_irq_state();
                    eprintln!(
                        "  pc={:#06x} delta={} cycles={} ql:{}->{} nmi:{}/{}->{}/{} irq_pending:{}->{} inhibit:{}->{} I:{}->{} got_nmi={} got_irq={}",
                        pc_now,
                        delta,
                        cpu.cycles,
                        ql_before,
                        ql_after,
                        pn_before,
                        np_before,
                        pn_after,
                        np_after,
                        irqp_before,
                        irqp_after,
                        inhibit_before,
                        inhibit_after,
                        flagi_before,
                        flagi_after,
                        got_nmi,
                        got_irq
                    );
                }
                total_cycles += delta;
            }
        }
    }

    let data = load_rom("cpu_interrupts_v2/rom_singles/3-nmi_and_irq.nes");
    run(&data, false, 0);
    eprintln!();
    run(&data, false, 1);
}

// Diagnostic: single-pass summary across all 12 rows of test 3, identifying
// WHICH mechanism preempts first within each row's $E357-$E364 window:
// IRQ preempting LDA #1/CLC/NOP directly, NMI preempting directly (Path A),
// or neither (the instruction ran clean). Finds the row where the mechanism
// changes from "IRQ preempts, NMI hijacks its vector" (rows 0-1, traced
// precisely in nmi_irq_row0_arbitration_trace) to whatever produces the
// readme's $20/$25 families.
//   cargo test nmi_irq_all_rows_summary -- --nocapture --ignored
#[test]
#[ignore]
fn nmi_irq_all_rows_summary() {
    let data = load_rom("cpu_interrupts_v2/rom_singles/3-nmi_and_irq.nes");
    let cartridge = crate::cartridge::Cartridge::from_ines(&data).unwrap();
    let mut bus = Bus::new();
    bus.insert_cartridge(cartridge);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    let _ = bus.tick_ppu(8);
    let _ = bus.tick_apu(8);

    let mut total_cycles: u64 = 0;
    let mut deferred_nmi = false;
    let mut prev_pc = 0u16;
    let mut row = -1i32;
    let mut preempt_report: Option<String> = None;

    loop {
        let sig_valid =
            bus.read(0x6001) == SIG[0] && bus.read(0x6002) == SIG[1] && bus.read(0x6003) == SIG[2];
        if sig_valid {
            let status = bus.read(0x6000);
            if status < 0x80 {
                return;
            }
        }
        if total_cycles >= MAX_CYCLES {
            panic!("timeout");
        }

        let cycles_before = cpu.cycles;
        let pc_now = cpu.pc;

        if pc_now == 0xE364 && prev_pc != 0xE364 {
            if row >= 0 {
                eprintln!(
                    "row {:2}: preempt={:<28} $1F={:#04x} $1D={:#04x}",
                    row,
                    preempt_report
                        .clone()
                        .unwrap_or_else(|| "none(clean)".to_string()),
                    bus.read(0x1F),
                    bus.read(0x1D)
                );
            }
            row += 1;
            preempt_report = None;
            if row > 11 {
                return;
            }
        }
        prev_pc = pc_now;

        if bus.dma_active() {
            bus.tick_dma();
            if bus.tick_ppu(1) {
                cpu.nmi();
            }
            if bus.tick_apu(1) {
                cpu.irq();
            }
            total_cycles += 1;
        } else {
            let (pn_before, _np_before, ql_before) = cpu.debug_nmi_state();
            let (irqp_before, _inhibit_before, _flagi_before) = cpu.debug_irq_state();
            cpu.tick(&mut bus);
            let delta = cpu.cycles - cycles_before;

            // Detect a preempt: queue-empty dispatch (ql_before==0) that jumps
            // straight to 5 (dummy-read-then-service signature) within the
            // row's test window, while it hasn't been reported yet this row.
            let (_pn_after, _np_after, ql_after) = cpu.debug_nmi_state();
            if preempt_report.is_none()
                && ql_before == 0
                && ql_after == 5
                && (0xE357..=0xE363).contains(&pc_now)
            {
                let who = if pn_before {
                    "NMI(pending_nmi already true)"
                } else if irqp_before {
                    "IRQ(irq_pending already true)"
                } else {
                    "??? (neither flag set before dispatch)"
                };
                preempt_report = Some(format!("{} at pc={:#06x}", who, pc_now));
            }

            let (extra, extra_nmi) = bus.take_ppu_preadvance();
            let mut got_nmi = extra_nmi;
            let remaining = delta.saturating_sub(extra as u64);
            let mut new_deferred = false;
            for i in 0..remaining {
                if bus.tick_ppu(1) {
                    if i + 1 == remaining {
                        new_deferred = true;
                    } else {
                        got_nmi = true;
                    }
                }
            }
            if deferred_nmi {
                got_nmi = true;
            }
            deferred_nmi = new_deferred;
            if got_nmi {
                cpu.nmi();
            }
            if bus.tick_apu(delta) {
                cpu.irq();
            }
            total_cycles += delta;
        }
    }
}
