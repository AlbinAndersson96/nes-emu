use std::path::Path;

use crate::bus::Bus;
use crate::cartridge::Cartridge;
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
        let b = bus.peek(addr);
        if b == 0 {
            break;
        }
        out.push(b as char);
        addr = addr.wrapping_add(1);
    }
    out
}

fn run_until_complete(filename: &str, bus: &mut Bus, cpu: &mut Cpu) {
    let mut total_cycles: u64 = 0;
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
            bus.peek(0x6001) == SIG[0] && bus.peek(0x6002) == SIG[1] && bus.peek(0x6003) == SIG[2];

        if sig_valid {
            let status = bus.peek(0x6000);
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
                if total_cycles.saturating_sub(since) >= RESET_DELAY_CYCLES
                    && cpu.instruction_boundary()
                {
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

        total_cycles += clock.step(cpu, bus).cycles;
    }
}

fn run_rom(filename: &str) {
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
    run_until_complete(filename, &mut bus, &mut cpu);
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

// ppu_read_buffer — $2007 read-buffer behavior (mapper 3 / CNROM, 32 KB
// banked CHR-ROM; also exercises CHR bank switching)
rom_test!(ppu_read_buffer, "ppu_read_buffer/test_ppu_read_buffer.nes");

// mmc3_test — MMC3 scanline counter / IRQ (mapper 4). 6-MMC6.nes is NOT
// wired: it tests MMC6 (and rev-A-like MMC3 chips), whose IRQ-on-forced-
// reload behavior is mutually exclusive with the normal MMC3 behavior that
// 5-MMC3.nes verifies (see the suite's readme).
rom_test!(mmc3_test_clocking, "mmc3_test/1-clocking.nes");
rom_test!(mmc3_test_details, "mmc3_test/2-details.nes");
rom_test!(mmc3_test_a12_clocking, "mmc3_test/3-A12_clocking.nes");
rom_test!(mmc3_test_scanline_timing, "mmc3_test/4-scanline_timing.nes");
rom_test!(mmc3_test_mmc3, "mmc3_test/5-MMC3.nes");

// mmc3_test_2 — later revision of the same suite. 6-MMC3_alt.nes is NOT
// wired for the same reason as 6-MMC6 above (alternate/rev-A behavior,
// mutually exclusive with 5-MMC3).
rom_test!(
    mmc3_test_2_clocking,
    "mmc3_test_2/rom_singles/1-clocking.nes"
);
rom_test!(mmc3_test_2_details, "mmc3_test_2/rom_singles/2-details.nes");
rom_test!(
    mmc3_test_2_a12_clocking,
    "mmc3_test_2/rom_singles/3-A12_clocking.nes"
);
rom_test!(
    mmc3_test_2_scanline_timing,
    "mmc3_test_2/rom_singles/4-scanline_timing.nes"
);
rom_test!(mmc3_test_2_mmc3, "mmc3_test_2/rom_singles/5-MMC3.nes");
