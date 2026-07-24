# NES PPU Implementation Reference

Source: https://www.nesdev.org/wiki/PPU (and sub-pages)

---

## Table of Contents

1. [Overview](#overview)
2. [PPU Memory Map](#ppu-memory-map)
3. [Registers](#registers)
4. [OAM](#oam)
5. [Internal Registers (t/v/x/w)](#internal-registers-tvxw)
6. [Rendering Pipeline](#rendering-pipeline)
7. [Background Rendering](#background-rendering)
8. [Sprite Rendering](#sprite-rendering)
9. [Palettes](#palettes)
10. [NMI](#nmi)
11. [Scrolling](#scrolling)
12. [Pixel Priority](#pixel-priority)
13. [Known Hardware Quirks](#known-hardware-quirks)

---

## Overview

The NES PPU (Picture Processing Unit) is the 2C02 chip (NTSC) / 2C07 (PAL). It generates a 256×240 pixel image at ~60 Hz (NTSC). The PPU has its own 16-bit address bus separate from the CPU's, with 2 KB of internal VRAM and optional cartridge-supplied CHR-ROM/RAM.

**Key numbers (NTSC):**

| Parameter | Value |
|-----------|-------|
| Dots per scanline | 341 |
| Visible scanlines | 240 (0–239) |
| Post-render scanline | 240 |
| VBlank scanlines | 241–260 |
| Pre-render scanline | 261 (−1) |
| Total scanlines/frame | 262 |
| CPU cycles per PPU dot | 1/3 (PPU runs at 3× CPU clock) |
| CPU cycles per frame | 29,780.5 (odd frames: 29,780) |

---

## PPU Memory Map

The PPU addresses 16 KB (`$0000`–`$3FFF`). Addresses `$4000`–`$FFFF` mirror `$0000`–`$3FFF`.

```
$0000–$0FFF  Pattern table 0  (CHR-ROM/RAM, 4 KB, tile data for background/sprites)
$1000–$1FFF  Pattern table 1  (CHR-ROM/RAM, 4 KB)
$2000–$23FF  Nametable 0      (1 KB)
$2400–$27FF  Nametable 1      (1 KB)
$2800–$2BFF  Nametable 2      (1 KB)
$2C00–$2FFF  Nametable 3      (1 KB)
$3000–$3EFF  Mirror of $2000–$2EFF
$3F00–$3F1F  Palette RAM      (32 bytes)
$3F20–$3FFF  Mirror of $3F00–$3F1F (repeated)
```

### Pattern Tables

Each pattern table holds 256 tiles. Each tile is 8×8 pixels encoded as two 1-bit planes:

- **Plane 0** (low bit): bytes `$000`–`$007` of the tile (row 0 at byte 0)
- **Plane 1** (high bit): bytes `$008`–`$00F` of the tile

For each pixel, combine bit N of the corresponding plane 0 and plane 1 bytes to get a 2-bit value (0–3). Combined with a 2-bit palette attribute, this gives a 4-bit palette index.

### Nametables

The NES has 2 KB of internal VRAM holding two physical nametables. The remaining two nametable addresses (`$2800`–`$2FFF`) are mirrors controlled by the cartridge's mirroring mode:

| Mode | $2000 | $2400 | $2800 | $2C00 |
|------|-------|-------|-------|-------|
| Horizontal | A | A | B | B |
| Vertical | A | B | A | B |
| Single A | A | A | A | A |
| Single B | B | B | B | B |
| Four-screen | A | B | C | D (cartridge supplies extra RAM) |

Each nametable is 1024 bytes:
- Bytes `$000`–`$3BF` (960 bytes): tile indices, 32×30 grid
- Bytes `$3C0`–`$3FF` (64 bytes): attribute table, 8×8 grid of 2-bit palette selectors

### Attribute Table

Each attribute byte covers a 4×4 block of tiles (32×32 pixels), split into four 2×2 quadrants:

```
Bits 7-6: bottom-right quadrant palette
Bits 5-4: bottom-left  quadrant palette
Bits 3-2: top-right    quadrant palette
Bits 1-0: top-left     quadrant palette
```

The attribute byte address for tile (TX, TY):
```
attr_addr = 0x23C0 | (nametable & 0x0C00) | ((TY / 4) << 3) | (TX / 4)
```
The quadrant within the byte:
```
shift = ((TY & 2) << 1) | (TX & 2)
palette = (attr_byte >> shift) & 0x03
```

### Palette RAM

32 bytes of internal palette RAM, not addressable via VRAM (reads/writes go through `$2007` with the PPU address pointing here).

```
$3F00        Universal background color
$3F01–$3F03  Background palette 0 (colors 1–3)
$3F05–$3F07  Background palette 1 (colors 1–3)
$3F09–$3F0B  Background palette 2 (colors 1–3)
$3F0D–$3F0F  Background palette 3 (colors 1–3)
$3F10        Mirror of $3F00 (universal background)
$3F11–$3F13  Sprite palette 0 (colors 1–3)
$3F15–$3F17  Sprite palette 1 (colors 1–3)
$3F19–$3F1B  Sprite palette 2 (colors 1–3)
$3F1D–$3F1F  Sprite palette 3 (colors 1–3)
```

`$3F04`, `$3F08`, `$3F0C` are unused background palette slots; `$3F10`, `$3F14`, `$3F18`, `$3F1C` mirror `$3F00`, `$3F04`, `$3F08`, `$3F0C`. Palette index 0 of any palette is transparent for sprites and maps to the universal background color.

---

## Registers

CPU-visible PPU registers are mapped at `$2000`–`$2007`, with `$4014` for OAM DMA.

### $2000 — PPUCTRL (write)

```
7  bit  0
---- ----
VPHB SINN
|||| ||||
|||| ||++-- Base nametable address (0=$2000, 1=$2400, 2=$2800, 3=$2C00)
|||| ||       (also sets bits 10–11 of internal t register)
|||| |+---- VRAM address increment per $2007 CPU read/write
|||| |        0: add 1 (going across); 1: add 32 (going down)
|||| +----- Sprite pattern table address for 8×8 sprites
||||          0: $0000; 1: $1000 (ignored in 8×16 mode)
|||+------- Background pattern table address (0: $0000; 1: $1000)
||+-------- Sprite size (0: 8×8; 1: 8×16)
|+--------- PPU master/slave select (0: read from EXT; 1: write to EXT) — avoid on consumer hardware
+---------- Generate NMI at start of VBlank (0: off; 1: on)
```

Writing `$2000` sets bits 10–11 of the internal `t` register to the nametable select bits (bits 0–1 of the written value).

### $2001 — PPUMASK (write)

```
7  bit  0
---- ----
BGRs bMmG
|||| ||||
|||| |||+-- Greyscale (0: normal; 1: produce greyscale image)
|||| ||+--- Show background in leftmost 8 pixels (0: hide; 1: show)
|||| |+---- Show sprites in leftmost 8 pixels (0: hide; 1: show)
|||| +----- Show background
|||+------- Show sprites
||+-------- Emphasize red   (NTSC) / green (PAL)
|+--------- Emphasize green (NTSC) / red   (PAL)
+---------- Emphasize blue
```

Rendering is enabled when bits 3 or 4 are set. When both are clear, the PPU is in "forced blank" — the VRAM address is not updated during rendering, and reads/writes to `$2007` behave normally at any time.

### $2002 — PPUSTATUS (read)

```
7  bit  0
---- ----
VSO. ....
||
|+--------- Sprite 0 hit
|             Set when a non-transparent sprite-0 pixel overlaps a non-transparent background pixel.
|             Cleared at dot 1 of the pre-render scanline (scanline 261).
+---------- VBlank flag
              Set at dot 1 of scanline 241.
              Cleared at dot 1 of scanline 261 (pre-render).
              Cleared on read of $2002.

Bits 4–0: stale bits from the last CPU write to a PPU register (open bus behavior on real hardware).
Bit 5: Sprite overflow flag (buggy on hardware — see quirks section).
```

**Side effects of reading `$2002`:**
- VBlank flag (bit 7) is cleared.
- The write-toggle `w` is cleared (resets the PPUSCROLL/PPUADDR latch).
- If read exactly at the cycle VBlank is set (dot 1, scanline 241), the NMI may be suppressed for that frame.

### $2003 — OAMADDR (write)

Sets the address in OAM for the next `$2004` read/write. OAM is 256 bytes. On most games, OAMADDR is written to `$00` before a DMA.

**Hardware quirk**: writing `$2003` during rendering can corrupt OAM. During rendering, the PPU increments OAMADDR in ways that may differ from the written value.

### $2004 — OAMDATA (read/write)

Read/write the byte at the current OAMADDR, then increment OAMADDR. Reads during rendering (scanlines 0–239 when rendering is enabled) return OAM bytes being used by the sprite evaluation unit, not the current OAMADDR.

Writing during rendering is undefined. Use `$4014` (OAM DMA) instead.

### $2005 — PPUSCROLL (write ×2)

Controls the fine scroll position. Two writes per frame (first X, then Y), tracked by the `w` toggle.

- **First write** (w=0): sets `t[4:0]` = coarse X (pixel >> 3), sets `x` = fine X (pixel & 7). Clears `w`.
- **Second write** (w=1): sets `t[14:12]` = fine Y (pixel & 7), `t[9:5]` = coarse Y (pixel >> 3). Clears `w`.

### $2006 — PPUADDR (write ×2)

Sets the VRAM address for `$2007` accesses. Two writes (high byte first), tracked by the `w` toggle.

- **First write** (w=0): sets `t[13:8]` = data[5:0], clears `t[14]`, clears `w`.
- **Second write** (w=1): sets `t[7:0]` = data[7:0], copies `t` into `v`, clears `w`.

This sharing of `t` with `$2005` is what makes simultaneous scroll + vram-address manipulation interact in complex ways (see [Scrolling](#scrolling)).

### $2007 — PPUDATA (read/write)

Read/write the byte at the current VRAM address `v`, then increment `v` by 1 or 32 (per PPUCTRL bit 2).

**Read buffer**: PPUDATA reads are delayed by one read cycle. The CPU gets the value from an internal read buffer, which is then filled with the value at `v`. Exception: reads from palette RAM (`$3F00`–`$3FFF`) return the palette data immediately (buffer is still updated with nametable data "under" the palette address).

During rendering (enabled + scanlines 0–239), reads/writes to `$2007` cause the PPU's `v` register to be incremented both coarse-X and fine-Y simultaneously, which corrupts the scroll state.

### $4014 — OAMDMA (write)

Writing a value `N` to `$4014` initiates an OAM DMA: the CPU is suspended for 513 or 514 cycles (514 on an odd CPU cycle), and 256 bytes from CPU page `$NN00`–`$NNFF` are written to OAM starting at the current OAMADDR. This is the standard way to upload sprite data each frame.

---

## OAM

OAM (Object Attribute Memory) is 256 bytes of internal PPU RAM storing data for up to 64 sprites. Each sprite is 4 bytes:

| Byte | Bits | Description |
|------|------|-------------|
| 0 | 7–0 | Y position minus 1 (sprite is at Y+1; values ≥ $EF hide the sprite) |
| 1 | 7–0 | Tile index (for 8×16: bit 0 selects pattern table; bits 7–1 are tile number) |
| 2 | 7 | Flip vertically (0: normal; 1: flipped) |
| 2 | 6 | Flip horizontally (0: normal; 1: flipped) |
| 2 | 5 | Sprite priority (0: in front of background; 1: behind background) |
| 2 | 4–2 | Unimplemented (read as 0) |
| 2 | 1–0 | Palette (4–7; combined with 2-bit CHR pixel to form 4-bit palette index) |
| 3 | 7–0 | X position |

### Secondary OAM

During rendering, the PPU copies up to 8 sprites from primary OAM into secondary OAM (32 bytes) for the current scanline. Sprites are evaluated starting from sprite 0. The "sprite overflow" flag (PPUSTATUS bit 5) is set when more than 8 sprites appear on the same scanline, but due to a hardware bug this flag may be set or cleared incorrectly (see [Known Hardware Quirks](#known-hardware-quirks)).

---

## Internal Registers (t/v/x/w)

The PPU has four internal registers not directly accessible by the CPU:

| Register | Width | Description |
|----------|-------|-------------|
| `v` | 15 bits | Current VRAM address (used for background fetches during rendering, and for $2007 R/W) |
| `t` | 15 bits | Temporary VRAM address; also holds top-left tile position of the next display frame |
| `x` | 3 bits | Fine X scroll (0–7) |
| `w` | 1 bit | Write toggle for $2005/$2006 (0=first write, 1=second write) |

### `v` and `t` bit layout

```
yyy NN YYYYY XXXXX
||| || ||||| +++++-- coarse X scroll (tile column 0–31)
||| || +++++-------- coarse Y scroll (tile row 0–29)
||| ++-------------- nametable select (0–3)
+++----------------- fine Y scroll (0–7)
```

Bit 14 is unused during rendering. The full 15-bit layout:

```
Bit: 14  13  12  11  10  9   8   7   6   5   4   3   2   1   0
      y2  y1  y0  N1  N0  Y4  Y3  Y2  Y1  Y0  X4  X3  X2  X1  X0
```

Where `y` = fine Y (3 bits), `N` = nametable (2 bits), `Y` = coarse Y (5 bits), `X` = coarse X (5 bits).

---

## Rendering Pipeline

### Scanline Timing

Each scanline is 341 PPU dots (dot 0 is idle). One PPU dot = 1/3 CPU cycle.

```
Scanline  -1 (261): Pre-render  — clears VBlank/sprite-0/overflow; loads scroll
Scanlines  0–239:   Visible     — background + sprite fetches, pixel output
Scanline  240:      Post-render — idle
Scanlines 241–260:  VBlank      — NMI fires at dot 1 of scanline 241
```

### Dot-level Timeline (visible scanlines + pre-render)

```
Dot   0:        Idle
Dots  1–256:    Pixel output (dot 1 = leftmost pixel column 0)
  Each 8 dots:  Fetch nametable byte, attribute byte, pattern low, pattern high
Dots 257–320:   Sprite fetches for NEXT scanline (8 sprites × 8 cycles each)
Dots 321–336:   First two tile fetches for next scanline (background pipe fill)
Dots 337–340:   Two dummy nametable fetches
```

### Pre-render Scanline (261)

- **Dot 1**: Clear VBlank flag, sprite-0 hit, sprite overflow.
- **Dots 1–256**: Background fetches proceed as during visible scanlines (address updates happen even though no pixels are output).
- **Dots 280–304**: If rendering enabled, `v[14:11]` (fine Y + coarse Y) and `v[11:10]` (nametable Y bits) are reloaded from `t` (vertical scroll reset).
- **Dot 339** (odd frames only, rendering enabled): The pre-render scanline is one dot shorter — dot 339 is skipped, effectively advancing the dot counter to 0 of scanline 0.

### VBlank

- **Dot 1, scanline 241**: VBlank flag set in `$2002` bit 7. If PPUCTRL bit 7 is set, NMI is triggered.
- **Dot 1, scanline 261**: VBlank flag cleared.

---

## Background Rendering

### Per-8-dot Fetch Sequence

Each tile column in the visible area requires four PPU memory reads over 8 dots:

| Dot offset | Fetch |
|------------|-------|
| +1 | Nametable byte: `$2000 | (v & 0x0FFF)` |
| +3 | Attribute byte: `$23C0 | (v & 0x0C00) | ((v >> 4) & 0x38) | ((v >> 2) & 0x07)` |
| +5 | Pattern table low byte: `(PPUCTRL.BG_PT << 12) | (tile_id << 4) | fine_y` |
| +7 | Pattern table high byte: same as above + 8 |

Data is loaded into two 16-bit shift registers (one per pattern plane) and two 8-bit shift registers (attribute bits, reloaded every 8 dots). The fine-X scroll (`x`) selects which bit to output from these registers.

### Horizontal Scroll Update (dot 256 per scanline)

At dot 256 of each visible and pre-render scanline, the PPU increments the fine-Y component of `v`:

```rust
// Increment fine Y; carry into coarse Y; handle nametable wrap at row 29
if v & 0x7000 != 0x7000 {
    v += 0x1000;  // increment fine Y
} else {
    v &= !0x7000; // fine Y = 0
    let mut y = (v >> 5) & 0x1F;
    if y == 29 {
        y = 0;
        v ^= 0x0800;  // flip vertical nametable
    } else if y == 31 {
        y = 0;        // wrap without flipping (attribute area)
    } else {
        y += 1;
    }
    v = (v & !0x03E0) | (y << 5);
}
```

### Coarse-X Increment (every 8 dots)

At dots 8, 16, 24, …, 256 and 328, 336 (whenever coarse-X needs to advance):

```rust
if (v & 0x001F) == 31 {   // coarse X == 31
    v &= !0x001F;          // coarse X = 0
    v ^= 0x0400;           // flip horizontal nametable
} else {
    v += 1;                // increment coarse X
}
```

### Horizontal Scroll Copy (dot 257)

At dot 257, copy horizontal bits from `t` to `v`:

```rust
// Copy bits 0–4 (coarse X) and bit 10 (horizontal nametable) from t to v
v = (v & !0x041F) | (t & 0x041F);
```

---

## Sprite Rendering

### Sprite Evaluation (dots 1–64: secondary OAM cleared; dots 65–256: evaluation)

During the visible portion of each scanline, the PPU evaluates sprites for the *next* scanline:

1. **Dots 1–64**: Secondary OAM is filled with `$FF`.
2. **Dots 65–256**: The PPU iterates through primary OAM (sprites 0–63). For each sprite, if `Y ≤ current_scanline + 1 < Y + sprite_height`, the sprite is copied into secondary OAM. Up to 8 sprites are copied; the sprite-overflow flag is set if more would fit (with hardware bugs — see quirks).
3. **Dots 257–320**: Pattern table fetches for the 8 selected sprites; X coordinates and attributes latched.
4. **Dots 321–340**: Background pipe filled for next scanline.

### 8×16 Sprites

When PPUCTRL bit 5 is set, sprites are 8×16 pixels:
- Tile byte bit 0: selects pattern table (`0=$0000`, `1=$1000`)
- Tile byte bits 7–1: top tile number (bottom tile = top + 1, or top − 1 if vertically flipped)
- The top 8 rows use the top tile; bottom 8 rows use the bottom tile.

### Sprite-0 Hit

Sprite-0 hit is set when:
- The first sprite in OAM (sprite 0) has a non-transparent pixel (CHR bits ≠ 0)
- That pixel overlaps a non-transparent background pixel (CHR bits ≠ 0 and the BG pixel is not the universal background color)
- Background rendering (`$2001` bit 3) and sprite rendering (`$2001` bit 4) are both enabled
- The pixel is not in column 255 (dot 256 is excluded on real hardware)
- If `$2001` bits 1–2 are 0 (left-clip enabled), columns 0–7 are excluded

Sprite-0 hit is **not** cleared when the sprite-0 pixel is obscured by another sprite. It is only cleared at dot 1 of the pre-render scanline.

---

## Palettes

The NES palette has 64 entries (6-bit values, `$00`–`$3F`; values `$0D` and above `$3F` are black or undefined). The 32 bytes of palette RAM index into these 64 colors.

Color bits `$xE` and `$xF` are considered identical to `$xD` on real hardware (dark greys map to black). Values `$20` and `$30` are both "white."

### Palette RAM Quirk

Reads from `$3F10`, `$3F14`, `$3F18`, `$3F1C` return the same data as `$3F00`, `$3F04`, `$3F08`, `$3F0C` respectively. When reading `$2007` with the PPU address pointing to palette RAM, the read buffer is loaded with the nametable data "behind" the palette address (`addr & 0x2FFF`).

### Greyscale Mode

When `$2001` bit 0 is set, palette reads are ANDed with `$30`, forcing all colors to the greyscale entries (`$00`, `$10`, `$20`, `$30`).

---

## NMI

The PPU triggers a CPU NMI (Non-Maskable Interrupt) at the start of VBlank if:
- `$2002` bit 7 (VBlank flag) is set, **and**
- `$2000` bit 7 (NMI enable) is set

The NMI line is level-triggered with edge detection: NMI fires when VBLANK flag transitions from 0→1 while NMI-enable is 1, **or** when NMI-enable transitions from 0→1 while VBlank flag is already 1.

**Enabling NMI during VBlank**: If `$2000` bit 7 is written from 0 to 1 while the VBlank flag is already set, an NMI fires immediately.

**Disabling and re-enabling NMI**: If NMI-enable is cleared and then set again during VBlank before `$2002` is read, another NMI fires.

**Reading `$2002` near VBlank start**: If `$2002` is read at the exact cycle VBlank is set (dot 1, scanline 241), the VBlank flag is cleared before the CPU can see it, and the NMI is suppressed. The exact behavior depends on when within the dot the read occurs (race condition).

---

## Scrolling

The PPU's scrolling system is implemented through the shared `t`/`v`/`x`/`w` registers. A typical frame follows this sequence:

### Typical Frame Scroll Setup

```
During VBlank (after NMI):
  Write $2000: sets nametable bits → sets t[11:10]
  Write $2005 (first):  coarse X, fine X → t[4:0], x[2:0]; w=0→1
  Write $2005 (second): coarse Y, fine Y → t[9:5], t[14:12]; w=1→0
  Write $2006 (first):  upper addr bits → t[13:8], t[14]=0; w=0→1
  Write $2006 (second): lower addr bits → t[7:0], then v=t; w=1→0
```

Note: `$2006` writes override the scroll position set by `$2005` because the second `$2006` write copies all of `t` into `v`. Games that use `$2005` for scroll should not mix in `$2006` writes.

### Mid-frame Scroll (split scroll / raster tricks)

To change horizontal scroll mid-frame, write `$2005` first write (odd X) and `$2006` second write (lower address byte) on the appropriate scanline. The `t` register's horizontal bits will be copied into `v` at dot 257.

To change the nametable mid-frame, write `$2000` (which updates `t[11:10]`).

### Coarse Y and Nametable Wrapping

Coarse Y increments to 30 before wrapping (tile row 29 → 0 with nametable Y bit flipped). Rows 30–31 exist in the attribute area and are not valid tile rows; incrementing into row 31 wraps to 0 without flipping the nametable bit (this is a quirk used by some games).

---

## Pixel Priority

Each pixel passes through a priority multiplexer:

```
Background pixel: (BG_palette, BG_color)
Sprite pixel:     (SP_palette + 4, SP_color, SP_priority, sprite_index)

Output rule:
  if BG_color == 0 and SP_color == 0: universal background color ($3F00)
  if BG_color == 0 and SP_color != 0: sprite pixel
  if BG_color != 0 and SP_color == 0: background pixel
  if BG_color != 0 and SP_color != 0:
    if SP_priority == 0: sprite pixel    (in front)
    if SP_priority == 1: background pixel (behind)
  (sprite-0 hit check happens here when sprite_index == 0)
```

When rendering is disabled (`$2001` bits 3–4 both clear), the PPU outputs the universal background color unless `v` points into palette RAM (`$3F00`–`$3FFF`), in which case it outputs the palette color at that address.

---

## Known Hardware Quirks

### Sprite Overflow Bug

The sprite overflow flag (PPUSTATUS bit 5) has a well-documented hardware bug. When more than 8 sprites appear on a scanline, the hardware reads OAM with a corrupted index: it increments both the sprite index (bits 7–2) and the byte index within the sprite (bits 1–0) simultaneously. This means:

- Sprite overflow can be set even when there are ≤ 8 sprites on the line (false positive)
- Actual overflow (> 8 sprites) may not set the flag (false negative)

A correct emulation replicates this buggy behavior. The pseudocode:

```
n = 0, m = 0 (sprite index, byte index within sprite)
secondary_count = 0

while n < 64:
    y = OAM[n*4 + m]
    if y is in range for next scanline:
        copy OAM[n*4 .. n*4+3] to secondary OAM
        secondary_count++
        n++
        if secondary_count == 8:
            set overflow flag when any further sprite Y matches (with m advancing buggy)
    else:
        n++
        m = (m + 1) & 3  # BUG: m increments on miss
```

### PPUSTATUS Open Bus

Bits 4–0 of `$2002` are "open bus" — they return whatever value was last written to any PPU register. Precise emulation requires tracking a PPU bus latch.

### OAM Decay

On real hardware, unrefreshed OAM bits decay to 0 over time (~1 frame at room temperature). The Y-coordinate bytes decay fastest. Most emulators do not emulate this.

### $2004 Read During Rendering

Reading OAMDATA (`$2004`) during rendering (scanlines 0–239, rendering enabled) returns the internal OAM byte being accessed by the sprite evaluation unit, not the byte at OAMADDR. The returned value changes every 2 PPU cycles during sprite evaluation.

### Dot 0 on Odd Frames

On odd frames (counting from frame 0), when rendering is enabled, dot 339 of the pre-render scanline is skipped. This means odd frames are 1 PPU dot shorter: 89,341 instead of 89,342 dots. This causes a slight visual difference that manifests as a 1-pixel flicker in the top-left area with certain scrolling positions, but is important for cycle-accurate APU/IRQ timing.

### NMI at VBlank Edge

The CPU polling for NMI happens between instructions, once per CPU cycle. If `$2002` is read in the same CPU cycle that VBlank is set:
- The VBlank flag is never seen as 1 by the CPU (it was cleared before the read result propagates)
- The NMI that would have fired is suppressed

### PPU Warming Up

On power-on, the PPU takes approximately 27,384 CPU cycles (~2 frames) to stabilize. During this time, writes to `$2000` and `$2001` are ignored, and `$2002` reads return garbage. The `w` register is also in an undefined state after power-on. Emulators typically skip this behavior and treat the PPU as ready immediately.

### PPUDATA Read Buffer on Palette Addresses

When reading palette RAM via `$2007`, the read buffer is updated with the nametable byte at the mirrored address (`v & 0x2FFF`), not the palette byte. The returned value is the palette byte. This means a dummy read at a non-palette address is **not** required before reading palettes, but the read buffer does not contain the palette data afterward.

---

## Emulator Implementation Notes

The reference above describes the 2C02. This section records how *this* emulator
(`src/ppu/mod.rs`) realizes the trickier behaviors — most of them added while chasing the
AccuracyCoin accuracy ROM. Fuller design notes for each live under "Key design notes" in the
project [`CLAUDE.md`](../CLAUDE.md); the AccuracyCoin story is in
[`accuracycoin_outcome.md`](accuracycoin_outcome.md).

- **Register access timing.** The `Bus` gives $2002/$2004/$2007 reads a "pre-advance": it
  ticks the PPU ~8 dots, samples the register on the read's final dot, then ticks 1 more —
  so a read observes the state the real chip would present mid-read, not at the instruction's
  start. `$2002` additionally latches the VBL flag at the *start* of the read cycle but
  re-samples the sprite-0/overflow bits ~2 dots later (`Ppu::resample_sprite_flags`).
- **NMI is a sampled level, not a stored edge.** `nmi = vblank && nmi_enable` is edge-detected
  once per CPU cycle by `SystemClock`, which yields the hardware 2-dot NMI-suppression windows
  around VBL onset and the "flag never sets" race for a $2002 read one dot before VBL.
- **$2007 during rendering uses the glitch increment.** A $2007 read/write while rendering is
  enabled on a visible/pre-render scanline triggers the coarse-X + Y increment pulse
  (`v += $1001`-ish), *not* the PPUCTRL +1/+32 increment (`advance_v_after_ppudata_access`).
- **$2004 during rendering returns the OAM data bus.** Not `oam[oam_addr]` but `oam_buffer`,
  the byte the sprite pipeline last latched that dot ($FF during the dot 1–64 clear window).
- **Sprite evaluation is seeded from OAMADDR.** The per-scanline OAM pointer starts at whatever
  `$2003` holds (not always index 0), which is how "misaligned OAM" and "sprite zero is
  whichever object is examined first" arise. OAMADDR is also forced to 0 during dots 257–320.
- **Sprite units are real down-counter + shifter pipelines**, not X-compares — counting units
  decrement even during forced blank; halted units shift only while rendering is enabled. This
  reproduces "stale sprite" draw behavior.
- **$2001 mask writes have two taps.** The register updates immediately (what the odd-frame
  skip decision samples) but the render pipeline sees the change `MASK_WRITE_DELAY_DOTS` later.
- **Palette RAM** is initialized to the documented 2C02 power-up palette; greyscale zeroes the
  low 4 bits of $2007 palette *reads* only.
- **The PPU reports bus addresses to the mapper** (`notify_ppu_bus` → `Cartridge::ppu_bus_addr`)
  at calibrated dots, driving the MMC3 A12 scanline-IRQ counter and the MMC2/MMC4 CHR latch.

## Implementation Checklist

For a minimal but correct PPU implementation (all implemented here):

- [x] Internal registers: `v`, `t`, `x`, `w`
- [x] Register R/W behavior: `$2000`–`$2007`, `$4014`
- [x] `$2002` side effects: clear VBlank on read, clear `w` on read
- [x] VRAM address space with nametable mirroring (configured by cartridge)
- [x] Pattern table reads (CHR-ROM/RAM via cartridge)
- [x] Scanline counter and dot counter (341 dots × 262 scanlines NTSC)
- [x] VBlank flag: set dot 1 scanline 241, cleared dot 1 scanline 261
- [x] NMI generation (edge detection on VBlank × NMI-enable)
- [x] Pre-render scanline: vertical scroll reload (dots 280–304)
- [x] Horizontal scroll update: `t`→`v` copy at dot 257
- [x] Coarse-X increment every 8 dots during rendering
- [x] Fine-Y / coarse-Y increment at dot 256
- [x] Background tile fetch pipeline (nametable, attribute, CHR low, CHR high)
- [x] Sprite evaluation for next scanline (secondary OAM, 8-sprite limit)
- [x] Sprite pattern fetch and latching
- [x] Sprite-0 hit detection
- [x] Pixel output with priority multiplexer
- [x] Palette RAM (32 bytes) with mirrors
- [x] OAM DMA (`$4014`): 513/514 CPU cycle stall
- [x] Odd-frame dot skip (dot 339 skipped when rendering enabled)
