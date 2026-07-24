use serde::{Deserialize, Serialize};
/// Output timer periods for the DMC (NTSC), in CPU cycles, indexed by $4010 bits 3–0.
const NTSC_RATE: [u16; 16] = [
    428, 380, 340, 320, 286, 254, 226, 214, 190, 160, 142, 128, 106, 84, 72, 54,
];

/// Delta-modulation channel ($4010–$4013).
///
/// Streams 1-bit delta-encoded PCM from CPU address space $C000–$FFFF. DMA reads
/// stall the CPU for 4 cycles and must be driven by the bus — the DMC signals its
/// need via `needs_dma()` / `dma_address()` and accepts the fetched byte via
/// `supply_dma_byte()`.
#[derive(Serialize, Deserialize)]
pub struct DmcChannel {
    pub irq_flag: bool,
    irq_enabled: bool,
    loop_flag: bool,
    rate_index: u8,
    /// 7-bit output level (DAC).
    output_level: u8,
    /// Base sample address: $C000 + ($4012 << 6).
    sample_address: u16,
    /// Sample byte count: ($4013 << 4) | 1.
    sample_length: u16,
    // --- reader state ---
    current_address: u16,
    bytes_remaining: u16,
    sample_buffer: Option<u8>,
    // --- output unit state ---
    shift_register: u8,
    bits_remaining: u8,
    silence: bool,
    /// APU-rate down-counter; reloaded from NTSC_RATE[rate_index] / 2.
    timer: u16,
}

impl DmcChannel {
    pub fn new() -> Self {
        Self {
            irq_flag: false,
            irq_enabled: false,
            loop_flag: false,
            rate_index: 0,
            output_level: 0,
            sample_address: 0xC000,
            sample_length: 1,
            current_address: 0xC000,
            bytes_remaining: 0,
            sample_buffer: None,
            shift_register: 0,
            bits_remaining: 0,
            silence: true,
            // Period halved because we clock at APU rate (every 2 CPU
            // cycles), minus 1 because the countdown fires on the 0 tick.
            timer: (NTSC_RATE[0] >> 1) - 1,
        }
    }

    /// Register write. `reg` is 0–3 ($4010–$4013).
    pub fn write_reg(&mut self, reg: u8, data: u8) {
        match reg {
            0 => {
                self.irq_enabled = data & 0x80 != 0;
                self.loop_flag = data & 0x40 != 0;
                self.rate_index = data & 0x0F;
                if !self.irq_enabled {
                    self.irq_flag = false;
                }
            }
            1 => {
                self.output_level = data & 0x7F;
            }
            2 => {
                self.sample_address = 0xC000 | (u16::from(data) << 6);
            }
            3 => {
                self.sample_length = (u16::from(data) << 4) | 1;
            }
            _ => {}
        }
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if enabled {
            if self.bytes_remaining == 0 {
                self.current_address = self.sample_address;
                self.bytes_remaining = self.sample_length;
            }
        } else {
            self.bytes_remaining = 0;
        }
    }

    /// True while there are sample bytes left to play.
    pub fn active(&self) -> bool {
        self.bytes_remaining > 0
    }

    /// True when the output unit needs a new byte from the bus.
    pub fn needs_dma(&self) -> bool {
        self.sample_buffer.is_none() && self.bytes_remaining > 0
    }

    /// The CPU address the DMC reader wants to fetch next.
    pub fn dma_address(&self) -> u16 {
        self.current_address
    }

    /// Called by the bus after it has stalled the CPU and performed the DMA read.
    pub fn supply_dma_byte(&mut self, data: u8) {
        self.sample_buffer = Some(data);

        self.current_address = self.current_address.wrapping_add(1);
        if self.current_address == 0 {
            // Address wraps from $FFFF to $8000 (not $0000)
            self.current_address = 0x8000;
        }

        self.bytes_remaining -= 1;
        if self.bytes_remaining == 0 {
            if self.loop_flag {
                self.current_address = self.sample_address;
                self.bytes_remaining = self.sample_length;
            } else if self.irq_enabled {
                self.irq_flag = true;
            }
        }
    }

    /// APU-rate clock (every 2 CPU cycles).
    pub fn clock_timer(&mut self) -> bool {
        if self.timer > 0 {
            self.timer -= 1;
            return false;
        }
        // Reload with period-1: a countdown that fires on the tick where it
        // reads 0 spans reload+1 ticks, so NTSC_RATE/2 here would stretch the
        // output-bit period to NTSC_RATE+2 CPU cycles (AccuracyCoin's DMASync
        // hard-codes the true 432-cycle fetch spacing at rate $F and can
        // never lock on otherwise).
        self.timer = (NTSC_RATE[self.rate_index as usize] >> 1) - 1;
        self.clock_output_unit();
        // Return true when a DMA fetch is needed so the caller can stall the CPU.
        self.needs_dma()
    }

    fn clock_output_unit(&mut self) {
        // Load a new byte into the shift register when the previous one is spent.
        if self.bits_remaining == 0 {
            self.bits_remaining = 8;
            match self.sample_buffer.take() {
                Some(byte) => {
                    self.silence = false;
                    self.shift_register = byte;
                }
                None => {
                    self.silence = true;
                }
            }
        }

        if !self.silence {
            if self.shift_register & 1 == 1 {
                if self.output_level <= 125 {
                    self.output_level += 2;
                }
            } else if self.output_level >= 2 {
                self.output_level -= 2;
            }
        }
        self.shift_register >>= 1;
        self.bits_remaining -= 1;
    }

    /// 7-bit sample output (0–127).
    pub fn output(&self) -> u8 {
        self.output_level
    }
}
