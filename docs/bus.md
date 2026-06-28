# NES System Bus

The NES CPU has a 16-bit address space ($0000–$FFFF). The system bus decodes
this into regions routed to RAM, the PPU, the APU, controllers, and the
cartridge. The CPU itself only calls `bus.read(addr)` / `bus.write(addr, data)`
and is unaware of the mapping.

## Address Map

| Range | Size | Device | Notes |
|---|---|---|---|
| $0000–$07FF | 2 KB | Internal RAM | Zero page $00–$FF, stack $0100–$01FF |
| $0800–$0FFF | 2 KB | RAM mirror 1 | `addr & 0x07FF` |
| $1000–$17FF | 2 KB | RAM mirror 2 | `addr & 0x07FF` |
| $1800–$1FFF | 2 KB | RAM mirror 3 | `addr & 0x07FF` |
| $2000–$2007 | 8 B | PPU registers | PPUCTRL … PPUDATA |
| $2008–$3FFF | ~8 KB | PPU register mirrors | `addr & 0x2007` |
| $4000–$4015 | 22 B | APU registers | Pulse, triangle, noise, DMC |
| $4016 | 1 B | Controller 1 | Serial shift register |
| $4017 | 1 B | Controller 2 / APU frame | Dual role |
| $4018–$401F | 8 B | Disabled | Open bus on retail hardware |
| $4020–$5FFF | — | Cartridge expansion | Rarely used |
| $6000–$7FFF | 8 KB | Cartridge WRAM | Battery-backed save RAM (optional) |
| $8000–$FFFF | 32 KB | PRG-ROM / mapper | Interrupt vectors at $FFFA–$FFFF |

## Mirroring

### RAM mirrors ($0800–$1FFF)
Only 11 address lines are connected to the 2 KB RAM chip. The upper bits are
ignored, so any address in $0000–$1FFF maps to `addr & 0x07FF`.

### PPU register mirrors ($2008–$3FFF)
Only 3 address lines reach the PPU's register select input. Any address in
$2000–$3FFF maps to PPU register `addr & 0x0007`, equivalent to `addr & 0x2007`
for the canonical address.

## PPU Registers ($2000–$2007)

| Offset | Name | R/W | Description |
|---|---|---|---|
| $2000 | PPUCTRL | W | Control flags (NMI enable, sprite size, etc.) |
| $2001 | PPUMASK | W | Rendering enable, colour emphasis |
| $2002 | PPUSTATUS | R | VBlank flag, sprite 0 hit, sprite overflow |
| $2003 | OAMADDR | W | OAM address |
| $2004 | OAMDATA | R/W | OAM data |
| $2005 | PPUSCROLL | W | Scroll position (write twice) |
| $2006 | PPUADDR | W | VRAM address (write twice) |
| $2007 | PPUDATA | R/W | VRAM data (auto-increments address) |

> These are stubs — `ppu_registers` is a plain byte array. Only `$2002`
> (PPUSTATUS) is partially implemented: the VBlank flag is derived from the
> PPU's cycle counter in `Ppu::read_status()`. A full PPU module will replace
> this array and implement all register side-effects.

## APU / I/O ($4000–$4017)

| Range | Function |
|---|---|
| $4000–$4003 | Pulse 1 |
| $4004–$4007 | Pulse 2 |
| $4008–$400B | Triangle |
| $400C–$400F | Noise |
| $4010–$4013 | DMC |
| $4014 | OAM DMA (triggers 513/514-cycle transfer) |
| $4015 | APU status / channel enable |
| $4016 | Controller 1 strobe/read |
| $4017 | Controller 2 read / APU frame counter |

### APU Channel Registers

Each channel occupies 4 consecutive registers. Writes take effect immediately
unless otherwise noted.

**Pulse 1 ($4000–$4003) and Pulse 2 ($4004–$4007)**

| Offset | Bits | Field |
|---|---|---|
| +0 | 7–6 | Duty cycle (00=12.5%, 01=25%, 10=50%, 11=75%) |
| +0 | 5 | Length counter halt / envelope loop |
| +0 | 4 | Envelope: 0=decay, 1=constant volume |
| +0 | 3–0 | Volume / envelope divider period |
| +1 | 7 | Sweep enable |
| +1 | 6–4 | Sweep period |
| +1 | 3 | Sweep negate |
| +1 | 2–0 | Sweep shift count |
| +2 | 7–0 | Timer low byte |
| +3 | 7–3 | Length counter load index |
| +3 | 2–0 | Timer high 3 bits (writing this also restarts the envelope and phase) |

Pulse 1 uses ones-complement negation in its sweep unit; pulse 2 uses
twos-complement. This means pulse 1 cannot reach a period of 0 during sweep.

**Triangle ($4008–$400B)**

| Offset | Bits | Field |
|---|---|---|
| +0 | 7 | Length counter halt / linear counter control |
| +0 | 6–0 | Linear counter reload value |
| +2 | 7–0 | Timer low byte |
| +3 | 7–3 | Length counter load index |
| +3 | 2–0 | Timer high 3 bits |

The triangle sequencer outputs a 32-step waveform at the timer frequency.
The linear counter runs at the quarter-frame clock and halts the sequencer
when it reaches zero.

**Noise ($400C–$400F)**

| Offset | Bits | Field |
|---|---|---|
| +0 | 5 | Length counter halt / envelope loop |
| +0 | 4 | Constant volume |
| +0 | 3–0 | Volume / envelope divider |
| +2 | 7 | Mode (0=bit-1 feedback, 1=bit-6 feedback) |
| +2 | 3–0 | Period index (NTSC table of 16 values) |
| +3 | 7–3 | Length counter load |

The LFSR is 15 bits wide. In mode 0 the feedback tap is bit 1; in mode 1 it
is bit 6 (producing metallic percussion tones).

**DMC ($4010–$4013)**

| Offset | Bits | Field |
|---|---|---|
| +0 | 7 | IRQ enable |
| +0 | 6 | Loop |
| +0 | 3–0 | Rate index (NTSC: 16 values from 54 to 428 CPU cycles) |
| +1 | 6–0 | Direct load (7-bit DAC value) |
| +2 | 7–0 | Sample address: $C000 + (value × 64) |
| +3 | 7–0 | Sample length: (value × 16) + 1 bytes |

The DMC reader fetches bytes from CPU address space and delta-decodes them
into a 7-bit output level. Each byte drives 8 output steps. When the buffer
empties the channel signals a DMA need; `Bus::tick_apu()` arms a 4-cycle
stall counter, and `Bus::tick_dma()` fetches the byte from `dma_address()` and
supplies it via `supply_dma_byte()` on the final stall cycle.

### $4015 — APU Status (R/W)

**Write** — channel enable:

| Bit | Channel |
|---|---|
| 4 | DMC |
| 3 | Noise |
| 2 | Triangle |
| 1 | Pulse 2 |
| 0 | Pulse 1 |

Writing 0 to a channel bit silences it and clears its length counter.
Writing 1 restarts the DMC from its sample address if it had stopped.
Writing $4015 also clears the DMC IRQ flag.

**Read** — status:

| Bit | Meaning |
|---|---|
| 7 | DMC IRQ flag |
| 6 | Frame IRQ flag |
| 4 | DMC active (bytes remaining > 0) |
| 3–0 | Length counter active for noise / triangle / pulse2 / pulse1 |

Reading $4015 clears the frame IRQ flag (bit 6) but not the DMC IRQ flag.

### $4017 — Frame Counter (W) / Controller 2 (R)

Write to $4017 controls the APU frame counter:

| Bit | Meaning |
|---|---|
| 7 | Mode: 0 = 4-step (generates IRQ), 1 = 5-step (no IRQ) |
| 6 | IRQ inhibit: 1 = suppress frame IRQ |

Writing resets the frame cycle counter. If bit 6 is set, the frame IRQ flag
is also cleared. In 5-step mode ($80), an immediate quarter-frame and
half-frame clock fires on the write.

#### Frame Counter Step Sequences

**4-step mode (mode 0)** — generates an IRQ:

| CPU cycle | Quarter-frame | Half-frame | IRQ |
|---|---|---|---|
| 7,457 | ✓ | | |
| 14,913 | ✓ | ✓ | |
| 22,371 | ✓ | | |
| 29,829 | ✓ | ✓ | ✓ (if not inhibited) |
| 29,830 | | | reset to 0 |

**5-step mode (mode 1)** — no IRQ:

| CPU cycle | Quarter-frame | Half-frame |
|---|---|---|
| 7,457 | ✓ | |
| 14,913 | ✓ | ✓ |
| 22,371 | ✓ | |
| 29,829 | — | — |
| 37,281 | ✓ | ✓ |
| 37,282 | | reset to 0 |

Quarter-frame events clock the envelope generators and triangle linear
counter. Half-frame events additionally clock length counters and sweep units.

#### IRQ line behaviour

The frame IRQ flag (`frame_irq_flag`) persists once set until cleared by
reading $4015 or by writing $4017 with bit 6 set. The IRQ line is asserted
as long as `frame_irq_flag && !irq_inhibit`. `Apu::take_irq()` returns this
level directly so the bus can signal the CPU on every tick.

### Controller Strobe / Read ($4016–$4017)

Writing 1 to bit 0 of $4016 continuously reloads both controller shift
registers from their latched button state. Writing 0 freezes the latch and
starts serial readout: each subsequent read of $4016 or $4017 shifts out one
button bit (A, B, Select, Start, Up, Down, Left, Right).

## Cartridge ($4020–$FFFF)

The cartridge receives all addresses not decoded by the CPU's internal logic.
Two mappers are currently supported:

### Mapper 0 — NROM

- **16 KB PRG-ROM**: mapped at both $8000–$BFFF and $C000–$FFFF (mirrored).
- **32 KB PRG-ROM**: mapped linearly at $8000–$FFFF.
- **WRAM** (optional, 8 KB): at $6000–$7FFF; battery-backed.

The iNES header byte 4 gives the number of 16 KB PRG-ROM banks. The `Cartridge`
struct mirrors smaller ROMs via `(addr - 0x8000) % prg_rom.len()`.

### Mapper 1 — MMC1 (SxROM)

- **PRG-ROM** up to 512 KB: four bank modes controlled by register $E000.
  - Mode 0/1: switch full 32 KB at $8000.
  - Mode 2: fix $8000–$BFFF to first bank, switch $C000–$FFFF.
  - Mode 3: switch $8000–$BFFF, fix $C000–$FFFF to last bank.
- **CHR**: not yet wired (PPU rendering not implemented).
- **PRG-RAM** (8 KB): always present at $6000–$7FFF regardless of the iNES header
  flag, because blargg test ROMs write results there unconditionally.
- Configuration via a 5-bit serial shift register: five consecutive writes to any
  address $8000–$FFFF with bit 0 clock the shift register; the fifth write also
  carries the register-select bits (bits 13–14 of the address).

### Interrupt vectors
The cartridge ROM supplies all three vectors in its final 6 bytes:

| Address | Vector |
|---|---|
| $FFFA/$FFFB | NMI |
| $FFFC/$FFFD | RESET |
| $FFFE/$FFFF | IRQ / BRK |

## Implementation Notes

- `Bus::read` takes `&mut self`. Some registers have read side-effects ($2002
  clears the VBlank flag; $2007 auto-increments the VRAM address; controller
  reads shift out bits). The `&mut self` signature handles this without needing
  `Cell`/`RefCell`.
- OAM DMA ($4014) is handled cycle-by-cycle: writing $4014 sets `oam_dma_active`
  and arms a 513-cycle counter. While active, the run loop calls `Bus::tick_dma()`
  instead of `cpu.tick()`, copying one byte per two cycles from the source page
  into PPU OAM. (The +1 parity cycle for odd-cycle triggers is not yet modelled.)
- Controller reads shift out bits LSB-first. After 8 bits the remaining reads
  return 1 (open bus / pull-up).
