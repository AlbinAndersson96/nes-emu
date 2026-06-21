use std::path::Path;

use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::cpu::Bus as CpuBus;
use crate::cpu::Cpu;

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
    std::fs::read(&path)
        .unwrap_or_else(|_| panic!("ROM not found: {} — place it in tests/roms/", path.display()))
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

fn run_until_complete(bus: &mut Bus, cpu: &mut Cpu) {
    let mut total_cycles: u64 = 0;

    loop {
        let sig_valid = bus.read(0x6001) == SIG[0]
            && bus.read(0x6002) == SIG[1]
            && bus.read(0x6003) == SIG[2];

        if sig_valid {
            let status = bus.read(0x6000);
            if status < 0x80 {
                let text = read_output(bus);
                print_raw(text.trim());
                assert_eq!(status, 0, "test failed with code {:#04x}", status);
                return;
            }
        }

        if total_cycles >= MAX_CYCLES {
            let text = read_output(bus);
            print_raw(text.trim());
            panic!("timed out after {} cycles", total_cycles);
        }

        let cycles = cpu.step(bus) as u64;

        let dma_stall = bus.take_oam_dma_stall() as u64;
        let extra = if dma_stall > 0 { dma_stall + (cpu.cycles & 1) } else { 0 };
        let total = cycles + extra;

        if bus.tick_ppu(total) {
            cpu.nmi();
        }
        if bus.tick_apu(total) {
            cpu.irq();
        }

        // DMC DMA stall: the APU fetched a sample byte; tick PPU/APU by the stall
        // cycles so timing stays consistent (approximate — stall is mid-instruction
        // on real hardware, but whole-instruction emulation can't do better).
        let dmc_stall = bus.take_dmc_dma_stall() as u64;
        if dmc_stall > 0 {
            if bus.tick_ppu(dmc_stall) {
                cpu.nmi();
            }
            if bus.tick_apu(dmc_stall) {
                cpu.irq();
            }
        }

        total_cycles += total + dmc_stall;
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
    run_until_complete(&mut bus, &mut cpu);
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
rom_test!(basics,     "01-basics.nes");
rom_test!(implied,    "02-implied.nes");
rom_test!(immediate,  "03-immediate.nes");
rom_test!(zero_page,  "04-zero_page.nes");
rom_test!(zp_xy,      "05-zp_xy.nes");
rom_test!(absolute,   "06-absolute.nes");
rom_test!(abs_xy,     "07-abs_xy.nes");
rom_test!(ind_x,      "08-ind_x.nes");
rom_test!(ind_y,      "09-ind_y.nes");
rom_test!(branches,   "10-branches.nes");
rom_test!(stack,      "11-stack.nes");
rom_test!(jmp_jsr,    "12-jmp_jsr.nes");
rom_test!(rts,        "13-rts.nes");
rom_test!(rti,        "14-rti.nes");
rom_test!(brk,        "15-brk.nes");
rom_test!(special,    "16-special.nes");

// instr_test-v5 — full suite (Mapper 1 / MMC1, 256 KB PRG-ROM)
rom_test!(official_only, "official_only.nes");

// cpu_interrupts_v2 — interrupt timing and sequencing
rom_test!(cpu_interrupts_v2_cli_latency,      "cpu_interrupts_v2/rom_singles/1-cli_latency.nes");
rom_test!(cpu_interrupts_v2_nmi_and_brk,      "cpu_interrupts_v2/rom_singles/2-nmi_and_brk.nes");
rom_test!(cpu_interrupts_v2_nmi_and_irq,      "cpu_interrupts_v2/rom_singles/3-nmi_and_irq.nes");
rom_test!(cpu_interrupts_v2_irq_and_dma,      "cpu_interrupts_v2/rom_singles/4-irq_and_dma.nes");
rom_test!(cpu_interrupts_v2_branch_delays_irq, "cpu_interrupts_v2/rom_singles/5-branch_delays_irq.nes");
rom_test!(cpu_interrupts_v2_all,              "cpu_interrupts_v2/cpu_interrupts.nes");

// instr_misc — instruction behaviour edge cases
rom_test!(instr_misc_abs_x_wrap,    "instr_misc/rom_singles/01-abs_x_wrap.nes");
rom_test!(instr_misc_branch_wrap,   "instr_misc/rom_singles/02-branch_wrap.nes");
rom_test!(instr_misc_dummy_reads,   "instr_misc/rom_singles/03-dummy_reads.nes");
rom_test!(instr_misc_dummy_reads_apu, "instr_misc/rom_singles/04-dummy_reads_apu.nes");
rom_test!(instr_misc_all,           "instr_misc/instr_misc.nes");

// instr_timing — cycle-accurate instruction timing (Mapper 1 / MMC1)
rom_test!(instr_timing, "instr_timing/instr_timing.nes");
