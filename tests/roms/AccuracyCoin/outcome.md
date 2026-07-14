# AccuracyCoin results

**Status (2026-07-14, branch `worktree-dmc-dma-cycle-accurate`): 105 of the 141 tests
pass**, up from 100 (Session 2) / 91 (develop baseline). `cargo test` remains fully green
(321 passed / 0 failed). (Page 15, "Power On State", is all `DRAW` tests with no pass/fail
verdict and is excluded from the 141.)

## Session 3 (2026-07-14): OAMADDR reset during sprite fetch — the Sprite Zero Hit cluster

Investigated the ~11-test Sprite Zero Hit cluster (Sprite0Hit_Behavior, ArbitrarySpriteZero,
SprOverflow_Behavior, MisalignedOAM_Behavior, Address2004_Behavior, SuddenlyResizeSprite,
OAM_Corruption, AttributesAsTiles, RenderingFlagBehavior, ALE + Read, Hybrid Addresses),
which all shared error code 1 and several literally prerequisite-check "Sprite Zero Hits
should be working". Root cause (found via systematic debugging: instrumented harness runs
dumping PPU mask/OAMADDR/OAM state at the exact cycle window `Sprite0Hit_Behavior` ran —
see the git history of this file for the removed temporary instrumentation): our PPU never
implemented the documented nesdev hardware quirk that **OAMADDR ($2003) is forced to 0
during ticks 257-320 of every visible and pre-render scanline while rendering is enabled**.
`Sprite0Hit_Behavior`'s own setup left OAMADDR at 5 from an earlier register write; without
the reset, its `$4014` OAM DMA (256 bytes, always full-wrapping) still ran correctly, but
started at OAM byte 5 instead of 0 — so "sprite 0" (OAM bytes 0-3, which is what hit
detection and rendering always look at, regardless of where the DMA started) ended up
holding the *wrapped tail* of the source page (still `$FF` filler) instead of the intended
sprite's Y/CHR/Attr/X bytes.

**Fix** (TDD: `oam_addr_resets_to_zero_during_sprite_fetch_when_rendering`,
`oam_addr_untouched_by_sprite_fetch_window_when_rendering_disabled`, and
`oam_dma_after_sprite_fetch_reset_lands_sprite_zero_at_oam_zero` in `src/tests/ppu.rs`,
watched RED before the fix): `Ppu::fetch_sprites`'s existing `dot == 257` branch now also
sets `self.oam_addr = 0`, gated the same way as the rest of sprite fetch/evaluation (only
while `render && is_render_scanline`). `Ppu::oam_addr()`/`Ppu::oam_byte()` were added as
permanent `pub(crate)` test accessors (mirroring the existing `nt0_tile()`).

**Result: 5 tests newly pass cleanly** (Sprite0Hit_Behavior, SprOverflow_Behavior,
AttributesAsTiles, RenderingFlagBehavior, SuddenlyResizeSprite), and 7 more progressed past
their "Sprite Zero Hits should be working" prerequisite into their own next, more specific
sub-check (ArbitrarySpriteZero code 1→2, Address2004_Behavior 1→4, InstructionTiming 6→2,
OAM_Corruption 1→2, Rendering2007Read 1→2, ALERead 1→2, HybridAddresses 1→2) — real
progress, though those 7 still need their own follow-up work. Full `cargo test` stayed
green throughout (318→321, +3 new PPU tests), with **zero regressions** across the whole
blargg suite despite touching sprite-fetch/evaluation code exercised by
`sprite_overflow_tests`, `sprite_hit_tests_2005.10.05`, `oam_read`, and `oam_stress`.

One curiosity, not chased further (out of scope — entangled with the already-documented,
unresolved `Implied Dummy Reads` hang from Session 2): with this fix, a full AccuracyCoin
run now finishes in ~6s instead of the ~70s `MAX_CYCLES` timeout, and ZP `$12`
(`DMASync_PreTest`) reads back `$00` instead of `$01` — both fully deterministic across
repeated runs. This means the OAMADDR fix shifted the exact cycle timing enough to change
*how* the `Implied Dummy Reads` runaway-recursion chaos resolves (apparently now unwinding
back into the ROM's own control flow earlier/differently), not that it fixed or worsened
that bug. `Implied Dummy Reads`, `Branch Dummy Reads`, `JSR Edge Cases`, and
`Internal Data Bus` remain `NOT-RUN` exactly as in Session 2.

## Session 2 (2026-07-14): JSR/RTS/RTI/PLA/PLP silent-cycle fix

The prior session identified (but didn't fix) the root cause of most remaining DMA-cluster
failures: `Apu::dmc_tick_realtime` ticks once per `Bus::read`/`write` call, but JSR, RTS,
RTI, PLA, and PLP each have 1-2 CPU cycles that previously performed **no** bus access at
all (silent cycles), so the realtime DMC clock under-counted elapsed time during
JSR/RTS-heavy code.

**Fix** (TDD: new tests in `src/tests/cpu.rs` — `jsr_bus_access_sequence`,
`rts_bus_access_sequence`, `rti_bus_access_sequence`, `pla_bus_access_sequence`,
`plp_bus_access_sequence`, `pha_bus_access_sequence` — assert the exact
`(address, is_write)` sequence each instruction performs against a new `TestBus.trace`
log; watched RED, then made GREEN in `src/cpu/instructions.rs`):

- **JSR** reordered to match hardware's real cycle table: fetch ADL, dummy read of
  `$0100|SP` (hardware's "predecrement S" cycle), push PCH, push PCL, *then* fetch ADH
  (previously both operand bytes were fetched up front, before the pushes — which also
  left the wrong byte on open bus after JSR completed).
- **RTS** gained two dummy reads: `$0100|SP` before the first pop (hardware's "increment
  S" cycle), and a read of the popped target address itself before the final `+1`
  (hardware's cycle 6 — distinct from the next instruction's own opcode fetch at target+1).
- **RTI** gained the same `$0100|SP` dummy read before its first pop.
- **PLA**/**PLP** gained the same `$0100|SP` dummy read before their pop.
- **PHA**/**PHP** needed no change — already exactly cycle-accurate (verified by a
  regression test, `pha_bus_access_sequence`).
- **BRK** needed no change — its interrupt-sequence micro-ops (`PushPcHi`, `PushPcLo`,
  `PushP`, `VectorFetch`, `VectorFetchHi`, `src/cpu/mod.rs`) already perform one real bus
  access per cycle.

Full `cargo test` stayed green throughout (318/0). AccuracyCoin's own README independently
confirms the JSR reordering was required: "Implied Dummy Reads" error code 4 says *"Or if
your emulator crashes here, the cycles of JSR are in the wrong order"*, and "JSR Edge
Cases" error code 2 says *"JSR should push the return address to the stack between reading
the first and second operand"* — exactly what was fixed.

### Re-swept `DMC_DMA_ALIGNED_PARITY` / `DMC_LOAD_DMA_DELAY`: unchanged

Per the prior session's plan, both constants were re-swept (`DMC_DMA_ALIGNED_PARITY` ∈
{true, false}, `DMC_LOAD_DMA_DELAY` ∈ {1..5}) against the full AccuracyCoin suite now that
the realtime clock no longer drifts on JSR/RTS-heavy code. **Neither constant changed**:
`aligned=false` is decisively worse across the whole sweep (pass count collapses from
~100 to ~77, since it flips the 3-vs-4-cycle stall choice); `aligned=true` with
`delay` ∈ {1,2,3,4} all score identically, and `delay=5` ties the best score but by passing
DMA + $2002 Read's *uncommon* variant ("Load DMA after 3 APU cycles") instead of `delay=3`'s
*common* NES variant ("Load DMA after 2 APU cycles") — so `delay=3` remains the right
choice on hardware-accuracy grounds, not just the score. (The fact that the sweep found no
improvement is itself informative: the silent-cycle fix didn't need recalibration because
it doesn't change any instruction's *total* cycle count, only which of those cycles are
real bus accesses — so it shouldn't have shifted DMA alignment for most code, and indeed it
didn't.)

### Net effect: pass count unchanged (100/141), but composition shifted

Diffing the full results table before/after (same calibration, `aligned=true`/`delay=3`)
against the prior session's baseline:

**Fixed** (as predicted):
- `DMA + $4015 Read`: FAIL code 2 → **pass**.

**Newly broken** — a real regression:
- `DMA + $2007 Write`: pass → **FAIL code 1** ("DMA + $2007 Read did not pass" — a
  prerequisite-check wording; `DMA + $2007 Read` itself still passes standalone, so this is
  the same class of issue as `APU Register Activation`'s known *inline rerun* sensitivity
  below, not a broken prerequisite).

**Newly hanging** — the most significant finding, detailed below:
- `Implied Dummy Reads`: FAIL code 3 → **NOT-RUN** (never completes; the harness times out
  at `MAX_CYCLES` = 1.073B).
- `Branch Dummy Reads`, `JSR Edge Cases`, `Internal Data Bus`: FAIL → **NOT-RUN**, purely as
  collateral — these run *after* `Implied Dummy Reads` in the ROM's fixed test order
  (`Suite_CPUBehavior2`: Instruction Timing, Implied Dummy Reads, Branch Dummy Reads, JSR
  Edge Cases, Internal Data Bus) and the hang prevents the harness from ever reaching them.
  Their own correctness is simply unknown, not regressed.

**Changed but still failing** (different sub-check reached, netural): `Open Bus` (code 4→7),
`APU Register Activation` (code 1→4), `Instruction Timing` (code 2→6), `DMC DMA Bus
Conflicts` (code 2→1, deliberately out of scope), `Implicit`/`Explicit DMA Abort` (code
1→2, deliberately out of scope).

Net: +1 fixed, -1 regressed (2007W), -4 downgraded from clean-fail to hang (1 direct + 3
collateral) = same 100/141 raw score as before, worse composition. **The fix itself is
still correct** — verified independently by the new cycle-exact unit tests, the unchanged
full blargg regression, and AccuracyCoin's own error-code text (quoted above) confirming
the old JSR cycle order was wrong. The hang is a *separate*, deeper, unresolved issue in
how DMC DMA interacts with JSR's now-correct timing during one specific, extraordinarily
exotic test.

### The `Implied Dummy Reads` hang, investigated (not resolved)

`TEST_ImpliedDummyRead`'s "Test 5" (`AccuracyCoin.asm` ~line 11967, the ROM author's own
comment: *"Abandon hope all ye who enter here... the most insane assembly code I have ever
written"*) verifies implied-addressing dummy reads by **executing from open bus**: it
JSRs to an APU register address (e.g. `$4011`/`$4012`), times a DMC DMA to land exactly so
the DMA's fetched sample byte ends up as the "opcode" fetched from that open-bus address,
runs the real opcode under test, whose own T2 dummy-read of `$4015` clears the frame IRQ
flag, then fetches *another* fake opcode from `$4015` — expected to be `$00`(BRK) or
`$40`(RTI) depending on which of 11 opcodes is under test, redirecting through a
`JMP $0600` "software IRQ vector" the test sets up (`$600`=`$4C`,
`$601`/`$602`=target-lo/hi) to a pass/fail handler.

Instrumented debug runs (temporary prints of `cpu.pc`/`sp`/DMC-channel state, removed
before committing) show: once stuck, the DMC channel is legitimately idle
(`loop_flag=false, bytes_remaining=0, sample_buffer=None` — waiting to be re-armed by the
*next* test iteration's `DMASyncWith48`/`WithA5`, which never comes) while the stack
pointer **decreases continuously, wrapping around the 256-byte stack repeatedly**, with
`PC` oscillating through `$0600-$0602` (the `JMP` "vector") for 30M+ cycles before hitting
the cycle cap. This is consistent with the open-bus-fetched "opcode" landing on something
*other* than the expected BRK/RTI — most plausibly another JSR — triggering unbounded
recursion instead of the test's own designed pass/fail path (the ROM author's own
comment on this exact code path: *"if we fail the test, it will probably crash or
something. I don't know."*).

This means the DMA-during-JSR interaction still has *some* residual cycle-level
inaccuracy relative to real hardware, precise enough to matter only in this one
sub-cycle-exact open-bus trick (no other test — including the two DMA-cluster tests that
now newly pass — is sensitive enough to catch it). Not root-caused this session; a
faithful fix likely needs a real-hardware or visual6502-style reference trace of exactly
which read cycle of a JSR that straddles a DMC DMA request gets intercepted, to confirm
whether the new dummy-stack-read cycle (JSR's T3) is a valid DMA-halt point at all, or
whether some other subtlety (open-bus value composition across the halt, DMA-during-a
push-adjacent-cycle) is off by one. Runs that reach this test now take the full ~70s
`MAX_CYCLES` timeout instead of finishing in ~5s.

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

## JSR/RTS silent cycles: FIXED in Session 2 (see above)

This section documented the root-cause analysis before the fix landed; kept for history.
The realtime DMC clock (`Apu::dmc_tick_realtime`) ticks once per `Bus::read`/`write` call,
which assumes every CPU cycle performs a bus access. That assumption was false for JSR (5
bus accesses over 6 cycles), RTS (3 over 6), RTI, PLA, and PLP — all fixed in Session 2.
Proof of the drift this caused: the tick-stamped trace showed the *same* test
(DMA + $2007 Read) locking sync→target in 392 ticks when run standalone (passed) but 393
ticks when re-run inline inside APU Register Activation (failed its pre-check) — identical
instruction stream, one cycle of drift from the different JSR/RTS history. Fixing it
resolved exactly one test cleanly (DMA + $4015 Read) and surfaced a deeper, still-unresolved
DMA-during-JSR timing issue (the `Implied Dummy Reads` hang) — see "Session 2" above.

## Remaining failures (32 failing + 4 hung = 36 non-passing), by cluster

Codes are the ROM's on-screen error codes; meanings from `README.md`. Table regenerated
2026-07-14 at the post-Session-3 state.

### DMC DMA cluster
| Test | Code | Meaning |
|---|---|---|
| APU Register Activation | 4 | Controllers were clocked by the bus conflict with OAM DMA when they shouldn't have been (or vice versa — see README's success-code table for this test). |
| Instruction Timing | 2 | The DMA timing is not accurate enough to test this — progressed past Session 2's code-6 failure once the sprite-fetch OAMADDR fix landed. |
| Delta Modulation Channel | L | Writing $4015 when the DMC timer has 2 cycles until clocked shouldn't trigger the DMA until after the write's 3-4 cycle delay — the load-delay vs timer-edge interaction, finer than the fixed 3-cycle `DMC_LOAD_DMA_DELAY`. |
| Controller Strobing | 4 | Controllers should not be strobed on put→get transitions (needs cycle-accurate $4016 write phase). |
| Controller Clocking | 2 | Reading a strobed controller port shouldn't affect shift register contents. |
| Frame Counter IRQ | 7 | IRQ flag shouldn't clear yet on a get→put transition (frame-counter/$4015-read edge, adjacent to but not the same as the DMA work). |
| DMA + $2007 Write | 1 | Session 2 regression — see its "Newly broken" note; likely the same inline-rerun sensitivity as APU Register Activation. |

### Hung (never reach a verdict) — the `Implied Dummy Reads` chain, unresolved, see Session 2
| Test |
|---|
| Implied Dummy Reads |
| Branch Dummy Reads (collateral — never reached) |
| JSR Edge Cases (collateral — never reached) |
| Internal Data Bus (collateral — never reached) |

### Deliberately out of scope of the DMC-DMA plan (log-only; see plan Global Constraints)
| Test | Code | Meaning |
|---|---|---|
| DMC DMA Bus Conflicts | 1 | APU-register address mirroring during the DMA fetch not modeled. |
| DMC DMA + OAM DMA | 1 | Overlapping-DMA interleaving not modeled (guarded against instead). |
| Explicit DMA Abort | 2 | Mid-stall DMA cancellation not modeled. |
| Implicit DMA Abort | 2 | Mid-stall DMA cancellation not modeled. |

### Sprite Zero Hit cluster — mostly FIXED in Session 3 (see above); 6 items remain
The shared "Sprite Zero Hits should be working" root cause (missing OAMADDR reset during
sprite fetch) is fixed. Five tests now pass outright. The rest progressed to their own
next-level checks — no longer a single shared root cause, back to independent bugs:
| Test | Code | Meaning |
|---|---|---|
| Arbitrary Sprite Zero | 2 | The first processed sprite of a scanline should be treated as "sprite zero" — a sprite-evaluation-order bug, not the OAMADDR one. |
| Misaligned OAM Behavior | 1 | Misaligned OAM should be able to trigger a sprite zero hit (this test's own code-1 prerequisite, unrelated to the fixed one). |
| Address $2004 Behavior | 4 | Reads from $2004 during PPU cycles 1-64 of a visible scanline (rendering enabled) should always read $FF. |
| OAM Corruption | 2 | OAM Corruption should "corrupt" a row in OAM by copying the 8 values from row 0 to another row — a real hardware quirk (stray OAM writes during evaluation) not yet modeled. |
| $2007 read w/ rendering | 2 | A well-timed read from $2007 should be able to affect the PPU address bus during the background read cadence, reading a bit plane from an unintended address. |
| ALE + Read / Hybrid Addresses | 2 | Same class as the $2007-read item above, for $2007/$2006 respectively. |

### Independent smaller items (pre-existing, unrelated to the DMC-DMA/JSR/OAMADDR work)
| Test | Code | Meaning |
|---|---|---|
| Open Bus (page 1) | 7 | PC in open bus should execute from floating data bus values; write cycles should update the bus. |
| SHA $93/$9F, SHS $9B, SHY $9C, SHX $9E | 1 | Target address of the instruction was not correct (see also `blargg_nes_cpu_test5` SHY/SHX note in CLAUDE.md). |
| All NOP instructions | 2 | Opcode $0C (NOP Absolute) malfunctioned. |
| Palette RAM Quirks | 6 | Greyscale mode should zero the low 4 bits of reads. |
| Stale BG / Sprite Shift Registers | 3 / 3 | Shift registers shouldn't clock during H/F-Blank. |
| BG Serial In | 2 | Shift registers should bring in 1s at bit 0. |
| $2004 Stress / $2007 Stress | 2 / 2 | OAMADDR-overflow reads / read-buffer fill timing. |
| 2002 Flag Clear Timing | 1 | Flags weren't cleared on the correct PPU cycle. |

## Calibration constants (all in code; re-swept in Session 2, unchanged)

- `DMC_DMA_ALIGNED_PARITY = true` (`src/bus.rs`) — which live parity gets the 3-cycle
  (vs 4-cycle) stall. `false` was re-confirmed decisively worse (pass count collapses by
  ~23 tests).
- `DMC_LOAD_DMA_DELAY = 3` (`src/bus.rs`) — CPU cycles from a fresh-start $4015 write to
  the first halt-eligible cycle. Re-swept 1-5; only 3 reproduces DMA + $2002 Read's
  documented *common* NES variant ("Load DMA after 2 APU cycles") — `delay=5` ties on raw
  score but only by passing the *uncommon* variant instead, so 3 remains correct on
  hardware-accuracy grounds.
- The realtime reseed anchor (`Apu::dmc_tick_realtime`, `cycle_count & 1 == 1`) — must
  stay consistent with `tick_one`'s channel-clock condition (`cycle_count & 1 == 0`
  post-increment).
