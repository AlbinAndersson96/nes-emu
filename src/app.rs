use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::cpu::Cpu;
use crate::renderer::Renderer;
use crate::replay::{Fm2Movie, Fm2Player};
use crate::system::SystemClock;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

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
    fps: f64,
    frames_since_update: u32,
    last_fps_update: Instant,
    fps_overlay_enabled: bool,
    replay: Option<Fm2Player>,
    current_rom_bytes: Option<Vec<u8>>,
}

// Safety bound only: a real NTSC frame is 29781 CPU cycles (even) or 29780
// (odd — the pre-render skipped dot), so one frame always completes well under
// this. It guards `step_frame`'s loop against a pathological state where the
// PPU never signals `frame_ready`.
const MAX_CYCLES_PER_FRAME: u64 = 40_000;

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

/// Applies one replay frame's worth of state to `state`: a hard-reset
/// command rebuilds `state` from `rom_bytes` (mirroring a real power
/// cycle); a soft-reset command resets the Cpu in place (mirroring the
/// reset button). Either way, both controller ports are then set from the
/// frame's recorded input. Once the replay runs out of frames, `replay` is
/// cleared and control reverts to whatever else is writing the controller
/// latch (e.g. the keyboard).
fn apply_replay_frame(
    state: &mut AppState,
    replay: &mut Option<Fm2Player>,
    rom_bytes: &Option<Vec<u8>>,
) {
    if !matches!(state, AppState::Running { .. }) {
        return;
    }
    let frame = match replay {
        Some(player) => player.next_frame(),
        None => return,
    };
    let Some(frame) = frame else {
        eprintln!("replay finished");
        *replay = None;
        return;
    };
    if frame.hard_reset {
        if let Some(bytes) = rom_bytes {
            let _ = load_rom_into(state, bytes);
        }
    } else if frame.soft_reset
        && let AppState::Running { cpu, bus, .. } = state
    {
        cpu.reset(bus);
    }
    if let AppState::Running { bus, .. } = state {
        bus.set_controller_state(0, frame.controllers[0]);
        bus.set_controller_state(1, frame.controllers[1]);
    }
}

/// Folds one presented frame into the FPS running window. Returns `true`
/// when the 500ms window elapsed and `fps` was recomputed (and the counter
/// reset), `false` if the window is still accumulating. Takes `now`
/// explicitly so it can be unit tested without real wall-clock delays.
fn update_fps(
    fps: &mut f64,
    frames_since_update: &mut u32,
    last_update: &mut Instant,
    now: Instant,
) -> bool {
    *frames_since_update += 1;
    let elapsed = now.duration_since(*last_update);
    if elapsed >= Duration::from_millis(500) {
        *fps = *frames_since_update as f64 / elapsed.as_secs_f64();
        *frames_since_update = 0;
        *last_update = now;
        true
    } else {
        false
    }
}

fn toggle_flag(flag: &mut bool) {
    *flag = !*flag;
}

/// Advances the machine until the PPU completes one frame (`frame_ready`, set
/// once per PPU frame at scanline 239 dot 256), leaving it set for the caller.
/// Each call consumes the exact NTSC frame length — 29781 CPU cycles on even
/// frames, 29780 on odd (the pre-render skipped dot) — so it averages 29780.5,
/// rather than a fixed count that would drift ~0.5 cycle/frame against the PPU.
/// The `MAX_CYCLES_PER_FRAME` bound only guards against a pathological state
/// where `frame_ready` never fires. Steps via `SystemClock`, which carries the
/// blargg-verified interrupt-delivery rules (deferred NMI edges, per-cycle APU
/// ticking, DMA interrupt deferral) — the same code the ROM test harness uses.
/// Returns the CPU cycles consumed.
fn run_one_frame(cpu: &mut Cpu, bus: &mut Bus, clock: &mut SystemClock) -> u64 {
    bus.ppu.frame_ready = false;
    let mut elapsed = 0u64;
    while !bus.ppu.frame_ready && elapsed < MAX_CYCLES_PER_FRAME {
        elapsed += clock.step(cpu, bus).cycles;
    }
    elapsed
}

impl App {
    pub fn new(renderer: Renderer) -> Self {
        Self {
            state: AppState::NoRom,
            renderer,
            fps: 0.0,
            frames_since_update: 0,
            last_fps_update: Instant::now(),
            fps_overlay_enabled: false,
            replay: None,
            current_rom_bytes: None,
        }
    }

    pub fn load_rom(&mut self, path: &Path) -> Result<(), String> {
        let data = fs::read(path).map_err(|e| format!("cannot read '{}': {e}", path.display()))?;
        load_rom_into(&mut self.state, &data)?;
        self.current_rom_bytes = Some(data);
        self.replay = None;
        Ok(())
    }

    pub fn load_replay(&mut self, path: &Path) -> Result<(), String> {
        if !matches!(self.state, AppState::Running { .. }) {
            return Err("load a ROM before starting a replay".to_string());
        }
        let text = fs::read_to_string(path)
            .map_err(|e| format!("cannot read '{}': {e}", path.display()))?;
        let movie = Fm2Movie::parse(&text)?;
        self.replay = Some(Fm2Player::new(movie));
        Ok(())
    }

    pub fn step_frame(&mut self) {
        apply_replay_frame(&mut self.state, &mut self.replay, &self.current_rom_bytes);
        match &mut self.state {
            AppState::NoRom => {
                self.renderer.present_placeholder();
            }
            AppState::Running { cpu, bus, clock } => {
                run_one_frame(cpu, bus, clock);
                if bus.ppu.frame_ready {
                    bus.ppu.frame_ready = false;
                    update_fps(
                        &mut self.fps,
                        &mut self.frames_since_update,
                        &mut self.last_fps_update,
                        Instant::now(),
                    );
                    let fps = self.fps_overlay_enabled.then_some(self.fps);
                    self.renderer.present(&bus.ppu.frame, fps);
                }
            }
        }
    }

    pub fn set_controller_buttons(&mut self, port: usize, buttons: u8) {
        set_controller_buttons_on(&mut self.state, port, buttons);
    }

    pub fn toggle_fps_overlay(&mut self) {
        toggle_flag(&mut self.fps_overlay_enabled);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_rom_bytes() -> Vec<u8> {
        std::fs::read(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/roms/nes-test-roms/instr_test-v5/rom_singles/01-basics.nes"),
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

    // On-demand throughput benchmark (no assertion — debug legitimately exceeds
    // the frame budget). Reports emulation-only cost per frame so a slowdown is
    // easy to spot. Run: `cargo test bench_emulation_throughput -- --ignored
    // --nocapture` (add `--release` to measure the shipping profile). 60 fps
    // needs <= 16.639 ms/frame; see the [profile.dev] note in Cargo.toml.
    #[test]
    #[ignore]
    fn bench_emulation_throughput() {
        let mut state = AppState::NoRom;
        load_rom_into(&mut state, &test_rom_bytes()).unwrap();
        let AppState::Running { cpu, bus, clock } = &mut state else {
            unreachable!("just loaded a ROM");
        };
        for _ in 0..60 {
            run_one_frame(cpu, bus, clock);
        }
        let n = 600u32;
        let start = Instant::now();
        for _ in 0..n {
            run_one_frame(cpu, bus, clock);
        }
        let per_frame = start.elapsed() / n;
        let ms = per_frame.as_secs_f64() * 1000.0;
        eprintln!(
            "emulation only: {ms:.3} ms/frame  ({:.0} fps ceiling, need <=16.639 ms for 60fps)",
            1000.0 / ms
        );
    }

    #[test]
    fn run_one_frame_consumes_exactly_one_ntsc_frame() {
        let mut state = AppState::NoRom;
        load_rom_into(&mut state, &test_rom_bytes()).unwrap();
        let AppState::Running { cpu, bus, clock } = &mut state else {
            unreachable!("just loaded a ROM");
        };
        // Settle past power-up before measuring.
        for _ in 0..5 {
            run_one_frame(cpu, bus, clock);
        }
        let cycles = run_one_frame(cpu, bus, clock);
        assert!(bus.ppu.frame_ready, "a PPU frame must complete each call");
        // One NTSC frame is 29780 (odd) or 29781 (even) CPU cycles; allow a few
        // cycles of overshoot from stopping on an instruction boundary past the
        // frame-ready dot. A regression running a fixed or doubled count lands
        // outside this band.
        assert!(
            (29_780..=29_790).contains(&cycles),
            "frame length {cycles} is not one NTSC frame (~29780.5 cycles)"
        );
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

    #[test]
    fn fps_does_not_update_before_half_second_window_elapses() {
        let mut fps = 0.0;
        let mut frames = 0u32;
        let start = Instant::now();
        let mut last_update = start;

        for i in 0..30u64 {
            let now = start + Duration::from_millis(i * 10); // spans 0..290ms
            let updated = update_fps(&mut fps, &mut frames, &mut last_update, now);
            assert!(!updated, "must not update before 500ms elapses");
        }
        assert_eq!(fps, 0.0);
        assert_eq!(frames, 30);
    }

    #[test]
    fn fps_updates_and_resets_after_half_second_window() {
        let mut fps = 0.0;
        let mut frames = 0u32;
        let start = Instant::now();
        let mut last_update = start;

        for i in 0..30u64 {
            let now = start + Duration::from_millis(i * 10);
            update_fps(&mut fps, &mut frames, &mut last_update, now);
        }

        // 31st frame arrives at 510ms, past the 500ms window.
        let now = start + Duration::from_millis(510);
        let updated = update_fps(&mut fps, &mut frames, &mut last_update, now);

        assert!(updated, "must update once 500ms elapses");
        assert!((fps - 60.78).abs() < 1.0, "expected ~60.78 fps, got {fps}");
        assert_eq!(frames, 0, "counter must reset after computing fps");
        assert_eq!(last_update, now, "window start must reset to now");
    }

    #[test]
    fn toggle_flag_flips_bool_back_and_forth() {
        let mut enabled = false;
        toggle_flag(&mut enabled);
        assert!(enabled);
        toggle_flag(&mut enabled);
        assert!(!enabled);
    }

    #[test]
    fn apply_replay_frame_sets_both_controller_latches() {
        use crate::cpu::Bus as CpuBus;
        use crate::replay::{Fm2Movie, Fm2Player};

        let mut state = AppState::NoRom;
        load_rom_into(&mut state, &test_rom_bytes()).unwrap();
        let movie = Fm2Movie::parse("fourscore 0\n|0|.......A|......B.||\n").unwrap();
        let mut replay = Some(Fm2Player::new(movie));

        apply_replay_frame(&mut state, &mut replay, &None);

        match &mut state {
            AppState::Running { bus, .. } => {
                bus.write(0x4016, 1);
                bus.write(0x4016, 0);
                assert_eq!(bus.read(0x4016) & 1, 1, "port0 A must be pressed");
                assert_eq!(bus.read(0x4017) & 1, 0, "port1 A bit must be clear");
                assert_eq!(
                    bus.read(0x4017) & 1,
                    1,
                    "port1 B bit must be set on the 2nd read"
                );
            }
            AppState::NoRom => panic!("expected Running"),
        }
    }

    #[test]
    fn apply_replay_frame_clears_replay_when_frames_exhausted() {
        use crate::replay::{Fm2Movie, Fm2Player};

        let mut state = AppState::NoRom;
        load_rom_into(&mut state, &test_rom_bytes()).unwrap();
        let movie = Fm2Movie::parse("fourscore 0\n").unwrap();
        let mut replay = Some(Fm2Player::new(movie));

        apply_replay_frame(&mut state, &mut replay, &None);

        assert!(
            replay.is_none(),
            "replay must be cleared once frames run out"
        );
    }

    #[test]
    fn apply_replay_frame_soft_reset_resets_cpu_but_keeps_ram() {
        use crate::cpu::Bus as CpuBus;
        use crate::replay::{Fm2Movie, Fm2Player};

        let mut state = AppState::NoRom;
        load_rom_into(&mut state, &test_rom_bytes()).unwrap();
        if let AppState::Running { cpu, bus, .. } = &mut state {
            bus.write(0x0010, 0x42);
            cpu.tick(bus);
        }
        let movie = Fm2Movie::parse("fourscore 0\n|1|........|........||\n").unwrap();
        let mut replay = Some(Fm2Player::new(movie));

        apply_replay_frame(&mut state, &mut replay, &None);

        match &mut state {
            AppState::Running { cpu, bus, .. } => {
                assert_eq!(cpu.cycles, 8, "soft reset must reset the Cpu");
                assert_eq!(bus.read(0x0010), 0x42, "soft reset must not clear RAM");
            }
            AppState::NoRom => panic!("expected Running"),
        }
    }

    #[test]
    fn apply_replay_frame_hard_reset_clears_ram() {
        use crate::cpu::Bus as CpuBus;
        use crate::replay::{Fm2Movie, Fm2Player};

        let mut state = AppState::NoRom;
        let rom_bytes = test_rom_bytes();
        load_rom_into(&mut state, &rom_bytes).unwrap();
        if let AppState::Running { bus, .. } = &mut state {
            bus.write(0x0010, 0x42);
        }
        let movie = Fm2Movie::parse("fourscore 0\n|2|........|........||\n").unwrap();
        let mut replay = Some(Fm2Player::new(movie));

        apply_replay_frame(&mut state, &mut replay, &Some(rom_bytes));

        match &mut state {
            AppState::Running { cpu, bus, .. } => {
                assert_eq!(cpu.cycles, 8, "hard reset must produce a freshly reset Cpu");
                assert_eq!(bus.read(0x0010), 0, "hard reset must clear RAM");
            }
            AppState::NoRom => panic!("expected Running"),
        }
    }
}
