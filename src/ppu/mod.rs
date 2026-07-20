use crate::cartridge::{Cartridge, Mirroring};

/// NES PPU — register state, VRAM, OAM, palette, VBlank/NMI, and scanline timing.
///
/// NTSC timing:
///   PPU clock:   5,369,318 Hz (3× CPU)
///   Dots/line:   341
///   Scanlines:   262 (0–239 visible, 240 post-render, 241–260 VBlank, 261 pre-render)
///   Dots/frame:  89,342 (89,341 on odd frames with rendering enabled)
const DOTS_PER_SCANLINE: u16 = 341;
const SCANLINES_PER_FRAME: u16 = 262;
const VBLANK_SCANLINE: u16 = 241;
const PRERENDER_SCANLINE: u16 = 261;

/// Dots between a sprite0_hit-affecting event (a colliding pixel, or the
/// pre-render scanline's reset) and the flag becoming visible via $2002.
/// Calibrated by bisection against blargg's `sprite_hit_tests_2005.10.05`
/// timing ROMs, run through the shared `SystemClock`: the valid window is
/// 0-2 dots, so 1 (the midpoint) — i.e. the flag is visible almost
/// immediately, there is no long internal pipeline. (An earlier calibration
/// arrived at 18-23 dots, but that measurement was taken with a stale test
/// harness that double-advanced the PPU 9 dots on every $2002 read — the
/// large "pipeline delay" was compensating for harness drift, not modeling
/// hardware. See docs/investigations/sprite_hit_timing_debug_log.md for the
/// original investigation.)
const SPRITE0_HIT_LATCH_DOTS: u8 = 1;

/// The SET path's visibility latch is one dot longer than the clear's:
/// AccuracyCoin's "$2002 Flag Timing" set-timing sub-test (bracketing the
/// hit/overflow set moment with consecutive-dot reads) needs the flags one
/// dot later than the clear-timing sub-test allows for the pre-render clear
/// — still inside blargg sprite_hit_tests' 0-2 valid window.
const SPRITE0_HIT_SET_LATCH_DOTS: u8 = 2;

/// Dots between the Bus applying a $2001 write (which its pre-advance lands
/// at the write cycle) and the mask value actually taking effect. Hardware
/// applies rendering toggles 2-5 dots after the write cycle depending on PPU
/// revision and alignment (AccuracyCoin's BG Serial In walkthrough); blargg's
/// ppu_vbl_nmi/10-even_odd_timing and the AccuracyCoin rendering-toggle tests
/// (Stale Sprite Shift Registers test 3, BG Serial In) bracket the value —
/// calibrated by sweeping: {3,4} is the passing window (2 and 5 lose BG Serial In), 3 chosen as the edge nearest hardware's common minimum.
const MASK_WRITE_DELAY_DOTS: u8 = 3;

/// PPU open-bus decay time: a decay-register bit that hasn't been refreshed
/// with a 1 decays to 0 after about 600 ms (blargg's `ppu_open_bus` readme;
/// the exact time varies with the console and temperature). 600 ms at the
/// NTSC PPU clock (5,369,318 Hz) is ~3.22 M dots.
const OPEN_BUS_DECAY_DOTS: u64 = 3_221_591;

pub struct Ppu {
    // Programmer-visible write-only registers
    ctrl: u8,     // $2000 PPUCTRL
    mask: u8,     // $2001 PPUMASK
    oam_addr: u8, // $2003 OAMADDR

    // Loopy registers (internal to the PPU, exposed via $2005/$2006/$2007)
    v: u16,     // current VRAM address (15-bit)
    t: u16,     // temporary VRAM address (15-bit)
    fine_x: u8, // fine X scroll (3-bit)
    w: bool,    // write toggle: false = first write, true = second write

    // PPUDATA read-ahead buffer (reads are delayed one access for non-palette data)
    read_buf: u8,

    // Internal memory
    oam: Box<[u8; 256]>,   // Object Attribute Memory
    vram: Box<[u8; 2048]>, // 2 KB nametable RAM
    palette: [u8; 32],     // Palette RAM

    // Status flags (make up $2002 PPUSTATUS)
    vblank: bool,
    sprite0_hit: bool,
    sprite_overflow: bool,
    // Real hardware doesn't latch the sprite0_hit flag the instant the
    // triggering event (a colliding pixel, or the pre-render scanline's
    // reset) happens — an internal pipeline delays visibility by a fixed
    // number of PPU dots, for both the set and the clear. Holds the target
    // value and remaining countdown; the flag flips when it expires. See
    // docs/investigations/sprite_hit_timing_debug_log.md.
    sprite0_hit_pending: Option<(bool, u8)>,
    /// Same visibility latch for the sprite-overflow flag's SET path (the
    /// pre-render clear stays immediate): the evaluation state machine finds
    /// the 9th in-range sprite mid-dot, but the flag reaches $2002 one dot
    /// later — AccuracyCoin's "$2002 Flag Timing" set-timing sub-test pins
    /// the order: a straddling read may see the sprite-0 hit without the
    /// overflow flag, never the reverse.
    sprite_overflow_set_pending: Option<u8>,

    // ── NMI line + CPU-side edge detector ────────────────────────────────────
    // The NMI line is a LEVEL: `vblank && (ctrl & 0x80)`. The CPU's edge
    // detector samples it once per CPU cycle (every 3rd dot, at the cycle
    // boundary); a rising edge between samples latches nmi_pending, which
    // stays latched until consumed by take_nmi() — a later $2002 read cannot
    // un-latch it, matching hardware. Because register read/write side
    // effects are applied mid-cycle (8 dots in, 1 dot before the boundary —
    // see Bus::read/write), a $2002 read or $2000 NMI-disable landing 0-1
    // PPU clocks after VBL onset drops the line before the sample ever sees
    // it high: blargg's 2-dot NMI-suppression window falls out naturally.
    nmi_pending: bool,
    /// NMI line level at the previous per-CPU-cycle sample.
    prev_nmi_line: bool,
    /// Counts processed dots mod 3; the line is sampled when it wraps. All
    /// bus paths keep whole-instruction dot totals a multiple of 3, so the
    /// wrap points stay aligned with CPU cycle boundaries.
    dot_phase: u8,
    /// Set by a $2002 read landing exactly 1 dot before VBL onset (the
    /// scanline-241-dot-1 race): the flag is never set that frame, so no NMI
    /// fires either. One-shot, consumed at the set point.
    suppress_vbl: bool,
    /// `rendering_enabled()` as sampled at the start of the previous dot's
    /// processing. The odd-frame skipped-dot decision (scanline-length
    /// selection in clock_dot, evaluated while processing pre-render dot 339)
    /// uses THIS instead of the current sample: blargg's
    /// ppu_vbl_nmi/10-even_odd_timing pins the decision to the rendering
    /// state one dot earlier — a $2001 write taking effect right before dot
    /// 339 is already too late to change whether the skip happens, in either
    /// direction (all four of its sub-tests are consistent with a single
    /// sample at the start of dot 338).
    render_prev_dot: bool,

    /// In-flight $2001 (PPUMASK) write from the Bus path: (dots remaining,
    /// value). Applied to `mask` (the rendering pipeline's tap) at the start
    /// of a dot's processing once the countdown expires — see
    /// schedule_mask_write and MASK_WRITE_DELAY_DOTS.
    pending_mask: Option<(u8, u8)>,
    /// The $2001 REGISTER value, updated at the write cycle itself (no
    /// pipeline delay). Hardware taps this signal at different depths: the
    /// rendering pipeline sees mask changes MASK_WRITE_DELAY_DOTS later
    /// (`mask`), but the odd-frame skipped-dot decision observes the
    /// register with the write-cycle timing blargg's
    /// ppu_vbl_nmi/10-even_odd_timing pinned — via `render_raw_prev_dot`.
    mask_raw: u8,
    /// `(mask_raw & 0x18) != 0` as sampled at the start of the previous
    /// dot's processing; the odd-frame skip decision's tap (see
    /// render_prev_dot for the one-dot-early derivation).
    render_raw_prev_dot: bool,

    // ── Open-bus decay register ──────────────────────────────────────────────
    // The PPU's CPU-facing data bus keeps the last value driven onto it (the
    // "decay register"). Writing any PPU register refreshes all 8 bits with
    // the written value; reads refresh only the bits the PPU itself drives
    // ($2002 bits 7-5, $2004 all, $2007 all / bits 5-0 for palette) and
    // return the decay register for the rest. Each bit decays to 0 once it
    // has gone OPEN_BUS_DECAY_DOTS without being refreshed with a 1
    // (blargg's `ppu_open_bus`). `io_bus_stamp` holds the `dots` timestamp
    // of each bit's last refresh; decay is applied lazily on read.
    io_bus: u8,
    io_bus_stamp: [u64; 8],
    /// Free-running processed-dot counter (timebase for open-bus decay).
    dots: u64,

    // Dot/scanline counters (PPU runs at 3× CPU clock)
    dot: u16,
    scanline: u16,
    odd_frame: bool,

    mirroring: Mirroring,

    // ── Background pipeline ─────────────────────────────────────────────────
    // Per-8-dot fetch latches
    bg_nt_byte: u8,   // nametable byte (tile id)
    bg_attr_byte: u8, // attribute byte (2-bit palette selector)
    bg_pat_lo: u8,    // pattern plane 0 for current tile
    bg_pat_hi: u8,    // pattern plane 1 for current tile
    // 16-bit shift registers: bit 15 is the pixel being output this dot
    bg_shift_lo: u16,
    bg_shift_hi: u16,
    bg_shift_attr_lo: u16, // attribute bit 0, replicated to 8 bits per reload
    bg_shift_attr_hi: u16, // attribute bit 1, replicated to 8 bits per reload

    // ── Sprite pipeline ──────────────────────────────────────────────────────
    // Hardware model (AccuracyCoin "Stale Sprite Shift Registers" walkthrough):
    // each of the 8 sprite units has an X down-counter with two modes.
    // "Counting": the counter decrements once per visible dot — even during
    // forced blank (rendering disabled) — and the unit switches to "halted"
    // on the dot it's checked at 0. "Halted": the unit outputs its shifter's
    // front bit (bit 7; h-flip is pre-applied at load) and the shifter clocks
    // once per dot, but ONLY while rendering is enabled — F-Blank and H-Blank
    // freeze the shifters, never the counting. All units are put back into
    // "counting" mode at dot 339 IF rendering is enabled on that dot;
    // otherwise they keep their state (usually halted), which makes stale
    // sprites draw immediately when rendering is re-enabled.
    secondary_oam: [u8; 32],    // up to 8 sprites for next scanline
    sprite_count: usize,        // sprites loaded for the scanline THIS DOT is rendering
    sprite_shift_lo: [u8; 8],   // pattern plane 0 shift registers (bit 7 = next pixel)
    sprite_shift_hi: [u8; 8],   // pattern plane 1 shift registers (bit 7 = next pixel)
    sprite_attr: [u8; 8],       // attribute bytes for active sprites
    sprite_counter: [u8; 8],    // X down-counters, loaded from sprite X at fetch
    sprite_counting: [bool; 8], // mode: true = counting, false = halted (drawing)
    sprite0_in_secondary: bool, // sprite 0 is among the sprites THIS DOT is rendering
    // Evaluation (dots 65–256) writes into these instead of the render-facing
    // fields above, which output_pixel() is still reading for dots 65–256 of
    // the SAME scanline (rendering data prepared during the PREVIOUS
    // scanline's evaluation). They're copied into the render-facing fields
    // once evaluation finishes, at the start of the sprite-fetch window
    // (dot 257) — see fetch_sprites().
    sprite_eval_count: usize,
    sprite0_eval: bool,
    // Per-dot evaluation state machine (see evaluate_sprites): one OAM check
    // every 2 dots, so the overflow flag sets at the hardware-exact dot
    // (blargg sprite_overflow_tests/3.Timing) and the buggy diagonal
    // overflow scan (4.Obscure) falls out of eval_n/eval_m.
    eval_n: usize,           // objects decided so far (0-64); addressing index once in the overflow scan
    eval_m: usize,           // byte-within-sprite offset used by the overflow scan
    eval_addr: u8,           // hardware OAMADDR-during-evaluation byte pointer, seeded from oam_addr at dot 65
    eval_copy_left: u8,      // bytes 1-3 still to copy for an in-range sprite
    eval_overflow_reads: u8, // the 3 dummy reads after the overflow flag sets
    eval_done: bool,         // n wrapped past 63 — evaluation idles until next line

    // ── Framebuffer ──────────────────────────────────────────────────────────
    // 256 × 240 pixels, each an index into the NES master palette (0x00–0x3F).
    pub frame: Box<[u8; 256 * 240]>,
    pub frame_ready: bool, // set when scanline 239 dot 256 is done; cleared by caller
}

impl Ppu {
    pub fn new() -> Self {
        Self {
            ctrl: 0,
            mask: 0,
            oam_addr: 0,
            v: 0,
            t: 0,
            fine_x: 0,
            w: false,
            read_buf: 0,
            oam: Box::new([0u8; 256]),
            vram: Box::new([0u8; 2048]),
            // 2C02 power-up palette (nesdev "PPU power up state"; the same
            // table blargg's ppu/power_up_palette ROM was recorded from).
            // The four mirrored backdrop entries (indices $10/$14/$18/$1C,
            // unreachable through palette_idx) hold the same values as
            // their $00/$04/$08/$0C targets, so a direct 32-byte init is
            // self-consistent.
            palette: [
                0x09, 0x01, 0x00, 0x01, 0x00, 0x02, 0x02, 0x0D, 0x08, 0x10, 0x08, 0x24, 0x00, 0x00,
                0x04, 0x2C, 0x09, 0x01, 0x34, 0x03, 0x00, 0x04, 0x00, 0x14, 0x08, 0x3A, 0x00, 0x02,
                0x00, 0x20, 0x2C, 0x08,
            ],
            vblank: false,
            sprite0_hit: false,
            sprite0_hit_pending: None,
            sprite_overflow_set_pending: None,
            sprite_overflow: false,
            nmi_pending: false,
            prev_nmi_line: false,
            dot_phase: 0,
            suppress_vbl: false,
            render_prev_dot: false,
            pending_mask: None,
            mask_raw: 0,
            render_raw_prev_dot: false,
            io_bus: 0,
            io_bus_stamp: [0; 8],
            dots: 0,
            dot: 0,
            scanline: 0,
            odd_frame: false,
            mirroring: Mirroring::Horizontal,
            bg_nt_byte: 0,
            bg_attr_byte: 0,
            bg_pat_lo: 0,
            bg_pat_hi: 0,
            bg_shift_lo: 0,
            bg_shift_hi: 0,
            bg_shift_attr_lo: 0,
            bg_shift_attr_hi: 0,
            secondary_oam: [0xFF; 32],
            sprite_count: 0,
            sprite_shift_lo: [0; 8],
            sprite_shift_hi: [0; 8],
            sprite_attr: [0; 8],
            sprite_counter: [0; 8],
            sprite_counting: [true; 8],
            sprite0_in_secondary: false,
            sprite_eval_count: 0,
            sprite0_eval: false,
            eval_n: 0,
            eval_m: 0,
            eval_addr: 0,
            eval_copy_left: 0,
            eval_overflow_reads: 0,
            eval_done: false,
            frame: Box::new([0u8; 256 * 240]),
            frame_ready: false,
        }
    }

    /// Set the nametable mirroring mode (called by the bus when a cartridge is
    /// inserted or when a mapper changes mirroring dynamically).
    pub fn set_mirroring(&mut self, m: Mirroring) {
        self.mirroring = m;
    }

    /// Advance PPU by `cpu_cycles` CPU cycles (= 3× PPU dots each).
    pub fn tick(&mut self, cpu_cycles: u64, cart: Option<&mut Cartridge>) {
        self.tick_dots(cpu_cycles * 3, cart);
    }

    /// Advance PPU by raw dots. Used by the bus's $2002-read pre-advance to
    /// position the sampling point within (not just at the end of) the read
    /// cycle; callers must keep whole-instruction totals a multiple of 3 so
    /// the PPU-CPU dot alignment never drifts.
    pub fn tick_dots(&mut self, dots: u64, mut cart: Option<&mut Cartridge>) {
        for _ in 0..dots {
            self.clock_dot(cart.as_deref_mut());
        }
    }

    /// Report a PPU address-bus value to the mapper (MMC3 A12 IRQ clocking).
    fn notify_ppu_bus(&self, cart: Option<&mut Cartridge>, addr: u16) {
        if let Some(c) = cart {
            c.ppu_bus_addr(addr, self.dots);
        }
    }

    /// Clock one PPU dot: run events for (scanline, dot), then advance counters.
    fn clock_dot(&mut self, mut cart: Option<&mut Cartridge>) {
        // Apply an in-flight $2001 write once its delay expires — before the
        // render sample below, so the new mask governs this dot.
        if let Some((dots_left, value)) = self.pending_mask {
            if dots_left == 0 {
                self.mask = value;
                self.pending_mask = None;
            } else {
                self.pending_mask = Some((dots_left - 1, value));
            }
        }
        let render = self.rendering_enabled();
        let visible = self.scanline <= 239;
        let is_render_scanline = visible || self.scanline == PRERENDER_SCANLINE;

        // ── VBlank / pre-render flag events ─────────────────────────────────
        match self.scanline {
            VBLANK_SCANLINE if self.dot == 1 => {
                // A $2002 read 1 dot earlier wins the race: flag never sets.
                if self.suppress_vbl {
                    self.suppress_vbl = false;
                } else {
                    self.vblank = true;
                }
                // NMI is not fired here: the line (vblank && nmi-enable) is
                // edge-sampled at the next CPU cycle boundary below.
            }
            PRERENDER_SCANLINE if self.dot == 1 => {
                self.vblank = false;
                self.sprite0_hit_pending = Some((false, SPRITE0_HIT_LATCH_DOTS));
                self.sprite_overflow = false;
                self.sprite_overflow_set_pending = None;
            }
            _ => {}
        }

        // ── Sprite-0 hit pipeline delay ──────────────────────────────────────
        // Runs unconditionally, every dot: this is an internal PPU pipeline
        // delay, not something gated on rendering being enabled.
        if let Some((target, n)) = self.sprite0_hit_pending {
            if n <= 1 {
                self.sprite0_hit = target;
                self.sprite0_hit_pending = None;
            } else {
                self.sprite0_hit_pending = Some((target, n - 1));
            }
        }
        if let Some(n) = self.sprite_overflow_set_pending {
            if n <= 1 {
                self.sprite_overflow = true;
                self.sprite_overflow_set_pending = None;
            } else {
                self.sprite_overflow_set_pending = Some(n - 1);
            }
        }

        // ── Background shift register clock (dots 1–256, 321–336) ────────────
        // Must run BEFORE pixel output below: a tile's pattern bit finishes
        // its 8-dot walk from the low byte (where reload_bg_shifters() puts
        // it) up to bit 15 (what the fine_x mux reads) on the very same dot
        // the next tile reloads — this dot's shift is what lands it there.
        // Reading the mux before shifting sees last dot's state, off by one.
        if render && is_render_scanline {
            match self.dot {
                1..=256 | 321..=336 => self.shift_bg_shifters(),
                _ => {}
            }
        }

        // ── Sprite unit clocking (visible scanlines, dots 1–256) ─────────────
        // Counters clock UNCONDITIONALLY — disabling rendering does not stop
        // them (AccuracyCoin StaleSprite test 2). A counting unit checked at
        // 0 halts and starts drawing this same dot; otherwise it decrements.
        if visible && self.dot >= 1 && self.dot <= 256 {
            for i in 0..8 {
                if self.sprite_counting[i] {
                    if self.sprite_counter[i] == 0 {
                        self.sprite_counting[i] = false;
                    } else {
                        self.sprite_counter[i] -= 1;
                    }
                }
            }
        }

        // ── Pixel output (visible scanlines, dots 1–256) ─────────────────────
        if visible && self.dot >= 1 && self.dot <= 256 {
            self.output_pixel();
        }

        // ── Sprite shifter clocking (after output: front bit was this dot's
        // pixel). Unlike the counters, the shifters clock only while
        // rendering is enabled — F-Blank and H-Blank freeze them mid-pattern
        // (StaleSprite tests 3/6), which is what lets stale sprite data draw
        // when rendering is re-enabled.
        if render && visible && self.dot >= 1 && self.dot <= 256 {
            for i in 0..8 {
                if !self.sprite_counting[i] {
                    self.sprite_shift_lo[i] <<= 1;
                    self.sprite_shift_hi[i] <<= 1;
                }
            }
        }

        // ── Sprite unit re-arm (dot 339) ─────────────────────────────────────
        // All units return to "counting" mode at dot 339, but only if the
        // PPU is rendering on that dot; during F-Blank they keep their state
        // (usually halted → stale sprites are treated as X=0 and draw as
        // soon as rendering is re-enabled — StaleSprite test 5, StaleBG
        // test 4).
        if render && is_render_scanline && self.dot == 339 {
            self.sprite_counting = [true; 8];
        }

        // ── Background tile fetch (visible + pre-render, dots 1–256, 321–336) ─
        if render && is_render_scanline {
            match self.dot {
                1..=256 | 321..=336 => self.bg_fetch(cart.as_deref_mut()),
                // Dummy nametable fetches at the end of the line (hardware
                // does two, dots 337-340). The data is unused, but the PPU
                // address bus carries the nametable address (A12 low), which
                // the MMC3's A12 low-time filter needs to see between one
                // line's last pattern fetch and the next line's first.
                337 | 339 => self.notify_ppu_bus(cart.as_deref_mut(), 0x2000 | (self.v & 0x0FFF)),
                _ => {}
            }
        }

        // ── Sprite evaluation (visible + pre-render scanlines) ───────────────
        // Pre-render evaluates for scanline 0 of the upcoming frame — skip it
        // and output_pixel()'s render-facing sprite_count/sprite0_in_secondary
        // go a full scanline stale at the top of every frame, still holding
        // whatever scanline 239 last evaluated (for the scanline that never
        // gets rendered, scanline 240) instead of scanline 0's real sprites.
        if render && is_render_scanline {
            self.evaluate_sprites();
        }

        // ── Sprite fetch (visible + pre-render scanlines, dots 257–320) ──────
        if render && is_render_scanline {
            self.fetch_sprites(cart);
        }

        // ── Frame-complete signal ─────────────────────────────────────────────
        if self.scanline == 239 && self.dot == 256 {
            self.frame_ready = true;
        }

        // ── Scroll pipeline ───────────────────────────────────────────────────
        if render && is_render_scanline {
            let coarse_x_tick = matches!(
                self.dot,
                8 | 16
                    | 24
                    | 32
                    | 40
                    | 48
                    | 56
                    | 64
                    | 72
                    | 80
                    | 88
                    | 96
                    | 104
                    | 112
                    | 120
                    | 128
                    | 136
                    | 144
                    | 152
                    | 160
                    | 168
                    | 176
                    | 184
                    | 192
                    | 200
                    | 208
                    | 216
                    | 224
                    | 232
                    | 240
                    | 248
                    | 256
                    | 328
                    | 336
            );
            if coarse_x_tick {
                self.increment_coarse_x();
            }
            if self.dot == 256 {
                self.increment_y();
            }
            if self.dot == 257 {
                self.copy_t_to_v_horizontal();
            }
            if self.scanline == PRERENDER_SCANLINE && (280..=304).contains(&self.dot) {
                self.copy_t_to_v_vertical();
            }
        }

        // ── Advance dot counter ──────────────────────────────────────────────
        self.dot += 1;
        // The odd-frame skip decision taps the RAW $2001 register (updated
        // at the write cycle), not the delayed rendering-pipeline mask —
        // blargg's 10-even_odd_timing pinned the decision to write-cycle
        // timing, while AccuracyCoin's rendering-toggle tests pin the
        // pipeline's extra MASK_WRITE_DELAY_DOTS.
        let skip_render = self.render_raw_prev_dot;
        self.render_prev_dot = render;
        self.render_raw_prev_dot = self.mask_raw & 0x18 != 0;
        let scanline_len = if self.scanline == PRERENDER_SCANLINE && self.odd_frame && skip_render {
            340
        } else {
            DOTS_PER_SCANLINE
        };
        if self.dot >= scanline_len {
            self.dot = 0;
            self.scanline += 1;
            if self.scanline >= SCANLINES_PER_FRAME {
                self.scanline = 0;
                self.odd_frame = !self.odd_frame;
            }
        }

        self.dots += 1;

        // ── CPU-side NMI edge sample (once per CPU cycle = every 3rd dot) ────
        self.dot_phase += 1;
        if self.dot_phase == 3 {
            self.dot_phase = 0;
            let line = self.nmi_line();
            if line && !self.prev_nmi_line {
                self.nmi_pending = true;
            }
            self.prev_nmi_line = line;
        }
    }

    /// Level of the PPU's /NMI output (active state as a bool).
    fn nmi_line(&self) -> bool {
        self.vblank && (self.ctrl & 0x80 != 0)
    }

    fn rendering_enabled(&self) -> bool {
        self.mask & 0x18 != 0
    }

    /// Increment coarse X in v, flipping horizontal nametable at tile 31.
    fn increment_coarse_x(&mut self) {
        if self.v & 0x001F == 31 {
            self.v &= !0x001F; // coarse X = 0
            self.v ^= 0x0400; // flip horizontal nametable
        } else {
            self.v += 1;
        }
    }

    /// Increment fine Y in v, carrying into coarse Y and wrapping at row 29.
    fn increment_y(&mut self) {
        if self.v & 0x7000 != 0x7000 {
            self.v += 0x1000; // increment fine Y
        } else {
            self.v &= !0x7000; // fine Y = 0
            let mut y = (self.v >> 5) & 0x1F;
            if y == 29 {
                y = 0;
                self.v ^= 0x0800; // flip vertical nametable
            } else if y == 31 {
                y = 0; // wrap without flipping (attribute area)
            } else {
                y += 1;
            }
            self.v = (self.v & !0x03E0) | (y << 5);
        }
    }

    /// Advance `v` after a $2007 access, per hardware. Outside rendering (or
    /// during VBlank), a $2007 read/write does the documented +1/+32
    /// `vram_increment()`. But a $2007 access during rendering (on a visible
    /// or pre-render scanline, with rendering enabled) doesn't go through the
    /// normal increment logic at all — it triggers the same coarse-X-increment
    /// + Y-increment pulse the background fetch pipeline itself uses,
    /// regardless of which dot the access lands on. AccuracyCoin's "$2007
    /// Read w/ Rendering" test pins this to exactly v += $1001 from a
    /// no-wrap starting v (+1 coarse X, +$1000 fine Y).
    fn advance_v_after_ppudata_access(&mut self) {
        let is_render_scanline = self.scanline <= 239 || self.scanline == PRERENDER_SCANLINE;
        if self.rendering_enabled() && is_render_scanline {
            self.increment_coarse_x();
            self.increment_y();
        } else {
            self.v = self.v.wrapping_add(self.vram_increment());
        }
    }

    /// Copy horizontal bits (coarse X + horizontal nametable) from t to v.
    fn copy_t_to_v_horizontal(&mut self) {
        self.v = (self.v & !0x041F) | (self.t & 0x041F);
    }

    /// Copy vertical bits (fine Y + coarse Y + vertical nametable) from t to v.
    fn copy_t_to_v_vertical(&mut self) {
        self.v = (self.v & !0x7BE0) | (self.t & 0x7BE0);
    }

    // ── Background rendering helpers ─────────────────────────────────────────

    fn bg_pattern_base(&self) -> u16 {
        if self.ctrl & 0x10 != 0 {
            0x1000
        } else {
            0x0000
        }
    }

    fn sprite_pattern_base(&self) -> u16 {
        if self.ctrl & 0x08 != 0 {
            0x1000
        } else {
            0x0000
        }
    }

    fn sprite_height(&self) -> u16 {
        if self.ctrl & 0x20 != 0 { 16 } else { 8 }
    }

    /// Reload background shift registers with the newly fetched tile data.
    fn reload_bg_shifters(&mut self) {
        self.bg_shift_lo = (self.bg_shift_lo & 0xFF00) | self.bg_pat_lo as u16;
        self.bg_shift_hi = (self.bg_shift_hi & 0xFF00) | self.bg_pat_hi as u16;
        // Replicate the 2-bit palette selector across 8 bits so we can shift it
        let attr_lo = if self.bg_attr_byte & 0x01 != 0 {
            0xFF
        } else {
            0x00
        };
        let attr_hi = if self.bg_attr_byte & 0x02 != 0 {
            0xFF
        } else {
            0x00
        };
        self.bg_shift_attr_lo = (self.bg_shift_attr_lo & 0xFF00) | attr_lo;
        self.bg_shift_attr_hi = (self.bg_shift_attr_hi & 0xFF00) | attr_hi;
    }

    /// Shift the background shift registers left by one bit.
    ///
    /// Serial input (AccuracyCoin "BG Serial In", confirmed by its own
    /// walkthrough): the LOW bit plane shifts in 0, the HIGH bit plane
    /// shifts in 1 — observable by skipping the dot%8==1 reload with a
    /// precisely timed rendering disable/re-enable, which then draws the
    /// shifted-in bits as solid color %10 pixels.
    fn shift_bg_shifters(&mut self) {
        self.bg_shift_lo <<= 1;
        self.bg_shift_hi = (self.bg_shift_hi << 1) | 1;
        self.bg_shift_attr_lo <<= 1;
        self.bg_shift_attr_hi <<= 1;
    }

    /// Pattern-table address (plane 0) for the background tile currently
    /// latched in bg_nt_byte.
    fn bg_pattern_addr(&self) -> u16 {
        let fine_y = (self.v >> 12) & 0x07;
        self.bg_pattern_base() | ((self.bg_nt_byte as u16) << 4) | fine_y
    }

    /// Run the per-8-dot background tile fetch sequence (called at dots 1..=336).
    ///
    /// Mapper A12 events: the nametable/attribute fetches report their
    /// (A12-low) addresses at the fetch dots; the tile's pattern address is
    /// reported once, on the group's LAST dot (dot%8 == 0, i.e. dots 8, 16,
    /// …, 256, 328, 336) — the group's only possible A12 rise. That dot is
    /// calibrated against blargg's mmc3_test 4-scanline_timing (see
    /// fetch_sprites; the BG-driven clock must land exactly 256 dots before
    /// the sprite-driven one, and the prefetch-driven clock 320 after it).
    fn bg_fetch(&mut self, mut cart: Option<&mut Cartridge>) {
        match self.dot % 8 {
            0 => {
                let addr = self.bg_pattern_addr();
                self.notify_ppu_bus(cart, addr);
            }
            1 => {
                // Reload shift registers with the tile fetched in the previous 8-dot window
                self.reload_bg_shifters();
                // Fetch nametable byte
                let nt_addr = 0x2000 | (self.v & 0x0FFF);
                self.notify_ppu_bus(cart.as_deref_mut(), nt_addr);
                self.bg_nt_byte = self.ppu_read(nt_addr, cart);
            }
            3 => {
                // Fetch attribute byte
                let attr_addr =
                    0x23C0 | (self.v & 0x0C00) | ((self.v >> 4) & 0x38) | ((self.v >> 2) & 0x07);
                self.notify_ppu_bus(cart.as_deref_mut(), attr_addr);
                let attr = self.ppu_read(attr_addr, cart);
                // Select the 2-bit palette for the current 2×2 tile quadrant
                let shift = ((self.v >> 4) & 0x04) | (self.v & 0x02);
                self.bg_attr_byte = (attr >> shift) & 0x03;
            }
            5 => {
                let addr = self.bg_pattern_addr();
                self.bg_pat_lo = self.ppu_read(addr, cart);
            }
            7 => {
                let addr = self.bg_pattern_addr() | 8;
                self.bg_pat_hi = self.ppu_read(addr, cart);
            }
            _ => {}
        }
    }

    // ── Sprite evaluation ─────────────────────────────────────────────────────

    /// Evaluate sprites for the NEXT scanline: a per-dot state machine over
    /// dots 65–256 of VISIBLE scanlines, one OAM check per 2 dots (hardware
    /// reads OAM on odd dots and writes secondary OAM on even dots):
    ///
    /// - An out-of-range sprite costs one step (2 dots): check Y, advance n.
    /// - An in-range sprite costs four steps (8 dots): copy its 4 bytes.
    /// - Once 8 sprites are found, the OVERFLOW SCAN begins, carrying the
    ///   hardware bug: on each out-of-range check BOTH n and m increment, so
    ///   successive sprites have successive bytes misinterpreted as their Y
    ///   (the diagonal scan blargg's 4.Obscure documents). An in-range hit
    ///   sets the overflow flag at that step's dot (3.Timing pins this) and
    ///   is followed by 3 dummy reads. The scan stops when n walks past
    ///   sprite 63 — without wrapping around (4.Obscure #7).
    ///
    /// The pre-render scanline does NOT evaluate (hardware): it only clears
    /// the state, so scanline 0 always starts with an empty sprite set — on
    /// real hardware sprites can never appear on scanline 0.
    fn evaluate_sprites(&mut self) {
        // Clear/init at dot 65 (after the secondary-OAM clearing cycles
        // 1-64). Note this must NOT touch sprite_count/sprite0_in_secondary —
        // output_pixel() is still reading those every dot through 256 to
        // render sprites found during the PREVIOUS scanline's evaluation.
        if self.dot == 65 {
            self.secondary_oam = [0xFF; 32];
            self.sprite_eval_count = 0;
            self.sprite0_eval = false;
            self.eval_n = 0;
            self.eval_m = 0;
            // Hardware seeds its OAM byte pointer from whatever OAMADDR
            // ($2003) currently holds — not always 0. A CPU write to $2003
            // between the previous scanline's forced reset (dots 257-320)
            // and this scanline's dot 65 sticks, and evaluation walks from
            // there, byte by byte, possibly starting misaligned.
            self.eval_addr = self.oam_addr;
            self.eval_copy_left = 0;
            self.eval_overflow_reads = 0;
            self.eval_done = false;
        }
        if self.scanline == PRERENDER_SCANLINE {
            return;
        }
        // One step per odd dot in 65..=255.
        if self.dot < 65 || self.dot > 255 || self.dot % 2 == 0 || self.eval_done {
            return;
        }

        // OAM Y is the sprite's top row minus 1 (hardware delays sprite
        // rendering by one scanline) — matches fetch_sprites()'s row calc,
        // which subtracts this same 1. Plain (non-wrapping) distance: a
        // sprite with Y near 255 (a common "hide it" convention) must never
        // wrap around to become visible at the top of the screen.
        let next_scanline = self.scanline + 1;
        let height = self.sprite_height() as i32;
        let in_range = |y: u8| (0..height).contains(&(next_scanline as i32 - y as i32 - 1));

        if self.sprite_eval_count < 8 {
            if self.eval_copy_left > 0 {
                // Copying bytes 1-3 of an in-range sprite, one per step, by
                // walking eval_addr forward — not always eval_n*4+byte, since
                // a misaligned start makes these bytes span what would
                // otherwise be two different OAM objects.
                let byte = 4 - self.eval_copy_left as usize;
                self.secondary_oam[self.sprite_eval_count * 4 + byte] =
                    self.oam[self.eval_addr as usize];
                self.eval_addr = self.eval_addr.wrapping_add(1);
                self.eval_copy_left -= 1;
                if self.eval_copy_left == 0 {
                    self.sprite_eval_count += 1;
                    self.eval_n += 1;
                    self.eval_done = self.eval_n == 64;
                    if self.sprite_eval_count == 8 {
                        // Hand off to the overflow scan below, which
                        // addresses via eval_n/eval_m — reconcile them from
                        // the byte pointer we've actually been walking.
                        self.eval_n = (self.eval_addr / 4) as usize;
                        self.eval_m = (self.eval_addr % 4) as usize;
                    }
                }
            } else {
                let y = self.oam[self.eval_addr as usize];
                if in_range(y) {
                    self.secondary_oam[self.sprite_eval_count * 4] = y;
                    // "Sprite zero" isn't literally OAM index 0 — it's
                    // whichever object evaluation examines FIRST this
                    // scanline (eval_n, our decision counter, is still 0
                    // only on that first decision). If that first object is
                    // out of range instead, no sprite zero exists this
                    // scanline at all, regardless of what a later object
                    // lands on in secondary OAM slot 0.
                    if self.eval_n == 0 {
                        self.sprite0_eval = true;
                    }
                    self.eval_addr = self.eval_addr.wrapping_add(1);
                    self.eval_copy_left = 3;
                } else {
                    self.eval_addr = self.eval_addr.wrapping_add(4) & 0xFC;
                    self.eval_n += 1;
                    self.eval_done = self.eval_n == 64;
                }
            }
        } else if self.eval_overflow_reads > 0 {
            // Dummy reads after the flag set; m increments with carry into n.
            self.eval_overflow_reads -= 1;
            self.eval_m += 1;
            if self.eval_m == 4 {
                self.eval_m = 0;
                self.eval_n += 1;
            }
            if self.eval_overflow_reads == 0 || self.eval_n >= 64 {
                self.eval_done = true;
            }
        } else {
            // Buggy overflow scan: OAM[n][m] is treated as a Y coordinate.
            let y = self.oam[self.eval_n * 4 + self.eval_m];
            if in_range(y) {
                self.sprite_overflow_set_pending = Some(2);
                self.eval_overflow_reads = 3;
            } else {
                self.eval_n += 1;
                self.eval_m = (self.eval_m + 1) & 3;
                self.eval_done = self.eval_n == 64;
            }
        }
    }

    /// Pattern-table address (plane 0) for sprite fetch slot `idx`, computed
    /// from secondary OAM. Empty slots read as $FF-filled (evaluation clears
    /// secondary OAM to $FF), reproducing the hardware's dummy tile-$FF
    /// fetches — whose bus address (A12 in particular) the MMC3 IRQ counter
    /// observes even when fewer than 8 sprites were found.
    fn sprite_pattern_addr(&self, idx: usize) -> u16 {
        let y_pos = self.secondary_oam[idx * 4] as u16;
        let tile = self.secondary_oam[idx * 4 + 1];
        let attr = self.secondary_oam[idx * 4 + 2];
        let flip_v = attr & 0x80 != 0;
        let height = self.sprite_height();

        let mut row = (self.scanline + 1).saturating_sub(y_pos + 1);
        if flip_v {
            row = (height - 1).saturating_sub(row);
        }

        let (pt_base, tile_idx) = if height == 16 {
            let pt = if tile & 0x01 != 0 {
                0x1000u16
            } else {
                0x0000u16
            };
            let t = (tile & 0xFE) as u16 + if row >= 8 { 1 } else { 0 };
            (pt, t)
        } else {
            (self.sprite_pattern_base(), tile as u16)
        };

        pt_base | (tile_idx << 4) | (row & 0x07)
    }

    /// Fetch sprite patterns for the sprites found in secondary OAM (dots 257–320).
    ///
    /// Each 8-dot slot reports a mapper-visible bus sequence — garbage
    /// nametable/attribute addresses (offsets 0/2, A12 low) and the slot's
    /// pattern address (offset 7, the slot's only possible A12 rise) — for
    /// ALL 8 slots, occupied or not, so the mapper sees a hardware-like A12
    /// waveform even on empty scanlines (dummy tile-$FF fetches). The
    /// pattern-notify dot (260+... in hardware terms; slot offset 7 = dot
    /// 264+8k here) and the matching BG dots in bg_fetch were calibrated as
    /// a set against blargg's mmc3_test 4-scanline_timing, which brackets
    /// the IRQ moment to 1 PPU dot in both $2000=$08 and $2000=$10 modes:
    /// sprite-driven clock = BG-driven clock + 256 dots, prefetch-driven
    /// clock = BG + 320, all three anchored to the $2002-read sampling
    /// point this emulator's VBL timing is calibrated to. The fetched data
    /// is captured into the shift registers on the same dot, for occupied
    /// slots only.
    fn fetch_sprites(&mut self, mut cart: Option<&mut Cartridge>) {
        if self.dot < 257 || self.dot > 320 {
            return;
        }
        // Evaluation for this scanline finished at dot 256; hand its results
        // off to the render-facing fields now, before output_pixel() needs
        // them on the very next scanline's dot 1.
        if self.dot == 257 {
            self.sprite_count = self.sprite_eval_count;
            self.sprite0_in_secondary = self.sprite0_eval;
            // Hardware quirk: OAMADDR is forced to 0 during ticks 257-320 of
            // every visible/pre-render scanline while rendering. A CPU write
            // to $2003 that sets OAMADDR nonzero and is never followed by
            // another $2003 write before the next rendered frame's sprite
            // fetch will observe OAMADDR back at 0 — most importantly, a
            // later $4014 OAM DMA then starts copying at OAM byte 0 instead
            // of wrapping mid-array.
            self.oam_addr = 0;
        }
        let idx = ((self.dot - 257) / 8) as usize;
        match (self.dot - 257) % 8 {
            0 | 2 => {
                self.notify_ppu_bus(cart, 0x2000 | (self.v & 0x0FFF));
            }
            7 => {
                let addr = self.sprite_pattern_addr(idx);
                self.notify_ppu_bus(cart.as_deref_mut(), addr);
                if idx >= self.sprite_count {
                    return;
                }
                let attr = self.secondary_oam[idx * 4 + 2];
                let x_pos = self.secondary_oam[idx * 4 + 3];
                let addr_lo = self.sprite_pattern_addr(idx);
                let lo = self.ppu_read(addr_lo, cart.as_deref_mut());
                let hi = self.ppu_read(addr_lo | 8, cart);

                // Horizontal flip
                let (lo, hi) = if attr & 0x40 != 0 {
                    (lo.reverse_bits(), hi.reverse_bits())
                } else {
                    (lo, hi)
                };

                self.sprite_shift_lo[idx] = lo;
                self.sprite_shift_hi[idx] = hi;
                self.sprite_attr[idx] = attr;
                // The unit's mode is NOT touched here — units go back to
                // "counting" only at dot 339, and only if rendering is
                // enabled on that dot.
                self.sprite_counter[idx] = x_pos;
            }
            _ => {}
        }
    }

    // ── Pixel output ──────────────────────────────────────────────────────────

    /// Output one pixel to the framebuffer and check sprite-0 hit.
    fn output_pixel(&mut self) {
        let x = (self.dot as usize).wrapping_sub(1); // dot 1 = column 0
        let y = self.scanline as usize;
        if x >= 256 || y >= 240 {
            return;
        }

        let bg_enabled = self.mask & 0x08 != 0;
        let sp_enabled = self.mask & 0x10 != 0;
        let bg_left_clip = self.mask & 0x02 == 0 && x < 8;
        let sp_left_clip = self.mask & 0x04 == 0 && x < 8;

        // Background pixel
        let (bg_pal, bg_col) = if bg_enabled && !bg_left_clip {
            let mux = 0x8000 >> self.fine_x;
            let lo = ((self.bg_shift_lo & mux) != 0) as u8;
            let hi = ((self.bg_shift_hi & mux) != 0) as u8;
            let pal_lo = ((self.bg_shift_attr_lo & mux) != 0) as u8;
            let pal_hi = ((self.bg_shift_attr_hi & mux) != 0) as u8;
            ((pal_hi << 1) | pal_lo, (hi << 1) | lo)
        } else {
            (0, 0)
        };

        // Sprite pixel (first non-transparent HALTED unit wins). A unit
        // contributes only while halted (its down-counter reached 0); the
        // pixel is its shifter's front bit. The shifters themselves clock in
        // clock_dot after this returns, so a rendering disable freezes each
        // unit mid-pattern and it resumes from the same bit on re-enable.
        let (sp_pal, sp_col, sp_priority, sp_is_zero) = if sp_enabled && !sp_left_clip {
            let mut result = (0u8, 0u8, false, false);
            for i in 0..self.sprite_count {
                if self.sprite_counting[i] {
                    continue; // still counting down to its X position
                }
                let lo = (self.sprite_shift_lo[i] >> 7) & 1;
                let hi = (self.sprite_shift_hi[i] >> 7) & 1;
                let col = (hi << 1) | lo;
                if col == 0 {
                    continue; // transparent
                }
                let pal = (self.sprite_attr[i] & 0x03) + 4;
                let priority = self.sprite_attr[i] & 0x20 != 0;
                result = (pal, col, priority, i == 0 && self.sprite0_in_secondary);
                break;
            }
            result
        } else {
            (0, 0, false, false)
        };

        // Sprite-0 hit: schedule it, don't latch immediately — see
        // sprite0_hit_pending's doc comment for why.
        if sp_is_zero
            && bg_col != 0
            && sp_col != 0
            && x != 255
            && !self.sprite0_hit
            && self.sprite0_hit_pending.is_none()
        {
            self.sprite0_hit_pending = Some((true, SPRITE0_HIT_SET_LATCH_DOTS));
        }

        // Priority multiplexer
        let (palette, color_idx) = match (bg_col, sp_col) {
            (0, 0) => (0u8, 0u8),       // both transparent → backdrop
            (0, _) => (sp_pal, sp_col), // only sprite visible
            (_, 0) => (bg_pal, bg_col), // only background visible
            _ => {
                if sp_priority {
                    (bg_pal, bg_col)
                } else {
                    (sp_pal, sp_col)
                }
            }
        };

        // Look up palette RAM: backdrop color at $3F00 if color_idx == 0
        let palette_addr = if color_idx == 0 {
            0x3F00u16
        } else {
            0x3F00 | ((palette as u16) << 2) | color_idx as u16
        };
        let nes_color = self.palette[Self::palette_idx(palette_addr)];
        let grey = self.mask & 0x01 != 0;
        self.frame[y * 256 + x] = if grey {
            nes_color & 0x30
        } else {
            nes_color & 0x3F
        };
    }

    /// Returns true (and clears the latch) if an NMI is pending.
    /// Read a raw byte from nametable 0 by tile coordinates (row 0–29, col 0–31).
    pub(crate) fn nt0_tile(&self, row: usize, col: usize) -> u8 {
        self.vram[row * 32 + col]
    }

    /// Current OAMADDR ($2003), for tests.
    pub(crate) fn oam_addr(&self) -> u8 {
        self.oam_addr
    }

    /// Background pattern shift registers, for tests (lo plane, hi plane).
    pub(crate) fn bg_shifters(&self) -> (u16, u16) {
        (self.bg_shift_lo, self.bg_shift_hi)
    }

    /// Recompute bits 6-5 (sprite-0 hit, sprite overflow) of a just-read
    /// $2002 value from the CURRENT flag state, refreshing those open-bus
    /// bits. The Bus calls this one dot after the register read proper:
    /// hardware samples the sprite flags at the end of the read cycle,
    /// ~2 dots after the VBL flag's start-of-cycle latch (AccuracyCoin
    /// "$2002 Flag Timing").
    pub fn resample_sprite_flags(&mut self, value: u8) -> u8 {
        let mut v = value & !0x60;
        if self.sprite0_hit {
            v |= 0x40;
        }
        if self.sprite_overflow {
            v |= 0x20;
        }
        self.refresh_open_bus(v, 0x60);
        v
    }

    /// Current VRAM address (the Loopy `v` register), for tests.
    pub(crate) fn v(&self) -> u16 {
        self.v
    }

    /// A byte of the secondary-OAM buffer sprite evaluation just built, for tests.
    pub(crate) fn secondary_oam_byte(&self, i: usize) -> u8 {
        self.secondary_oam[i]
    }

    /// Whether evaluation flagged secondary-OAM slot 0 as "sprite zero" this
    /// scanline, for tests.
    pub(crate) fn sprite0_eval(&self) -> bool {
        self.sprite0_eval
    }

    /// Read a raw OAM byte by index (0-255), for tests.
    pub(crate) fn oam_byte(&self, i: usize) -> u8 {
        self.oam[i]
    }

    pub fn take_nmi(&mut self) -> bool {
        let v = self.nmi_pending;
        self.nmi_pending = false;
        v
    }

    fn vram_increment(&self) -> u16 {
        if self.ctrl & 0x04 != 0 { 32 } else { 1 }
    }

    fn mirror_vram_addr(&self, addr: u16) -> usize {
        let a = (addr & 0x0FFF) as usize; // strip high bits → 0x000–0xFFF
        let nt = (a >> 10) & 0x3; // nametable index 0–3
        let off = a & 0x3FF; // byte offset within nametable
        let bank: usize = match self.mirroring {
            Mirroring::Horizontal => [0, 0, 1, 1][nt],
            Mirroring::Vertical => [0, 1, 0, 1][nt],
            Mirroring::SingleLow => 0,
            Mirroring::SingleHigh => 1,
            Mirroring::FourScreen => nt & 1, // only 2 KB on-chip; treat as vertical
        };
        bank * 0x400 + off
    }

    fn palette_idx(addr: u16) -> usize {
        let mut idx = (addr & 0x001F) as usize;
        // Backdrop mirrors: $3F10/$3F14/$3F18/$3F1C → $3F00/$3F04/$3F08/$3F0C
        if idx >= 0x10 && idx % 4 == 0 {
            idx -= 0x10;
        }
        idx
    }

    // Data access only — callers report bus addresses to the mapper via
    // notify_ppu_bus separately, because the mapper-visible A12 timing (the
    // dot at which an address appears on the bus) is calibrated against
    // blargg's mmc3_test 4-scanline_timing and does not always coincide with
    // the dot at which this emulator finds it convenient to fetch the data.
    fn ppu_read(&self, addr: u16, cart: Option<&mut Cartridge>) -> u8 {
        let addr = addr & 0x3FFF;
        match addr {
            0x0000..=0x1FFF => cart.map_or(0, |c| c.chr_read(addr)),
            0x2000..=0x3EFF => self.vram[self.mirror_vram_addr(addr)],
            _ => self.palette[Self::palette_idx(addr)],
        }
    }

    fn ppu_write(&mut self, addr: u16, data: u8, cart: Option<&mut Cartridge>) {
        let addr = addr & 0x3FFF;
        match addr {
            0x0000..=0x1FFF => {
                if let Some(c) = cart {
                    c.chr_write(addr, data);
                }
            }
            0x2000..=0x3EFF => {
                let idx = self.mirror_vram_addr(addr);
                self.vram[idx] = data;
            }
            _ => self.palette[Self::palette_idx(addr)] = data,
        }
    }

    /// Current value of the open-bus decay register, with expired bits
    /// (not refreshed with a 1 within OPEN_BUS_DECAY_DOTS) decayed to 0.
    fn open_bus(&mut self) -> u8 {
        for bit in 0..8 {
            if self.io_bus & (1 << bit) != 0
                && self.dots.saturating_sub(self.io_bus_stamp[bit]) > OPEN_BUS_DECAY_DOTS
            {
                self.io_bus &= !(1 << bit);
            }
        }
        self.io_bus
    }

    /// Refresh the decay-register bits selected by `mask` with `value`,
    /// restarting their decay timers. Unmasked bits keep their value and
    /// their old timers.
    fn refresh_open_bus(&mut self, value: u8, mask: u8) {
        self.io_bus = (self.io_bus & !mask) | (value & mask);
        for bit in 0..8 {
            if mask & (1 << bit) != 0 {
                self.io_bus_stamp[bit] = self.dots;
            }
        }
    }

    /// Read a PPU register. `reg` is the 3-bit register select (addr & 7).
    /// Open-bus behavior follows blargg's `ppu_open_bus` readme: write-only
    /// registers return the decay register unrefreshed; $2002/$2004/$2007
    /// return PPU-driven bits (which refresh their decay-register bits)
    /// combined with decay-register bits for the rest.
    pub fn read_register(&mut self, reg: u8, mut cart: Option<&mut Cartridge>) -> u8 {
        match reg {
            // $2002 PPUSTATUS — reading clears VBlank flag and write toggle.
            // Bits 7-5 are PPU-driven (and refresh those decay bits); bits
            // 4-0 read from the decay register.
            2 => {
                let status = ((self.vblank as u8) << 7)
                    | ((self.sprite0_hit as u8) << 6)
                    | ((self.sprite_overflow as u8) << 5)
                    | (self.open_bus() & 0x1F);
                self.refresh_open_bus(status, 0xE0);
                self.vblank = false;
                // Reading exactly 1 dot before VBL onset ((241,1) is the next
                // dot to process) wins the race: the flag never sets this
                // frame, and consequently no NMI fires. Reads at the set dot
                // or 1 after suppress the NMI via the ordinary line-level
                // mechanism (the clear above drops the line before the next
                // per-cycle edge sample). An edge already latched into
                // nmi_pending is NOT cleared — the CPU's edge detector cannot
                // be un-latched by a $2002 read.
                if self.scanline == VBLANK_SCANLINE && self.dot == 1 {
                    self.suppress_vbl = true;
                }
                self.w = false;
                status
            }
            // $2004 OAMDATA — fully PPU-driven; refreshes all decay bits.
            // Attribute bytes (every 4th, offset 2) have no storage for bits
            // 2-4, so they always read back clear. Dots 1-64 of every
            // visible/pre-render scanline are spent clearing secondary OAM to
            // $FF while rendering — a read during that window observes that
            // internal bus activity instead of the real byte at OAMADDR.
            4 => {
                let clearing_secondary_oam = self.rendering_enabled()
                    && (self.scanline <= 239 || self.scanline == PRERENDER_SCANLINE)
                    && (1..=64).contains(&self.dot);
                let mut value = if clearing_secondary_oam {
                    0xFF
                } else {
                    self.oam[self.oam_addr as usize]
                };
                if !clearing_secondary_oam && self.oam_addr & 3 == 2 {
                    value &= 0xE3;
                }
                self.refresh_open_bus(value, 0xFF);
                value
            }
            // $2007 PPUDATA — non-palette reads are buffered one cycle and
            // refresh all decay bits; palette reads drive only bits 5-0,
            // with bits 7-6 coming from the decay register unrefreshed.
            7 => {
                let addr = self.v;
                self.advance_v_after_ppudata_access();
                // The access itself puts v on the PPU bus (an A12 rise the
                // MMC3 observes), then the bus follows the incremented v.
                self.notify_ppu_bus(cart.as_deref_mut(), addr & 0x3FFF);
                let value = if addr & 0x3FFF >= 0x3F00 {
                    // Palette: return immediately; refresh buffer from nametable behind palette.
                    // Greyscale mode (PPUMASK bit 0) zeroes the low 4 bits of
                    // palette READS — storage is unaffected, writes land
                    // unmasked (AccuracyCoin "Palette RAM Quirks" codes 6-7).
                    self.read_buf = self.ppu_read(addr & 0x2FFF, cart.as_deref_mut());
                    let palette_mask = if self.mask & 0x01 != 0 { 0x30 } else { 0x3F };
                    let value = (self.open_bus() & 0xC0)
                        | (self.ppu_read(addr, cart.as_deref_mut()) & palette_mask);
                    self.refresh_open_bus(value, 0x3F);
                    value
                } else {
                    let buffered = self.read_buf;
                    self.read_buf = self.ppu_read(addr, cart.as_deref_mut());
                    self.refresh_open_bus(buffered, 0xFF);
                    buffered
                };
                // After the access the PPU's address bus follows the
                // incremented v — an A12 rise the MMC3 counter observes
                // (blargg 3-A12_clocking #5: a $2007 read at $0FFF clocks
                // the counter via the increment to $1000).
                self.notify_ppu_bus(cart, self.v & 0x3FFF);
                value
            }
            // Write-only registers ($2000/$2001/$2003/$2005/$2006): reading
            // returns the decay register and refreshes nothing.
            _ => self.open_bus(),
        }
    }

    /// Schedule a $2001 (PPUMASK) write to take effect MASK_WRITE_DELAY_DOTS
    /// dots from now (counted down in clock_dot, applied at the start of a
    /// dot's processing). Hardware applies rendering toggles a few dots
    /// after the write cycle; the Bus's $2001 path calls this instead of
    /// write_register so the delay rides on top of its pre-advance (which
    /// lands at the write cycle). The open-bus refresh is NOT deferred — the
    /// CPU-PPU bus transfer happens at the write itself. Direct
    /// write_register(1, ..) calls (unit tests) keep immediate semantics.
    pub fn schedule_mask_write(&mut self, data: u8) {
        self.refresh_open_bus(data, 0xFF);
        // The register itself updates now (write-cycle timing) — only the
        // rendering pipeline's tap (`mask`) is deferred.
        self.mask_raw = data;
        self.pending_mask = Some((MASK_WRITE_DELAY_DOTS, data));
    }

    /// Write a PPU register. `reg` is the 3-bit register select (addr & 7).
    /// Every write drives all 8 bits of the CPU-PPU bus, refreshing the
    /// whole open-bus decay register with the written value.
    pub fn write_register(&mut self, reg: u8, data: u8, mut cart: Option<&mut Cartridge>) {
        self.refresh_open_bus(data, 0xFF);
        match reg {
            // $2000 PPUCTRL
            0 => {
                // NMI enable/disable needs no special handling here: the line
                // level (vblank && enable) changes immediately, and the next
                // per-cycle edge sample picks it up — enabling mid-VBlank
                // fires the "instant NMI" quirk, disabling within a CPU cycle
                // of VBL onset suppresses the NMI, both as on hardware.
                self.ctrl = data;
                // Nametable select → t bits 10–11
                self.t = (self.t & 0xF3FF) | ((data as u16 & 0x03) << 10);
            }
            // $2001 PPUMASK (immediate path — Bus writes go through
            // schedule_mask_write instead, which defers the pipeline tap)
            1 => {
                self.mask = data;
                self.mask_raw = data;
            }
            // $2003 OAMADDR
            3 => self.oam_addr = data,
            // $2004 OAMDATA
            4 => {
                self.oam[self.oam_addr as usize] = data;
                self.oam_addr = self.oam_addr.wrapping_add(1);
            }
            // $2005 PPUSCROLL — two writes
            5 => {
                if !self.w {
                    self.t = (self.t & 0xFFE0) | ((data as u16) >> 3);
                    self.fine_x = data & 0x07;
                } else {
                    self.t = (self.t & 0x8C1F)
                        | ((data as u16 & 0x07) << 12)
                        | ((data as u16 & 0xF8) << 2);
                }
                self.w = !self.w;
            }
            // $2006 PPUADDR — two writes
            6 => {
                if !self.w {
                    self.t = (self.t & 0x00FF) | ((data as u16 & 0x3F) << 8);
                } else {
                    self.t = (self.t & 0xFF00) | data as u16;
                    self.v = self.t;
                    // The PPU address bus follows v — the MMC3 IRQ counter
                    // clocks on A12 rises produced by $2006 writes (blargg
                    // 3-A12_clocking #4).
                    self.notify_ppu_bus(cart, self.v & 0x3FFF);
                }
                self.w = !self.w;
            }
            // $2007 PPUDATA
            7 => {
                let addr = self.v;
                self.notify_ppu_bus(cart.as_deref_mut(), addr & 0x3FFF);
                self.ppu_write(addr, data, cart.as_deref_mut());
                self.advance_v_after_ppudata_access();
                // Post-increment bus value, as for $2007 reads.
                self.notify_ppu_bus(cart, self.v & 0x3FFF);
            }
            _ => {}
        }
    }

    /// Write a single byte to OAM at offset `offset` (used by OAM DMA via $4014).
    /// DMA starts at the current OAMADDR ($2003) and wraps mod 256.
    /// Hardware performs these as $2004 writes, so they refresh the open bus.
    pub fn oam_dma_write(&mut self, offset: u8, data: u8) {
        self.refresh_open_bus(data, 0xFF);
        self.oam[self.oam_addr.wrapping_add(offset) as usize] = data;
    }
}
