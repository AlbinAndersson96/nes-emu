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

pub struct Ppu {
    // Programmer-visible write-only registers
    ctrl: u8,     // $2000 PPUCTRL
    mask: u8,     // $2001 PPUMASK
    oam_addr: u8, // $2003 OAMADDR

    // Loopy registers (internal to the PPU, exposed via $2005/$2006/$2007)
    v: u16,      // current VRAM address (15-bit)
    t: u16,      // temporary VRAM address (15-bit)
    fine_x: u8,  // fine X scroll (3-bit)
    w: bool,     // write toggle: false = first write, true = second write

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

    // NMI edge detector
    nmi_pending: bool,

    // Dot/scanline counters (PPU runs at 3× CPU clock)
    dot: u16,
    scanline: u16,
    odd_frame: bool,
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
            palette: [0u8; 32],
            vblank: false,
            sprite0_hit: false,
            sprite_overflow: false,
            nmi_pending: false,
            dot: 0,
            scanline: 0,
            odd_frame: false,
        }
    }

    /// Advance PPU by `cpu_cycles` CPU cycles (= 3× PPU dots each).
    pub fn tick(&mut self, cpu_cycles: u64) {
        for _ in 0..cpu_cycles * 3 {
            self.clock_dot();
        }
    }

    /// Clock one PPU dot: run events for (scanline, dot), then advance counters.
    fn clock_dot(&mut self) {
        match self.scanline {
            VBLANK_SCANLINE if self.dot == 1 => {
                self.vblank = true;
                if self.ctrl & 0x80 != 0 {
                    self.nmi_pending = true;
                }
            }
            PRERENDER_SCANLINE if self.dot == 1 => {
                self.vblank = false;
                self.sprite0_hit = false;
                self.sprite_overflow = false;
            }
            _ => {}
        }

        // Advance dot; on odd frames with rendering enabled, pre-render scanline
        // is 340 dots (dot 339 is skipped, matching the NESdev "odd-frame short" behaviour).
        self.dot += 1;
        let scanline_len = if self.scanline == PRERENDER_SCANLINE
            && self.odd_frame
            && self.rendering_enabled()
        {
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
    }

    fn rendering_enabled(&self) -> bool {
        self.mask & 0x18 != 0
    }

    /// Returns true (and clears the latch) if an NMI is pending.
    pub fn take_nmi(&mut self) -> bool {
        let v = self.nmi_pending;
        self.nmi_pending = false;
        v
    }

    fn vram_increment(&self) -> u16 {
        if self.ctrl & 0x04 != 0 { 32 } else { 1 }
    }

    // Nametable mirroring: horizontal by default.
    // $2000/$2400 share the first 1 KB bank; $2800/$2C00 share the second.
    fn mirror_vram_addr(&self, addr: u16) -> usize {
        let addr = (addr & 0x0FFF) as usize;
        match addr {
            0x000..=0x3FF => addr,
            0x400..=0x7FF => addr - 0x400,
            0x800..=0xBFF => addr - 0x400,
            _ => addr - 0x800,
        }
    }

    fn ppu_read(&mut self, addr: u16) -> u8 {
        match addr & 0x3FFF {
            0x0000..=0x1FFF => 0, // CHR ROM/RAM — not yet wired to cartridge
            0x2000..=0x3EFF => {
                let idx = self.mirror_vram_addr(addr);
                self.vram[idx]
            }
            _ => {
                // Palette ($3F00–$3FFF, mirrored every 32 bytes)
                let mut idx = (addr & 0x001F) as usize;
                // Backdrop mirrors: $3F10/$3F14/$3F18/$3F1C → $3F00/$3F04/$3F08/$3F0C
                if idx >= 0x10 && idx % 4 == 0 {
                    idx -= 0x10;
                }
                self.palette[idx]
            }
        }
    }

    fn ppu_write(&mut self, addr: u16, data: u8) {
        match addr & 0x3FFF {
            0x0000..=0x1FFF => {} // CHR ROM/RAM — not yet wired
            0x2000..=0x3EFF => {
                let idx = self.mirror_vram_addr(addr);
                self.vram[idx] = data;
            }
            _ => {
                let mut idx = (addr & 0x001F) as usize;
                if idx >= 0x10 && idx % 4 == 0 {
                    idx -= 0x10;
                }
                self.palette[idx] = data;
            }
        }
    }

    /// Read a PPU register. `reg` is the 3-bit register select (addr & 7).
    pub fn read_register(&mut self, reg: u8) -> u8 {
        match reg {
            // $2002 PPUSTATUS — reading clears VBlank flag and write toggle
            2 => {
                let status = ((self.vblank as u8) << 7)
                    | ((self.sprite0_hit as u8) << 6)
                    | ((self.sprite_overflow as u8) << 5);
                self.vblank = false;
                self.nmi_pending = false; // suppress NMI that hasn't fired yet
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
                    // Palette: return immediately; refresh buffer from underlying nametable
                    self.read_buf = self.ppu_read(addr & 0x2FFF);
                    self.ppu_read(addr)
                } else {
                    let buffered = self.read_buf;
                    self.read_buf = self.ppu_read(addr);
                    buffered
                }
            }
            _ => 0,
        }
    }

    /// Write a PPU register. `reg` is the 3-bit register select (addr & 7).
    pub fn write_register(&mut self, reg: u8, data: u8) {
        match reg {
            // $2000 PPUCTRL
            0 => {
                let nmi_was_on = self.ctrl & 0x80 != 0;
                self.ctrl = data;
                let nmi_now_on = self.ctrl & 0x80 != 0;
                // Turning NMI enable on mid-VBlank fires an immediate NMI
                if !nmi_was_on && nmi_now_on && self.vblank {
                    self.nmi_pending = true;
                }
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
                    // First write: coarse X scroll into t[4:0], fine X separate
                    self.t = (self.t & 0xFFE0) | ((data as u16) >> 3);
                    self.fine_x = data & 0x07;
                } else {
                    // Second write: fine Y into t[14:12], coarse Y into t[9:5]
                    self.t = (self.t & 0x8C1F)
                        | ((data as u16 & 0x07) << 12)
                        | ((data as u16 & 0xF8) << 2);
                }
                self.w = !self.w;
            }
            // $2006 PPUADDR — two writes
            6 => {
                if !self.w {
                    // First write: high 6 bits of t (top 2 bits cleared)
                    self.t = (self.t & 0x00FF) | ((data as u16 & 0x3F) << 8);
                } else {
                    // Second write: low byte of t, then t → v
                    self.t = (self.t & 0xFF00) | data as u16;
                    self.v = self.t;
                }
                self.w = !self.w;
            }
            // $2007 PPUDATA
            7 => {
                let addr = self.v;
                self.ppu_write(addr, data);
                self.v = self.v.wrapping_add(self.vram_increment());
            }
            _ => {}
        }
    }

    /// Write a single byte to OAM at offset `offset` (used by OAM DMA via $4014).
    pub fn oam_dma_write(&mut self, offset: u8, data: u8) {
        self.oam[offset as usize] = data;
    }
}
