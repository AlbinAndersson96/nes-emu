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

> These are stubs in the current `Bus` implementation. A real PPU module will
> replace the `ppu_registers` array.

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

Writing 1 to bit 0 of $4016 continuously reloads both controller shift
registers from their latched button state. Writing 0 freezes the latch and
starts serial readout: each subsequent read of $4016 or $4017 shifts out one
button bit (A, B, Select, Start, Up, Down, Left, Right).

## Cartridge ($4020–$FFFF)

The cartridge receives all addresses not decoded by the CPU's internal logic.
Mapper 0 (NROM) is the only mapper currently supported:

- **16 KB PRG-ROM**: mapped at both $8000–$BFFF and $C000–$FFFF (mirrored).
- **32 KB PRG-ROM**: mapped linearly at $8000–$FFFF.
- **WRAM** (optional, 8 KB): at $6000–$7FFF; battery-backed.

The iNES header byte 4 gives the number of 16 KB PRG-ROM banks. The `Cartridge`
struct mirrors smaller ROMs via `(addr - 0x8000) % prg_rom.len()`.

### Interrupt vectors
The cartridge ROM supplies all three vectors in its final 6 bytes:

| Address | Vector |
|---|---|
| $FFFA/$FFFB | NMI |
| $FFFC/$FFFD | RESET |
| $FFFE/$FFFF | IRQ / BRK |

## Implementation Notes

- `Bus::read` takes `&self` (no mutation on reads) except for PPU $2002 and
  $2007 which have read-side effects. Those will need `Cell`/`RefCell` or an
  `&mut self` signature change when the PPU is wired in.
- OAM DMA ($4014) will need special handling in the CPU step loop: the bus
  triggers a 513/514-cycle copy from a CPU RAM page into OAM; this is not a
  normal memory read and must suspend the CPU for the duration.
- Controller reads shift out bits LSB-first. After 8 bits the remaining reads
  return 1 (open bus / pull-up).
