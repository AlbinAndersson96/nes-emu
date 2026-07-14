# AccuracyCoin results

**Status (2026-07-13, branch `worktree-dmc-dma-cycle-accurate` @ `3acd17b`): 100 of the
141 tests pass** (96 plain passes + 4 pass-with-variant), up from 91 on `develop`.
`cargo test` remains fully green (312 passed / 0 failed) at every commit in between.
(Page 15, "Power On State", is all `DRAW` tests with no pass/fail verdict and is excluded
from the 141.)

## How to regenerate these results (no manual runs needed)

The ROM no longer requires manual D-Pad navigation. `src/tests/accuracycoin.rs` boots the
ROM headless, injects a Start press at the menu top (which runs every test), and dumps all
verdicts from their fixed RAM addresses:

```bash
cargo test accuracycoin --release -- --ignored --nocapture
```

- Results live at `$0400-$0492` (one byte per test): `$00` = never ran, `$01` = pass,
  `1 | (n << 2)` = pass with acceptable-variant `n` (the ROM's light-blue codes),
  `(ErrorCode << 2) | 2` = fail, where `ErrorCode` equals the on-screen error code
  documented per-test in `README.md`.
- ZP `$12` (`result_DMADMASync_PreTest`) is the key diagnostic for the whole DMA cluster:
  `$01` = the ROM's precise open-bus DMA sync works, `$02` = it fell back to the
  timing-fragile blargg-2005 sync (every DMA test then reports a generic "wrong cycle"
  code regardless of how the DMA is modeled — this masking is why an entire rework can
  show *zero* movement).
- `TRACE_DMC_DMA=1` makes `Bus::maybe_dmc_dma` log every DMC-DMA halt with the in-flight
  CPU address, stall length, live parity, and an exact wall-cycle tick stamp
  (`Apu::debug_dmc_ticks`) — this is how every root cause below was found.

`outcome.png` is the original hand-run screenshot of the `develop` baseline (93/141 by
manual count; the automated harness counts 91 on the same code — the two runs differ on a
couple of boot/input-timing-sensitive tests such as OAM Corruption). The tables below
supersede the old hand-decoded transcription.

## Fixed on this branch (commits `86da3d9..3acd17b`)

All five were found by tracing, fixed one at a time, and validated by both the harness
and the full blargg regression:

1. **DMC timer period off-by-one** (`src/apu/dmc.rs`): the countdown reloaded
   `NTSC_RATE >> 1` and fired on the 0-tick, making every output bit `NTSC_RATE + 2` CPU
   cycles (448/byte at rate $F instead of hardware's 432). AccuracyCoin's `DMASync`
   hard-codes the 432-cycle fetch spacing, so *every* DMA test failed identically with or
   without cycle-accurate DMA — this single pre-existing bug is why the original
   mid-instruction-DMA rework appeared to have no effect. (Bug still present on
   `develop`.)
2. **Halt lands on the read *after* the buffer-empty assert** (`Bus::maybe_dmc_dma`
   checks `dmc_needs_dma()` *before* advancing the realtime clock): hardware samples RDY
   too late in a cycle to halt the assert cycle itself. Before this fix the halt orbit
   never visited an instruction's data-read cycle (0 of 7,583 traced halts had in-flight
   address `$4000`), so the open-bus sync pre-test could never pass.
3. **Channel-timer clocking anchored to `cycle_count`, not `frame_cycles`**
   (`Apu::tick_one`): `frame_cycles` resets on $4017 writes at arbitrary parity, flipping
   the DMC get/put phase (and the 3-vs-4-cycle stall choice) on ~half of all $4017
   writes — observable as identical-shaped tests (DMA + Open Bus vs DMA + $2007 Read)
   disagreeing.
4. **Stall length decided from live parity** (`Apu::dmc_realtime_current_parity`), not
   `apu.cycle_parity()` which is post-hoc and frozen at instruction start (stale by 0-6
   cycles at a mid-instruction halt). `DMC_DMA_ALIGNED_PARITY = true` calibrated by
   sweeping both values.
5. **Enable-started load DMAs delayed 3 CPU cycles** (`DMC_LOAD_DMA_DELAY`,
   `src/bus.rs`): a $4015 write that starts a fresh sample must not halt the CPU until
   the 4th cycle after the write cycle. Swept 2/3/4; only 3 reproduces DMA + $2002 Read's
   documented "Load DMA after 2 APU cycles" NES variant. Reload DMAs are not delayed.

Newly passing as a result: **DMA + Open Bus, DMA + $2002 Read (variant 1 = NTSC NES),
DMA + $2007 Read, DMA + $2007 Write, DMA + $4016 Read (variant 2 = Famicom), Interrupt
Flag Latency, INC $4014** — plus PPU Read Buffer and Scanline-0 Sprites turned out to be
pass-with-variant all along (the old harness displayed variant bytes as failures).

## Known root cause of most remaining DMA-cluster failures: JSR/RTS silent cycles

The realtime DMC clock (`Apu::dmc_tick_realtime`) is ticked once per `Bus::read`/`write`
call, on the assumption that every CPU cycle performs a bus access. That assumption is
false in `src/cpu/instructions.rs`: **JSR performs 5 bus accesses over 6 cycles and RTS
only 3 over 6** (PHA/PLA/PHP/PLP/RTI/BRK are likely similarly short). Real hardware
performs a bus read on those cycles too (stack peeks / dummy fetches), and RDY can halt
the CPU on them.

Consequences: during JSR/RTS-heavy code (AccuracyCoin's clockslides are nested JSR/RTS,
~40 silent cycles per 432-cycle DMA period) the DMC timer is caught up post-hoc at
instruction boundaries, so a DMA request that should surface mid-JSR surfaces late and
the halt drifts ±1 cycle depending on the surrounding code mix. Proof: the tick-stamped
trace shows the *same* test (DMA + $2007 Read) locking sync→target in 392 ticks when run
standalone (passes) but 393 ticks when re-run inline inside APU Register Activation
(fails its pre-check) — identical instruction stream, one cycle of drift from the
different JSR/RTS history.

**Fix direction for a later session:** model the missing cycles as the real reads
hardware performs (JSR's internal stack cycle, RTS's dummy fetch / stack increment /
PC-increment reads, etc.), so every CPU cycle ticks the realtime clock — then re-sweep
`DMC_DMA_ALIGNED_PARITY` and `DMC_LOAD_DMA_DELAY`, which were calibrated under the
current drift and may shift. Expect this to fix (at least): APU Register Activation,
DMA + $4015 Read, Instruction Timing, Implied Dummy Reads, Internal Data Bus, and
possibly the controller-port tests.

## Remaining failures (41), by cluster

Codes are the ROM's on-screen error codes; meanings from `README.md`.

### DMC DMA cluster — expected to move with the JSR/RTS silent-cycle fix
| Test | Code | Meaning |
|---|---|---|
| APU Register Activation | 1 | Prerequisite check failed — specifically its *inline rerun* of DMA + $2007 Read (`$50`=1 at failure), the proven ±1-cycle drift case. |
| DMA + $4015 Read | 2 | Wrong cycle, or halt cycles didn't read $4015 (should clear frame IRQ flag). |
| Instruction Timing | 2 | Cycle counts / DMA data-bus interaction. |
| Implied Dummy Reads | 3 | Data bus not updated on DMC DMA, or DMA timing off. |
| Internal Data Bus | 2 | Open-bus reads across page boundary / DMC DMA timing. |
| Delta Modulation Channel | L | Writing $4015 when the DMC timer has 2 cycles until clocked shouldn't trigger the DMA until after the write's 3-4 cycle delay — the load-delay vs timer-edge interaction, finer than the fixed 3-cycle `DMC_LOAD_DMA_DELAY`. |
| Controller Strobing | 4 | Controllers should not be strobed on put→get transitions (needs cycle-accurate $4016 write phase). |
| Controller Clocking | 2 | Reading a strobed controller port shouldn't affect shift register contents. |
| Frame Counter IRQ | 7 | IRQ flag shouldn't clear yet on a get→put transition (frame-counter/$4015-read edge, adjacent to but not the same as the DMA work). |

### Deliberately out of scope of the DMC-DMA plan (log-only; see plan Global Constraints)
| Test | Code | Meaning |
|---|---|---|
| DMC DMA Bus Conflicts | 2 | APU-register address mirroring during the DMA fetch not modeled. |
| DMC DMA + OAM DMA | 1 | Overlapping-DMA interleaving not modeled (guarded against instead). |
| Explicit DMA Abort | 1 | Mid-stall DMA cancellation not modeled. |
| Implicit DMA Abort | 1 | Mid-stall DMA cancellation not modeled. |

### Sprite Zero Hit cluster (pre-existing; likely one bug cascading into ~11 failures)
All code 1, and several messages literally read "Sprite Zero Hits should be working":
Sprite 0 Hit behavior, Arbitrary Sprite zero, Sprite overflow behavior, Misaligned OAM
behavior, Address $2004 behavior, Suddenly Resize Sprite, $2007 read w/ rendering,
ALE + Read, Hybrid Addresses, $2002 flag timing (code 1), OAM Corruption (code 1 —
"failed to sync CPU to VBlank at boot"; also the test where manual vs automated runs
disagreed on `develop`).

### Independent smaller items (pre-existing, unchanged by this branch)
| Test | Code | Meaning |
|---|---|---|
| Open Bus (page 1) | 4 | PC in open bus should execute from floating data bus values; write cycles should update the bus. |
| SHA $93/$9F, SHS $9B, SHY $9C, SHX $9E | 1 | Target address of the instruction was not correct (see also `blargg_nes_cpu_test5` SHY/SHX note in CLAUDE.md). |
| All NOP instructions | 2 | Opcode $0C (NOP Absolute) malfunctioned. |
| JSR Edge Cases | 2 | JSR should push the return address between reading operand 1 and operand 2 — directly related to the JSR silent-cycle work above. |
| Branch Dummy Reads | 4 | Cycle 3 of branches should dummy-read the byte after the operand. |
| Palette RAM Quirks | 6 | Greyscale mode should zero the low 4 bits of reads. |
| Rendering Flag Behavior | 2 | BG shift registers should clock when only sprites render. |
| Attributes As Tiles | 1 | Attribute bytes as tile data (scanlines 0-15). |
| Stale BG / Sprite Shift Registers | 3 / 3 | Shift registers shouldn't clock during H/F-Blank. |
| BG Serial In | 2 | Shift registers should bring in 1s at bit 0. |
| $2004 Stress / $2007 Stress | 2 / 2 | OAMADDR-overflow reads / read-buffer fill timing. |
| 2002 Flag Clear Timing | 1 | Flags weren't cleared on the correct PPU cycle. |

## Calibration constants (all in code, all swept — re-sweep after the silent-cycle fix)

- `DMC_DMA_ALIGNED_PARITY = true` (`src/bus.rs`) — which live parity gets the 3-cycle
  (vs 4-cycle) stall.
- `DMC_LOAD_DMA_DELAY = 3` (`src/bus.rs`) — CPU cycles from a fresh-start $4015 write to
  the first halt-eligible cycle.
- The realtime reseed anchor (`Apu::dmc_tick_realtime`, `cycle_count & 1 == 1`) — must
  stay consistent with `tick_one`'s channel-clock condition (`cycle_count & 1 == 0`
  post-increment).
