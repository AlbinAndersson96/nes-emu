use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::cpu::Cpu;
use crate::renderer::Renderer;
use crate::system::SystemClock;
use std::fs;
use std::path::Path;

pub enum AppState {
    NoRom,
    Running {
        cpu: Cpu,
        bus: Bus,
        clock: SystemClock,
    },
}

pub struct App {
    pub state: AppState,
    pub renderer: Renderer,
}

const CYCLES_PER_FRAME: u64 = 29_781;

/// Parses `data` as an iNES ROM and, on success, replaces `*state` with a
/// freshly reset `Running` state. On failure, `*state` is left untouched —
/// callers rely on this to keep an already-running session alive when a
/// dropped file turns out to be invalid.
fn load_rom_into(state: &mut AppState, data: &[u8]) -> Result<(), String> {
    let cartridge = Cartridge::from_ines(data).map_err(|e| e.to_string())?;
    let mut bus = Bus::new();
    bus.insert_cartridge(cartridge);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    // Pre-advance PPU by 7 cycles (the real 6502 reset takes 7 cycles on
    // hardware). APU advances by 8 to match cpu.cycles starting at 8.
    let _ = bus.tick_ppu(7);
    let _ = bus.tick_apu(8);
    *state = AppState::Running {
        cpu,
        bus,
        clock: SystemClock::new(),
    };
    Ok(())
}

fn set_controller_buttons_on(state: &mut AppState, port: usize, buttons: u8) {
    if let AppState::Running { bus, .. } = state {
        bus.set_controller_state(port, buttons);
    }
}

impl App {
    pub fn new(renderer: Renderer) -> Self {
        Self {
            state: AppState::NoRom,
            renderer,
        }
    }

    pub fn load_rom(&mut self, path: &Path) -> Result<(), String> {
        let data = fs::read(path).map_err(|e| format!("cannot read '{}': {e}", path.display()))?;
        load_rom_into(&mut self.state, &data)
    }

    pub fn step_frame(&mut self) {
        match &mut self.state {
            AppState::NoRom => {
                let _ = self.renderer.present_placeholder();
            }
            AppState::Running { cpu, bus, clock } => {
                // SystemClock carries the blargg-verified interrupt-delivery
                // rules (deferred NMI edges, per-cycle APU ticking, DMA
                // interrupt deferral) — the same stepping code the ROM test
                // harness uses.
                let mut elapsed = 0u64;
                while elapsed < CYCLES_PER_FRAME {
                    elapsed += clock.step(cpu, bus).cycles;
                }
                if bus.ppu.frame_ready {
                    bus.ppu.frame_ready = false;
                    self.renderer.present(&bus.ppu.frame).unwrap();
                }
            }
        }
    }

    pub fn set_controller_buttons(&mut self, port: usize, buttons: u8) {
        set_controller_buttons_on(&mut self.state, port, buttons);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_rom_bytes() -> Vec<u8> {
        std::fs::read(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/roms/cpu/01-basics.nes"),
        )
        .expect("fixture ROM must exist")
    }

    #[test]
    fn load_rom_from_no_rom_succeeds_and_transitions() {
        let mut state = AppState::NoRom;
        let result = load_rom_into(&mut state, &test_rom_bytes());
        assert!(result.is_ok());
        assert!(matches!(state, AppState::Running { .. }));
    }

    #[test]
    fn load_rom_with_garbage_from_no_rom_fails_and_stays_no_rom() {
        let mut state = AppState::NoRom;
        let result = load_rom_into(&mut state, b"not a rom");
        assert!(result.is_err());
        assert!(matches!(state, AppState::NoRom));
    }

    #[test]
    fn load_rom_with_garbage_while_running_leaves_running_untouched() {
        let mut state = AppState::NoRom;
        load_rom_into(&mut state, &test_rom_bytes()).unwrap();
        let cycles_before = match &state {
            AppState::Running { cpu, .. } => cpu.cycles,
            AppState::NoRom => unreachable!(),
        };
        let result = load_rom_into(&mut state, b"not a rom");
        assert!(result.is_err());
        match &state {
            AppState::Running { cpu, .. } => assert_eq!(cpu.cycles, cycles_before),
            AppState::NoRom => panic!("state must remain Running after a failed swap"),
        }
    }

    #[test]
    fn load_rom_while_running_swaps_to_fresh_state() {
        let mut state = AppState::NoRom;
        load_rom_into(&mut state, &test_rom_bytes()).unwrap();
        // Advance a little so we can prove the swap resets cycles.
        if let AppState::Running { cpu, bus, .. } = &mut state {
            cpu.tick(bus);
        }
        let result = load_rom_into(&mut state, &test_rom_bytes());
        assert!(result.is_ok());
        match &state {
            // Cpu::reset() unconditionally sets cycles to 8 (src/cpu/mod.rs), so a
            // freshly-reset Cpu always lands there, never at 0.
            AppState::Running { cpu, .. } => {
                assert_eq!(cpu.cycles, 8, "swap must produce a freshly reset Cpu")
            }
            AppState::NoRom => panic!("expected Running after a successful swap"),
        }
    }

    #[test]
    fn set_controller_buttons_updates_bus_when_running() {
        use crate::cpu::Bus as CpuBus;

        let mut state = AppState::NoRom;
        load_rom_into(&mut state, &test_rom_bytes()).unwrap();
        set_controller_buttons_on(&mut state, 0, 0xFF);
        match &mut state {
            AppState::Running { bus, .. } => {
                bus.write(0x4016, 1);
                bus.write(0x4016, 0);
                assert_eq!(bus.read(0x4016), 1);
            }
            AppState::NoRom => panic!("expected Running state"),
        }
    }

    #[test]
    fn set_controller_buttons_is_noop_when_no_rom() {
        let mut state = AppState::NoRom;
        set_controller_buttons_on(&mut state, 0, 0xFF);
        assert!(matches!(state, AppState::NoRom));
    }
}
