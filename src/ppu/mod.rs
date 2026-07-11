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
/// Real hardware doesn't latch the flag the instant the event happens — an
/// internal pipeline delays it. Calibrated by bisection against
/// blargg's `sprite_hit_tests_2005.10.05` timing ROMs (valid window measured
/// at 18-23 dots for the set side; 21 also satisfies the pre-render clear).
/// See docs/investigations/sprite_hit_timing_debug_log.md.
const SPRITE0_HIT_LATCH_DOTS: u8 = 21;

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
    secondary_oam: [u8; 32],    // up to 8 sprites for next scanline
    sprite_count: usize,        // sprites loaded for the scanline THIS DOT is rendering
    sprite_shift_lo: [u8; 8],   // pattern plane 0 shift registers
    sprite_shift_hi: [u8; 8],   // pattern plane 1 shift registers
    sprite_attr: [u8; 8],       // attribute bytes for active sprites
    sprite_x: [u8; 8],          // X counters for active sprites
    sprite0_in_secondary: bool, // sprite 0 is among the sprites THIS DOT is rendering
    // Evaluation (dots 65–256) writes into these instead of the render-facing
    // fields above, which output_pixel() is still reading for dots 65–256 of
    // the SAME scanline (rendering data prepared during the PREVIOUS
    // scanline's evaluation). They're copied into the render-facing fields
    // once evaluation finishes, at the start of the sprite-fetch window
    // (dot 257) — see fetch_sprites().
    sprite_eval_count: usize,
    sprite0_eval: bool,

    // ── Framebuffer ──────────────────────────────────────────────────────────
    // 256 × 240 pixels, each an index into the NES master palette (0x00–0x3F).
    pub frame: Box<[u8; 256 * 240]>,
    pub frame_ready: bool, // set when scanline 239 dot 256 is done; cleared by caller
}

impl Ppu {
    /// Diagnostic accessor for cycle-exact interrupt-timing tracers: set the
    /// exact (dot, scanline) position directly, bypassing tick(), for
    /// dot-level precision that `tick_ppu`'s CPU-cycle (3-dot) granularity
    /// can't reach.
    #[cfg(test)]
    pub(crate) fn debug_set_position(&mut self, dot: u16, scanline: u16) {
        self.dot = dot;
        self.scanline = scanline;
    }

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
            palette: [0u8; 32],
            vblank: false,
            sprite0_hit: false,
            sprite0_hit_pending: None,
            sprite_overflow: false,
            nmi_pending: false,
            prev_nmi_line: false,
            dot_phase: 0,
            suppress_vbl: false,
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
            sprite_x: [0; 8],
            sprite0_in_secondary: false,
            sprite_eval_count: 0,
            sprite0_eval: false,
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
    pub fn tick_dots(&mut self, dots: u64, cart: Option<&mut Cartridge>) {
        // Split borrow: we need &mut self and &Cartridge simultaneously.
        // Safety: we pass cart as Option<&Cartridge> (shared ref) to helpers
        // that only read it; ppu_write can take &mut Cartridge but that's CHR-RAM
        // which happens only via register writes, not during tick.
        let cart_ref: Option<&Cartridge> = cart.map(|c| &*c);
        for _ in 0..dots {
            self.clock_dot(cart_ref);
        }
    }

    /// Clock one PPU dot: run events for (scanline, dot), then advance counters.
    fn clock_dot(&mut self, cart: Option<&Cartridge>) {
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

        // ── Pixel output (visible scanlines, dots 1–256) ─────────────────────
        if visible && self.dot >= 1 && self.dot <= 256 {
            self.output_pixel();
        }

        // ── Background tile fetch (visible + pre-render, dots 1–256, 321–336) ─
        if render && is_render_scanline {
            match self.dot {
                1..=256 | 321..=336 => self.bg_fetch(cart),
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
        let scanline_len = if self.scanline == PRERENDER_SCANLINE && self.odd_frame && render {
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
    fn shift_bg_shifters(&mut self) {
        self.bg_shift_lo <<= 1;
        self.bg_shift_hi <<= 1;
        self.bg_shift_attr_lo <<= 1;
        self.bg_shift_attr_hi <<= 1;
    }

    /// Run the per-8-dot background tile fetch sequence (called at dots 1..=336).
    fn bg_fetch(&mut self, cart: Option<&Cartridge>) {
        match self.dot % 8 {
            1 => {
                // Reload shift registers with the tile fetched in the previous 8-dot window
                self.reload_bg_shifters();
                // Fetch nametable byte
                let nt_addr = 0x2000 | (self.v & 0x0FFF);
                self.bg_nt_byte = self.ppu_read(nt_addr, cart);
            }
            3 => {
                // Fetch attribute byte
                let attr_addr =
                    0x23C0 | (self.v & 0x0C00) | ((self.v >> 4) & 0x38) | ((self.v >> 2) & 0x07);
                let attr = self.ppu_read(attr_addr, cart);
                // Select the 2-bit palette for the current 2×2 tile quadrant
                let shift = ((self.v >> 4) & 0x04) | (self.v & 0x02);
                self.bg_attr_byte = (attr >> shift) & 0x03;
            }
            5 => {
                let fine_y = (self.v >> 12) & 0x07;
                let addr = self.bg_pattern_base() | ((self.bg_nt_byte as u16) << 4) | fine_y;
                self.bg_pat_lo = self.ppu_read(addr, cart);
            }
            7 => {
                let fine_y = (self.v >> 12) & 0x07;
                let addr = self.bg_pattern_base() | ((self.bg_nt_byte as u16) << 4) | fine_y | 8;
                self.bg_pat_hi = self.ppu_read(addr, cart);
            }
            _ => {}
        }
    }

    // ── Sprite evaluation ─────────────────────────────────────────────────────

    /// Evaluate sprites for the NEXT scanline; runs during dots 65–256 of visible scanlines.
    fn evaluate_sprites(&mut self) {
        // Clear secondary OAM at dot 65 (after clearing cycle 1–64). Note this
        // must NOT touch sprite_count/sprite0_in_secondary — output_pixel()
        // is still reading those every dot through 256 to render sprites
        // found during the PREVIOUS scanline's evaluation.
        if self.dot == 65 {
            self.secondary_oam = [0xFF; 32];
            self.sprite_eval_count = 0;
            self.sprite0_eval = false;
        }
        if self.dot < 65 || self.dot > 256 {
            return;
        }
        if self.sprite_eval_count >= 8 {
            // Already found 8 sprites; check overflow (simplified — no hardware bug)
            return;
        }
        // Each OAM entry is 4 bytes; sprite index = (dot-65)/4 maps badly here,
        // so we only do the full scan once at dot 256 to keep it simple.
        if self.dot != 256 {
            return;
        }
        // Pre-render's "next scanline" wraps around to scanline 0 of the new
        // frame, not 262 (which doesn't exist).
        let next_scanline = if self.scanline == PRERENDER_SCANLINE {
            0
        } else {
            self.scanline + 1
        };
        let height = self.sprite_height() as i32;
        let mut n = 0usize;
        while n < 64 && self.sprite_eval_count < 8 {
            let y = self.oam[n * 4];
            // OAM Y is the sprite's top row minus 1 (hardware delays sprite
            // rendering by one scanline) — matches fetch_sprites()'s row calc,
            // which already subtracts this same 1. Plain (non-wrapping)
            // distance: a sprite with Y near 255 (a common "hide it"
            // convention, since y+1 already exceeds every valid scanline)
            // must never wrap around to become visible at the top of the
            // screen — wrapping_sub would do that for small next_scanline.
            let row = next_scanline as i32 - y as i32 - 1;
            let in_range = (0..height).contains(&row);
            if in_range {
                let dst = self.sprite_eval_count * 4;
                self.secondary_oam[dst..dst + 4].copy_from_slice(&self.oam[n * 4..n * 4 + 4]);
                if n == 0 {
                    self.sprite0_eval = true;
                }
                self.sprite_eval_count += 1;
            }
            n += 1;
        }
        if n < 64 && self.sprite_eval_count == 8 {
            self.sprite_overflow = true;
        }
    }

    /// Fetch sprite patterns for the sprites found in secondary OAM (dots 257–320).
    fn fetch_sprites(&mut self, cart: Option<&Cartridge>) {
        if self.dot < 257 || self.dot > 320 {
            return;
        }
        // Evaluation for this scanline finished at dot 256; hand its results
        // off to the render-facing fields now, before output_pixel() needs
        // them on the very next scanline's dot 1.
        if self.dot == 257 {
            self.sprite_count = self.sprite_eval_count;
            self.sprite0_in_secondary = self.sprite0_eval;
        }
        let idx = ((self.dot - 257) / 8) as usize;
        if idx >= self.sprite_count {
            return;
        }
        // Only do the fetch on the last dot of each 8-dot slot
        if (self.dot - 257) % 8 != 7 {
            return;
        }

        let y_pos = self.secondary_oam[idx * 4] as u16;
        let tile = self.secondary_oam[idx * 4 + 1];
        let attr = self.secondary_oam[idx * 4 + 2];
        let x_pos = self.secondary_oam[idx * 4 + 3];
        let flip_v = attr & 0x80 != 0;
        let height = self.sprite_height();

        let mut row = (self.scanline + 1).saturating_sub(y_pos + 1);
        if flip_v {
            row = (height - 1) - row;
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

        let row_in_tile = row & 0x07;
        let addr_lo = pt_base | (tile_idx << 4) | row_in_tile;
        let lo = self.ppu_read(addr_lo, cart);
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
        self.sprite_x[idx] = x_pos;
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

        // Sprite pixel (first non-transparent sprite wins)
        let (sp_pal, sp_col, sp_priority, sp_is_zero) = if sp_enabled && !sp_left_clip {
            let mut result = (0u8, 0u8, false, false);
            for i in 0..self.sprite_count {
                // Plain (non-wrapping) distance: a sprite at X close to 255
                // is clipped by the right edge of the screen, not wrapped
                // around to reappear at x=0 — wrapping_sub would do that.
                let x_dist = x as i32 - self.sprite_x[i] as i32;
                if !(0..8).contains(&x_dist) {
                    continue;
                }
                let bit = 7 - x_dist as u8;
                let lo = (self.sprite_shift_lo[i] >> bit) & 1;
                let hi = (self.sprite_shift_hi[i] >> bit) & 1;
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
            self.sprite0_hit_pending = Some((true, SPRITE0_HIT_LATCH_DOTS));
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

    fn ppu_read(&self, addr: u16, cart: Option<&Cartridge>) -> u8 {
        match addr & 0x3FFF {
            0x0000..=0x1FFF => cart.map_or(0, |c| c.chr_read(addr)),
            0x2000..=0x3EFF => self.vram[self.mirror_vram_addr(addr)],
            _ => self.palette[Self::palette_idx(addr)],
        }
    }

    fn ppu_write(&mut self, addr: u16, data: u8, cart: Option<&mut Cartridge>) {
        match addr & 0x3FFF {
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

    /// Read a PPU register. `reg` is the 3-bit register select (addr & 7).
    pub fn read_register(&mut self, reg: u8, cart: Option<&Cartridge>) -> u8 {
        match reg {
            // $2002 PPUSTATUS — reading clears VBlank flag and write toggle
            2 => {
                let status = ((self.vblank as u8) << 7)
                    | ((self.sprite0_hit as u8) << 6)
                    | ((self.sprite_overflow as u8) << 5);
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
            // $2004 OAMDATA
            4 => self.oam[self.oam_addr as usize],
            // $2007 PPUDATA — non-palette reads are buffered one cycle
            7 => {
                let addr = self.v;
                self.v = self.v.wrapping_add(self.vram_increment());
                if addr & 0x3FFF >= 0x3F00 {
                    // Palette: return immediately; refresh buffer from nametable behind palette
                    self.read_buf = self.ppu_read(addr & 0x2FFF, cart);
                    self.ppu_read(addr, cart)
                } else {
                    let buffered = self.read_buf;
                    self.read_buf = self.ppu_read(addr, cart);
                    buffered
                }
            }
            _ => 0,
        }
    }

    /// Write a PPU register. `reg` is the 3-bit register select (addr & 7).
    pub fn write_register(&mut self, reg: u8, data: u8, cart: Option<&mut Cartridge>) {
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
            // $2001 PPUMASK
            1 => self.mask = data,
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
                }
                self.w = !self.w;
            }
            // $2007 PPUDATA
            7 => {
                let addr = self.v;
                self.ppu_write(addr, data, cart);
                self.v = self.v.wrapping_add(self.vram_increment());
            }
            _ => {}
        }
    }

    /// Write a single byte to OAM at offset `offset` (used by OAM DMA via $4014).
    /// DMA starts at the current OAMADDR ($2003) and wraps mod 256.
    pub fn oam_dma_write(&mut self, offset: u8, data: u8) {
        self.oam[self.oam_addr.wrapping_add(offset) as usize] = data;
    }
}
