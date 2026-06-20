mod apu;
mod bus;
mod cartridge;
mod cpu;
mod ppu;
#[cfg(test)]
mod tests;

use std::{env, fs, process};

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

    loop {
        let cycles = cpu.step(&mut bus) as u64;

        // OAM DMA stall: burn extra cycles (odd CPU cycle adds 1)
        let dma_stall = bus.take_oam_dma_stall() as u64;
        let extra = if dma_stall > 0 {
            dma_stall + (cpu.cycles & 1)
        } else {
            0
        };
        let total = cycles + extra;

        if bus.tick_ppu(total) {
            cpu.nmi();
        }
        if bus.tick_apu(total) {
            cpu.irq();
        }
    }
}
