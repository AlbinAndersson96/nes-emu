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
        if bus.tick_ppu(cycles) {
            cpu.nmi();
        }
    }
}
