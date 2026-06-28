# NES APU (2A03 Audio Processing Unit)

The NES APU is integrated into the Ricoh 2A03 CPU die. It produces audio via five channels —
two pulse waves, one triangle wave, one noise generator, and one delta-modulation channel (DMC)
— mixed into a single analogue output. All registers are memory-mapped into the CPU address
space at $4000–$4017.

## Channel Overview

| Channel | Registers | Generator | Notes |
|---------|-----------|-----------|-------|
| Pulse 1 | $4000–$4003 | Variable-duty square wave | Has sweep; negation is one's complement |
| Pulse 2 | $4004–$4007 | Variable-duty square wave | Has sweep; negation is two's complement |
| Triangle | $4008–$400B | Staircase triangle wave | No volume control; linear counter silences it |
| Noise | $400C–$400F | LFSR pseudo-random | 32,767-step or 93-step sequences |
| DMC | $4010–$4013 | 1-bit delta PCM | Can trigger IRQ or loop sample |

## Shared Components

### Envelope Generator (Pulse 1, Pulse 2, Noise)

Each envelope produces a volume level that either stays constant or decays over time.

```
$4000/$4004/$400C  bit 4   — constant volume flag (CVF)
                   bits 3–0 — volume (CVF=1) or decay period (CVF=0)
```

When CVF is set, the channel outputs the 4-bit volume directly. When clear, the envelope
divider ticks at 240 Hz (every quarter-frame); it counts down from `period` to 0, then
reloads and decrements the decay level (0–15). When decay reaches 0 it either wraps to 15
(loop flag set) or stays at 0 (channel goes silent).

The loop flag (`bit 5` of the same register) doubles as the length-counter halt flag.

Writing to the length-counter/period-high register ($4003/$4007/$400F) immediately
restarts the envelope: the decay level is set to 15 and the divider is reloaded.

### Length Counter (Pulse 1, Pulse 2, Triangle, Noise)

The length counter silences a channel after a programmable time. It is loaded via the
upper 5 bits of the channel's fourth register and clocked at 120 Hz (every half-frame).

```
Upper 5 bits → index into length counter table → 8-bit count
Clocked down 1 per half-frame if halt flag is clear and count > 0.
At 0 the channel is silenced.
```

**Length counter table** (index 0–31, upper 5 bits of the trigger register):

| High nybble →  | $x0  | $x1  | $x2  | $x4  | $x5  | $x6  | $x8  | $x9  | $xA  | $xB  | $xC  | $xD  | $xE  | $xF  |
|----------------|------|------|------|------|------|------|------|------|------|------|------|------|------|------|
| **$0x** | $0A | $FE | $14 | $28 | $50 | $A0 | $3C | $0E | $1A | $0C | $18 | $30 | $60 | $C0 | $48 | $10 |
| **$1x** | $20 | $02 | $04 | $06 | $08 | $0A | $0C | $0E | $10 | $12 | $14 | $16 | $18 | $1A | $1C | $1E |

The second row ($1x) is used by the triangle channel; both rows are used by pulse and noise
depending on the 5-bit index written.

Setting the channel-enable bit in $4015 does not reset the counter; clearing it forces the
counter to 0 immediately.

### Sweep Unit (Pulse 1 and Pulse 2 only)

The sweep unit periodically adjusts the pulse period to create pitch slides.

```
$4001/$4005  eppp nsss
             e — enable sweep
             ppp — divider period (P); the divider clocks every P+1 half-frames
             n — negate flag (0 = add, 1 = subtract)
             sss — shift count
```

Every half-frame clock: compute `target = current_period >> shift`. If `n` is clear,
`target_period = current_period + target`; if set, `target_period = current_period - target`
(Pulse 1 subtracts one extra: one's complement negation; Pulse 2 uses two's complement).

The channel is muted if `current_period < 8` or `target_period > $7FF`. If enable is set,
the divider is non-zero, and neither mute condition applies, the current period is replaced
with `target_period` on each divider clock.

Writing to $4001/$4005 immediately reloads and resets the sweep divider.

---

## Pulse Channels ($4000–$4003 and $4004–$4007)

### Registers

```
$4000/$4004  ddLE VVVV
             dd — duty cycle: 00=12.5%, 01=25%, 10=50%, 11=75% (negated 25%)
             L  — length counter halt / envelope loop
             E  — constant volume flag
             VVVV — volume or envelope period

$4001/$4005  eppp nsss   (see Sweep Unit above)

$4002/$4006  PPPP PPPP   — timer low 8 bits

$4003/$4007  LLLL LPPP
             LLLLL — length counter load index (top 5 bits → table lookup)
             PPP   — timer high 3 bits (period = ($4003[2:0] << 8) | $4002)
```

Writing to $4003/$4007 reloads the length counter, restarts the envelope, and resets the
pulse phase to 0.

### Timer and Frequency

The pulse timer counts down from `period` to 0 at the APU clock rate (CPU / 2). When it
reaches 0 it reloads and steps the 8-step duty sequencer. Output frequency:

```
f = CPU_clock / (16 * (period + 1))
  ≈ 1,789,773 / (16 * (period + 1))  Hz  [NTSC]
```

The channel is silenced when the period is < 8 (too high a frequency), the length counter
is 0, or the sweep mutes it.

### Duty Sequences (8 steps, output is 0 or 1)

| Duty | Sequence |
|------|----------|
| 12.5% | `0 1 0 0 0 0 0 0` |
| 25%   | `0 1 1 0 0 0 0 0` |
| 50%   | `0 1 1 1 1 0 0 0` |
| 75%   | `1 0 0 1 1 1 1 1` |

---

## Triangle Channel ($4008–$400B)

### Registers

```
$4008  CRRR RRRR
       C — control flag (linear counter halt / length counter halt)
       RRRRRRR — linear counter reload value (7 bits)

$4009  (unused, write has no effect)

$400A  PPPP PPPP   — timer low 8 bits

$400B  LLLL LPPP
       LLLLL — length counter load index
       PPP   — timer high 3 bits
```

Writing to $400B sets the linear counter reload flag and reloads the length counter.

### Linear Counter

A second counter specific to the triangle channel. On each quarter-frame clock:
- If the reload flag is set, reload from $4008[6:0].
- Otherwise if the counter is non-zero, decrement it.
- If $4008[7] (control flag) is clear, clear the reload flag.

The channel is silenced when either the linear counter or the length counter is 0.

### Timer and Waveform

The triangle timer counts at the CPU clock rate (not divided by 2). When it reaches 0 it
steps a 32-step waveform sequencer that produces the stairstep triangle shape (values 15
down to 0, then 0 up to 15). Effective frequency:

```
f = CPU_clock / (32 * (period + 1))
```

The triangle has no volume control; its volume is fixed. When the timer period is < 2 the
high-frequency "clicking" effect occurs — this is correct hardware behavior.

---

## Noise Channel ($400C–$400F)

### Registers

```
$400C  --LE VVVV
       L — length counter halt / envelope loop
       E — constant volume flag
       VVVV — volume or envelope period

$400D  (unused)

$400E  M--- PPPP
       M — mode flag (short sequence when set)
       PPPP — period index (into NTSC period table below)

$400F  LLLLL ---
       LLLLL — length counter load index
```

Writing to $400F reloads the length counter and restarts the envelope.

### NTSC Period Table ($400E bits 3–0)

| Index | Timer period |
|-------|-------------|
| 0 | 4 |
| 1 | 8 |
| 2 | 16 |
| 3 | 32 |
| 4 | 64 |
| 5 | 96 |
| 6 | 128 |
| 7 | 160 |
| 8 | 202 |
| 9 | 254 |
| A | 380 |
| B | 508 |
| C | 762 |
| D | 1016 |
| E | 2034 |
| F | 4068 |

### LFSR

The noise generator uses a 15-bit linear feedback shift register (LFSR) initialized to 1.
On each timer clock, the LFSR is shifted right; the incoming bit is the XOR of bit 0 and:

- Bit 1 if mode flag (M) is **clear** → 32,767-step sequence (musical noise)
- Bit 6 if mode flag (M) is **set** → 93-step sequence (harsh metallic noise)

The channel output is 0 (silent) when LFSR bit 0 is 1 or the length counter is 0.

---

## DMC Channel ($4010–$4013)

The DMC plays 1-bit delta-encoded PCM samples stored in CPU address space $C000–$FFFF.
It performs background DMA reads (stealing CPU cycles) to fill a single-byte sample buffer,
then shifts out bits to a 7-bit output level (DAC).

### Registers

```
$4010  IL-- FFFF
       I — IRQ enable
       L — loop flag
       FFFF — rate index (into NTSC rate table below)

$4011  -DDD DDDD
       DDDDDDD — direct DAC load (7-bit output level, written immediately)

$4012  AAAA AAAA
       Sample address: $C000 + (value << 6)   [range $C000–$FFC0]

$4013  LLLL LLLL
       Sample length: (value << 4) + 1         [1–4081 bytes]
```

### NTSC Rate Table ($4010 bits 3–0)

| Index | CPU cycles per output bit |
|-------|--------------------------|
| 0 | 428 |
| 1 | 380 |
| 2 | 340 |
| 3 | 320 |
| 4 | 286 |
| 5 | 254 |
| 6 | 226 |
| 7 | 214 |
| 8 | 190 |
| 9 | 160 |
| A | 142 |
| B | 128 |
| C | 106 |
| D | 84 |
| E | 72 |
| F | 54 |

### DMA and Output

The DMC reader maintains a current address and remaining bytes counter. When the sample
buffer is empty and bytes remain, it stalls the CPU for 4 cycles (or 2–4 depending on
alignment) and reads one byte from the current address. The byte is then shifted out LSB
first; each bit increments or decrements the 7-bit output level by 1 (clamped to 0–127).

When the remaining bytes counter reaches 0:
- If loop flag is set, the address and counter are reloaded from $4012/$4013 and playback
  continues.
- If IRQ flag is set, the DMC IRQ flag is set in $4015 and an IRQ is generated.

Writing a new value to $4011 sets the output level directly and takes effect immediately
(used for software-driven PCM).

---

## Status and Channel Enable ($4015)

### Write ($4015)

```
---D NT21
D — DMC enable
N — Noise enable
T — Triangle enable
2 — Pulse 2 enable
1 — Pulse 1 enable
```

Writing 0 to a channel bit forces its length counter to 0 immediately (silencing it).
Writing 1 allows the length counter to run; it does not reset the counter.
Writing to $4015 always clears the DMC IRQ flag.

### Read ($4015)

```
IF-D NT21
I — DMC IRQ flag (set when DMC sample ends with IRQ enabled)
F — Frame IRQ flag (set by frame counter)
D — DMC active (remaining bytes > 0)
N — Noise length counter > 0
T — Triangle length counter > 0
2 — Pulse 2 length counter > 0
1 — Pulse 1 length counter > 0
```

Reading $4015 clears the frame IRQ flag (F bit) but does **not** clear the DMC IRQ flag.

---

## Frame Counter ($4017)

### Write ($4017)

```
MI-- ----
M — mode: 0 = 4-step, 1 = 5-step
I — IRQ inhibit flag: 1 = disable frame IRQ
```

Writing to $4017 resets the frame counter divider. The reset takes effect 2 or 3 CPU cycles
after the write (on the next odd APU cycle). In 5-step mode, an extra half-frame clock fires
immediately on write.

Setting the IRQ inhibit flag also clears the frame IRQ flag in $4015.

### Step Sequences (NTSC)

Quarter-frame events clock the envelope generators and triangle linear counter.
Half-frame events additionally clock the length counters and sweep units.

**Mode 0 — 4-step sequence (generates IRQ)**

| Step | CPU cycle | Clocks |
|------|-----------|--------|
| 1 | 7,457 | Quarter-frame |
| 2 | 14,913 | Half-frame |
| 3 | 22,371 | Quarter-frame |
| 4 | 29,829 | Half-frame + IRQ |
| (reset) | 29,830 | — sequence restarts |

IRQ is generated at step 4 (and the cycle after) if the inhibit flag is clear. The frame
IRQ flag is set in $4015.

**Mode 1 — 5-step sequence (no IRQ)**

| Step | CPU cycle | Clocks |
|------|-----------|--------|
| 1 | 7,457 | Quarter-frame |
| 2 | 14,913 | Half-frame |
| 3 | 22,371 | Quarter-frame |
| 4 | 29,829 | (no clock) |
| 5 | 37,281 | Half-frame |
| (reset) | 37,282 | — sequence restarts |

> Mode 1 is typically used when the game synchronises audio updates to the PPU NMI; the
> extra step at 37,281 makes the total sequence length slightly longer than one NTSC frame.

---

## Mixer and Output

The five channel outputs are combined before reaching the DAC:

```
pulse_out  = 95.88 / (8128 / (pulse1 + pulse2) + 100)
tnd_out    = 159.79 / (1 / (triangle/8227 + noise/12241 + dmc/22638) + 100)
output     = pulse_out + tnd_out
```

The output value is in the range [0.0, 1.0]. Individual channel volumes are:

- Pulse 1 / Pulse 2: 0–15 (from envelope or constant volume)
- Triangle: fixed at 15 when active (0 when silenced)
- Noise: 0–15 (from envelope)
- DMC: 0–127 (7-bit DAC)

> These are the non-linear mixing lookup formulas from the NESdev wiki. A simpler linear
> approximation (`output = 0.00752*(p1+p2) + 0.00851*tri + 0.00494*noise + 0.00335*dmc`)
> is acceptable for most emulation purposes.

---

## Implementation Notes

### Clocking relationship

The APU runs at half the CPU clock rate. Most timers decrement once per APU clock (every 2
CPU cycles). The triangle timer is the exception — it decrements every CPU cycle.

The frame sequencer fires at CPU cycles relative to the last $4017 write; track elapsed CPU
cycles in `Bus` and compare against the step table above.

### IRQ generation

Frame IRQ fires at step 4 of mode 0 if the inhibit bit is clear. The flag is set in $4015
bit 6 and the IRQ line is asserted. The flag is cleared by reading $4015 or writing $4017.

DMC IRQ fires when the DMA sample exhausts with `I` set in $4010. The flag is set in $4015
bit 7. Cleared by writing $4010 with `I` clear or by writing $4015.

### Sweep negation difference

The two pulse channels differ in how they negate the target period:
- Pulse 1: `target = current - (current >> shift) - 1`  (one's complement)
- Pulse 2: `target = current - (current >> shift)`       (two's complement)

This means that with shift=0 and negate enabled, Pulse 1 slightly undershoots Pulse 2.

### Length counter enable vs. reload

Writing to $4015 enables or disables a channel but does **not** reload its length counter.
The length counter only reloads when the upper register ($4003/$4007/$400B/$400F) is written.
Disabling a channel ($4015 bit → 0) forces the length counter to 0 immediately.

### $4017 write timing

The reset of the frame sequencer does not happen on the same CPU cycle as the write. It
takes effect 2 cycles later (if the write occurs on an even APU cycle) or 3 cycles later
(odd APU cycle). This 2–3 cycle jitter is tested by `cpu_interrupts_v2`.

### DMC DMA stall

When the DMC needs a byte it halts the CPU for 4 cycles. This is implemented in
`Bus::tick_apu()` / `Bus::tick_dma()`: `tick_apu()` arms a 4-cycle stall counter when
`dmc.needs_dma()` is true; the run loop then calls `tick_dma()` instead of `cpu.tick()`
for those cycles, fetching the byte from `dmc.dma_address()` and supplying it via
`dmc.supply_dma_byte()` on the final stall cycle. The 2–4 cycle alignment jitter (fewer
cycles when the stall begins on a `write` cycle) is not currently modelled.
