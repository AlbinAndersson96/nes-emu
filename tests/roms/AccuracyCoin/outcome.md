# AccuracyCoin results

**Status (2026-07-20, branch `claude/standout-issues-next-jplv6h`): 115 of the 141 tests
pass**, up from 113 (Session 7) / 109 (Session 6) / 105 (Session 4) / 100 (Session 2) /
91 (develop baseline). `cargo test` remains fully green (376 passed / 0 failed). (Page
15, "Power On State", is all `DRAW` tests with no pass/fail verdict and is excluded from
the 141.)

## Session 8 (2026-07-20): small-quirk sweep (All NOPs, Palette RAM, BG Serial In)

**Fixed: `All NOPs` (code 2).** The unofficial NOPs ($04/$44/$64, $14/$34/…, $0C) never
performed their target-address read — $0C fetched its operand and stopped, so
`NOP $3AEA` (a $2002 mirror) failed to clear the VBlank flag, which is exactly what the
ROM's Test 2 checks. All three families now do the real read (also closing their silent
cycles — the same 1-bus-access-per-cycle invariant the realtime DMC clock depends on).
The abs,X NOPs already read. TDD: 3 bus-trace tests in `src/tests/cpu.rs`.

**Fixed: `Palette RAM Quirks` (code 6).** Greyscale (PPUMASK bit 0) masked rendered
pixels but not `$2007` palette *reads*; the read arm now masks to $30 when greyscale is
on. Storage and writes stay unmasked (the ROM's code 7 checks that side). TDD:
`greyscale_mode_masks_ppudata_palette_reads`.

**Implemented but test still fails: `BG Serial In` (code 2).** The hardware behavior —
the BG pattern shifters shift 0 into the LOW plane and 1 into the HIGH plane (per the
ROM's own walkthrough; the escaped bits draw as color %10) — is now modeled, and the
full skip-a-reload-via-rendering-disable mechanism is verified end-to-end by a new
regression test (`bg_reload_skip_via_rendering_disable_draws_serial_ones`: skipping the
dot%8==1 reload draws the serial 1s; continuous rendering never exposes them; watching
it fail first also caught that `tick_dots(N)` leaves dot N unprocessed). The remaining
gap is the `$2001` write-apply latency: the ROM's one meaningful iteration (the sprite-0
scanline) assumes hardware's +2-to-+5-dot post-write-cycle delay band, and our apply
lands a few dots earlier. That constant is pinned by `ppu_vbl_nmi/10-even_odd_timing`,
so moving it needs a compensated recalibration of both — deliberately not blind-swept.

**Assessed, not attempted: `Stale BG/Sprite Shift Registers` (both code 3).** The
sprite sub-tests (counters keep clocking during F-Blank, shifters don't; dot-339
counter activation; H-Blank reload suppression) probe a real per-dot down-counter +
shifter sprite pipeline; ours is a static X-compare model (`output_pixel`). Converting
it is a contained but genuine rework of sprite output touching the whole
blargg-verified sprite-hit/overflow surface — a dedicated-session item, and the
prerequisite for both Stale tests (Stale BG's code-3 check also rides on sprite
behavior).

**Downstream shuffle — favorable this time.** The NOP dummy reads shift the realtime
DMC clock phase for everything after the NOP tests, and the DMC-phase-sensitive set
reshuffled: `DMA + Open Bus` (Session-7 collateral) and `DMA + $2007 Write` (a
regression tracked since Session 2) both flipped back to **pass**; `INC $4014` and
`DMA + $4016 Read` are the new victims (plus error-code churn on 4 still-failing DMC
tests: APURegActivation 1→4, DMABusConflict 1→2, DMCDMA+OAMDMA 1→2, ImplicitDMAAbort
2→1). Net +4 −2 = 113 → 115. Same entangled-chaos class as Sessions 4/5/7; the fix
itself (real reads that hardware performs) is unambiguously correct.

## Session 7 (2026-07-20): the unstable-store cluster (SHA/SHS/SHY/SHX)

All five `UnOp_SH*` tests (SHA $93/$9F, SHS $9B, SHY $9C, SHX $9E — shared error code 1,
"target address of the instruction was not correct") fixed by replacing the two wrong
target-address models with the one the ROM's own behavior-detection identifies as
"behavior 1" (the common NES CPU; the ROM distinguishes four manufacturer variants and
we now report success-code "variant 1" on SHA/SHS):

- **Value stored** = `reg & (base_hi + 1)` (reg = A&X for SHA/SHS, X for SHX, Y for SHY;
  SHS also sets SP = A&X). This part was already right.
- **On a page cross, the write address's high byte BECOMES the stored value.** We
  previously wrote to the carried effective address (SHA/SHS) or the pre-carry address
  (SHY/SHX) — neither matches any of the ROM's four known hardware behaviors. The ROM's
  own sub-tests pin this three ways ($1F10-target case → write lands at $0D10/$0510 via
  RAM mirroring, distinguishable from both old models). Note the old pre-carry model had
  been calibrated against `instr_test-v5/07-abs_xy` — which the corrected model passes
  too; that ROM just can't tell the two apart.
- **Unconditional pre-carry dummy read**: these are fixed 5/6-cycle stores; the old code
  used the load-variant addressing helpers, silently skipping the cycle-4 dummy read when
  no page was crossed (a "silent cycle" of exactly the class the Session-2 JSR/RTS fix
  eliminated — one bus access short per execution, which also skews the realtime DMC
  clock).
- **RDY quirk** ("SHY just becomes STY if a DMA occurs on the right cpu cycle", README
  error codes 7-C): if RDY is low during the dummy-read cycle immediately before the
  write, the `& (base_hi + 1)` is dropped and the raw register is stored. Two detection
  legs, both needed: a DMC DMA *serviced on* the dummy read itself (request rose one
  cycle earlier), or a request that *rose during* the dummy read and is still pending
  after it (`Bus::dmc_dma_pending`, new) — in the latter case the CPU doesn't halt until
  its next read, which is *after* the write (writes never halt), but RDY is already low
  at the write and that is what corrupts the value on hardware. The second leg was found
  via `TRACE_DMC_DMA=1`: SHY's sub-test halt landed on the *post-write* instruction fetch
  ($0596 = the RAM function's PHA) while the other four landed on the dummy read —
  same intended RDY timing, ±1-cycle assert phase. A first attempt that instead widened
  the window to include halts on the operand-high fetch was wrong (RDY is high again by
  the write in that case) and did nothing; the pending-check is the faithful model.

TDD: 11 new `src/tests/cpu.rs` tests (page-cross address corruption per opcode, value
formula, the unconditional dummy-read bus trace, and the RDY quirk's three boundary
cases via a purpose-built `DmaOnReadBus`). `cargo test` 358→370/0.

**Bonus: `blargg_nes_cpu_test5/cpu.nes` fixed and un-`#[ignore]`d.** Its long-tracked
"Error 1 on 9C/9E" *and* its supposed "hangs forever on a JAM/KIL opcode" (previously
believed unrunnable-by-design, "on real hardware too") were both symptoms of this same
bug — the wrong-address unstable-store writes smashed the test's own state mid-sweep.
With the fix the full 11-test sweep completes and passes; the Known-gaps entry is
removed.

**Downstream shuffle (same entangled-chaos class as Sessions 4-5):** `DMA + Open Bus`
flipped pass → FAIL code 2 ("DMC DMA on the wrong cycle"), and `APU Register
Activation` moved code 4 → code 1 — which is literally "prerequisite DMA + Open Bus
failed", i.e. one root, its inline rerun of the same test. Nothing in the SH change
touches those code paths; the SH tests now execute far longer in-ROM code paths (the
full behavior-1 suites + their own DMC-arming DMA sub-tests) instead of failing out
early, shifting the cycle phase of everything that runs after them. Net +5 fixed,
-1 shuffled = 109 → 113.

## Session 6 (2026-07-14): DMC DMA cluster — one clean win, one reverted attempt

Worked through the DMC DMA cluster (`Controller Strobing`, `Controller Clocking`, `Frame
Counter IRQ`, and re-confirmed `Delta Modulation Channel`/`APU Register Activation`/
`Instruction Timing`/`DMA + $2007 Write` as out of scope this session).

**Fixed: `Controller Clocking`.** Real hardware continuously reloads a controller's shift
register from the live button state while `$4016` bit 0 is held high — a read during that
window returns the current A-button state directly and never advances (nothing has been
shifted out yet). `Bus`'s `$4016`/`$4017` read arms always shifted unconditionally. Fixed
with a new `controller_strobe: bool` field: while set, reads return
`controller_latch[port] & 1` directly, bypassing the shift register; the shift register is
reloaded on both the strobe-on write (as before) and the strobe-off write (new — captures
whichever value was most recently live during the strobed window, not the stale value from
when strobe first went high). TDD: new `src/tests/bus.rs` test. `cargo test` 331→332/0,
zero regressions. This one was a genuinely clean win — nothing else in the results table
moved.

**Attempted and reverted: `Frame Counter IRQ`'s test G (code 7).** The ROM's own
walkthrough (and suggested implementation) is explicit: reading `$4015` does not clear the
frame IRQ flag synchronously on the read — the clear is deferred to the next "get" cycle
strictly after the read cycle (a put-cycle read defers 1 cycle, a get-cycle read defers 2).
Implemented via a `frame_irq_clear_pending` flag consumed in `Apu::tick_one`, with a
`FRAME_IRQ_CLEAR_GET_PARITY` constant for the untested "which parity is get" question — and
confirmed via temporary unit tests that the deferred-clear mechanism itself was correctly
distinguishable from the old synchronous clear (2 of 3 new tests were meaningful RED/GREEN
differentiators). **But swept both possible values of the parity constant, and both broke
`cpu_interrupts_v2/3-nmi_and_irq`** — one of the most precisely hard-won pieces of this
whole codebase (see `docs/cpu_interrupts.md` and the investigation log it links). Since the
same test failed identically under both parities, this isn't a wrong-constant problem — the
deferred-clear model as implemented has a real interaction with the existing
`APU_READ_PREADVANCE`/interrupt-delivery machinery that I don't understand yet (working
hypothesis: the 4-cycle pre-advance's debt-repayment accounting may not leave enough
*subsequent* ticks within the same instruction for the pending clear to fire before
something else observes stale state — not confirmed). Given the regression risk to
already-blargg-verified interrupt timing, **reverted entirely** rather than force it
through by trial and error. Left for a future session with more room to trace the
interaction properly; not worth attempting without first understanding *why* it breaks,
not just tuning around the symptom.

**Not attempted this session (assessed, deliberately skipped):**
- **`Controller Strobing`** (code 4): requires get/put-cycle-parity awareness for `$4016`
  *writes* specifically — a `DEC $4016` (read-modify-write) performs two writes to the
  register one cycle apart, and whether the resulting 1-cycle strobe pulse "counts" depends
  on which CPU/PPU clock phase the final write lands on. The ROM's own cycle-by-cycle
  walkthrough is detailed but the exact get/put semantics were ambiguous enough on a first
  read that I didn't trust deriving the parity mapping without either a cleaner reference or
  the kind of careful sweep-and-verify process that just went wrong on Frame Counter IRQ.
- **`Delta Modulation Channel`** (code L) and the rest of the remaining DMC DMA cluster
  (`APU Register Activation`, `Instruction Timing`, `DMA + $2007 Write`): all sit in the
  same DMC-DMA-precision territory that's repeatedly shown itself entangled with the
  unresolved `Implied Dummy Reads` chaos across every prior session — not touched this
  session given the fresh caution from the Frame Counter IRQ revert.

## Session 5 (2026-07-14): sprite evaluation seeded from OAMADDR; $2004-during-clear

Two independent fixes, both following up on the Session 3 Sprite Zero Hit cluster work:

**Sprite evaluation now starts from OAMADDR, not always OAM index 0.** Real hardware
seeds its per-scanline sprite-evaluation OAM byte pointer from whatever `$2003` currently
holds — not hardcoded 0 — and walks it byte-by-byte (`+1` per byte while copying an
in-range sprite, `+4` then `& $FC` to skip/realign past an out-of-range one), which is how
"misaligned OAM" and "arbitrary sprite zero" happen on real hardware. `Ppu::evaluate_sprites`
hardcoded its walk to start at `eval_n=0` and treated literal OAM index 0 as "sprite zero"
for hit-detection purposes; both were wrong. Fixed with a new `eval_addr: u8` field (the
actual walked byte pointer, seeded from `oam_addr` at dot 65) driving the non-overflow
evaluation phase, while `eval_n` becomes a pure "objects decided" counter (unrelated to
addressing) whose `==0` check now correctly means "this is the very first decision this
scanline" — matching AccuracyCoin's own walkthrough: "sprite zero" isn't whichever object
lands in secondary OAM slot 0, it's specifically whether the *first-examined* object was
accepted; if it's rejected, no sprite zero exists that scanline at all, regardless of what
a later object finds. The buggy overflow-scan diagonal walk (already correct, blargg-
verified) is untouched — at the moment secondary OAM fills (the 8th accept), `eval_n`/
`eval_m` are reconciled from the just-walked `eval_addr` so the overflow phase picks up
from the right place.

TDD: 3 new `src/tests/ppu.rs` tests (`sprite_evaluation_starts_at_oamaddr_not_always_oam_index_zero`,
`sprite_evaluation_first_examined_object_out_of_range_means_no_sprite_zero_this_scanline`,
`sprite_evaluation_misaligned_oamaddr_walks_byte_by_byte`) — the first two attempts at the
latter two tests turned out to pass even against the *old* buggy code because the test data
didn't distinguish "walks from OAMADDR" from "always starts at 0, but happens to reach the
same bytes anyway"; rewritten with explicit CHR-byte markers that only match under correct
addressing. Fixed `Arbitrary Sprite Zero` and `Misaligned OAM Behavior` cleanly. `cargo test`
328→331/0 stayed green throughout, including all of `sprite_overflow_tests` (the overflow
scan's own blargg-verified behavior).

**`$2004` reads during the secondary-OAM-clear window (dots 1-64) always return `$FF`.**
While rendering, dots 1-64 of every visible/pre-render scanline are spent clearing
secondary OAM to `$FF` — the PPU is internally busy with that write activity, and a
`$2004` read during the window observes it instead of the real byte at OAMADDR.
`Ppu::read_register`'s `4 =>` arm read `oam[oam_addr]` unconditionally. Fixed by checking
`rendering_enabled() && is_render_scanline && (1..=64).contains(&dot)` and returning `$FF`
when true (with the attribute-byte bit-masking correctly skipped in that case too, since
there's no real OAM byte being read). 3 new TDD tests, `cargo test` stayed green.

This fix is verified correct in isolation but did **not** move `Address $2004 Behavior`'s
score (still code 4) — investigated with temporary tracing (removed before committing): its
Test 4 syncs to a precise dot via `VblSync_Plus_A` + a fixed-length OAM DMA + a
cycle-counted `Clockslide`, expecting to land in a visible scanline's dots 1-64, but our
emulator's read actually lands at scanline 261 (pre-render), dot 335 — nowhere near the
1-64 window on any scanline. That's a separate VBlank-sync/OAM-DMA-length precision issue
in the surrounding timing, not a flaw in the `$FF`-return logic itself; not chased further.

**Deliberately not attempted: `OAM Corruption`.** Its own test comments describe a
cycle-alignment-dependent internal "secondary OAM address" register with three different
per-phase increment rules (dots 1-64, 65-256, 257-320) plus a full row-corruption
mechanism triggered by disabling rendering mid-scanline — comparable in scope to the
ALE/octal-latch quirk (`ALE + Read`/`Hybrid Addresses`) already flagged as its own
dedicated effort rather than a quick fix. Skipped for the same reason: high implementation
risk to already-solid sprite-evaluation code, for one test of very narrow real-world
relevance.

## Session 4 (2026-07-14): $2007 access during rendering uses the glitch increment

`$2007 Read w/ Rendering`'s error code 2 is a distinct, well-documented hardware quirk from
the Session 3 OAMADDR fix: a $2007 read *or write* while rendering is enabled, on a visible
or pre-render scanline, does not use the normal `vram_increment()` (+1 or +32 depending on
the PPUCTRL increment-mode bit) at all — it instead triggers the same coarse-X-increment +
Y-increment pulse the background fetch pipeline itself uses on every 8th dot / dot 256,
regardless of which dot the CPU access actually lands on. `Ppu::read_register`/
`write_register`'s `7 =>` arms called `self.v.wrapping_add(self.vram_increment())`
unconditionally — this quirk was simply never implemented.

**Fix**: `Ppu::advance_v_after_ppudata_access()` (a new helper, replacing the inline
`vram_increment()` call at both the read and write `7 =>` sites) checks
`rendering_enabled() && (scanline <= 239 || scanline == PRERENDER_SCANLINE)`; when true it
calls `increment_coarse_x()` + `increment_y()` (the same methods the render pipeline itself
uses) instead of the normal increment. TDD: 4 new `src/tests/ppu.rs` tests —
`ppudata_read_during_rendering_uses_coarse_x_and_y_glitch_increment` and its write/negative
siblings assert exactly `v += $1001` from a no-wrap starting `v` (`+1` coarse X, `+$1000`
fine Y), matching the ROM's own comment ("the correct answer is, v += $1001") to the byte.
Watched RED first. `cargo test` 321→325/0, zero regressions.

**Result**: `$2007 Read w/ Rendering` now passes cleanly. Net AccuracyCoin score stayed at
105/141 — `INC $4014` newly failed (code 2) and `Instruction Timing` reverted to its
Session-2 code 6. Neither test touches `$2007` during rendering (INC4014's Test 2 runs with
rendering explicitly *disabled*), and both sit downstream of the `Implied Dummy Reads`
cluster in the ROM's fixed test order — the same entangled-chaos pattern already seen twice
(Session 3's fix similarly shuffled which downstream tests pass without changing any CPU
cycle count). Not treated as a real regression in this fix; not chased further, consistent
with leaving `Implied Dummy Reads` itself for a dedicated future session.

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

## Remaining failures (22 failing + 4 hung = 26 non-passing), by cluster

Codes are the ROM's on-screen error codes; meanings from `README.md`. Table regenerated
2026-07-20 at the post-Session-8 state.

### DMC DMA cluster
| Test | Code | Meaning |
|---|---|---|
| INC $4014 | 2 | Session 8 phase-shuffle victim (passed since Session 1; also flipped briefly in Session 4) — the same entangled-chaos class as every session's DMC-phase reshuffle. |
| DMA + $4016 Read | 1 | Session 8 phase-shuffle victim (passed as Famicom variant 2 before). |
| APU Register Activation | 4 | Controllers clocked (or not) by the OAM DMA bus conflict — back to its Session-6 code after Session 8's shuffle restored its DMA + Open Bus prerequisite. |
| Instruction Timing | 6 | Cycle counts / DMA data-bus interaction — downstream `Implied Dummy Reads` noise, code has bounced between 2 and 6 across sessions without any change to this cluster's own code. |
| Delta Modulation Channel | L | Writing $4015 when the DMC timer has 2 cycles until clocked shouldn't trigger the DMA until after the write's 3-4 cycle delay — the load-delay vs timer-edge interaction, finer than the fixed 3-cycle `DMC_LOAD_DMA_DELAY`. Not attempted (Session 6) — same territory as the reverted Frame Counter IRQ fix. |
| Controller Strobing | 4 | Controllers should not be strobed on put→get transitions — needs get/put-cycle-parity awareness for `$4016` *writes* specifically (a `DEC $4016` RMW's two same-cycle-adjacent writes, only one of which "counts" depending on clock phase). Assessed but not attempted (Session 6) — see its note above. |
| Frame Counter IRQ | 7 | IRQ flag shouldn't clear yet on a get→put transition. Attempted and reverted (Session 6) — both possible parity values broke `cpu_interrupts_v2/3-nmi_and_irq`; needs the interaction with `APU_READ_PREADVANCE` understood before retrying, not just the constant swept. |

(`DMA + Open Bus` and `DMA + $2007 Write` returned to passing in Session 8's reshuffle.)

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
| DMC DMA Bus Conflicts | 2 | APU-register address mirroring during the DMA fetch not modeled. |
| DMC DMA + OAM DMA | 2 | Overlapping-DMA interleaving not modeled (guarded against instead). |
| Explicit DMA Abort | 2 | Mid-stall DMA cancellation not modeled. |
| Implicit DMA Abort | 1 | Mid-stall DMA cancellation not modeled. |

### Sprite Zero Hit cluster — FIXED in Sessions 3-5, only `OAM Corruption` remains
The shared "Sprite Zero Hits should be working" root cause (missing OAMADDR reset during
sprite fetch, Session 3), `$2007 Read w/ Rendering`'s glitch-increment (Session 4), and
the OAMADDR-seeded sprite evaluation / `$2004`-during-clear bugs (Session 5) are all fixed.
Eight tests now pass outright.
| Test | Code | Meaning |
|---|---|---|
| Address $2004 Behavior | 4 | The `$FF`-during-clear fix (Session 5) is verified correct in isolation, but this test's own VBlank-sync precision lands the read at the wrong dot entirely (scanline 261 dot 335, nowhere near dots 1-64) — a separate timing bug, not chased. |
| OAM Corruption | 2 | Cycle-alignment-dependent internal "secondary OAM address" register + full row-corruption mechanism — deliberately not attempted, comparable complexity/risk to the ALE quirk below. |

### PPU-address-bus-during-background-fetch cluster — NOT the $2007-increment glitch, still open
`ALE + Read` and `Hybrid Addresses` describe a *different* mechanism from the one Session 4
fixed: a well-timed $2007/$2006 access substitutes the CPU-supplied address for the
background pipeline's own next fetch address on that exact dot (reading/fetching from an
"unintended address"), rather than just glitching the post-access increment. Scoped but
deliberately not attempted — see Session 4's assessment (requires a real octal-latch/ALE
bus model, comparable effort/risk to `OAM Corruption` above).
| Test | Code | Meaning |
|---|---|---|
| ALE + Read | 2 | A well-timed read from $2007 should be able to affect the PPU address bus during the background read cadence, reading a bit plane from an unintended address. |
| Hybrid Addresses | 2 | A well-timed write to $2006 should be able to affect the PPU address bus during the background read cadence, performing a nametable fetch from an unintended address. |

### Independent smaller items (pre-existing, unrelated to the DMC-DMA/JSR/OAMADDR/$2007 work)
| Test | Code | Meaning |
|---|---|---|
| Open Bus (page 1) | 7 | PC in open bus should execute from floating data bus values; write cycles should update the bus. |
| Stale BG / Sprite Shift Registers | 3 / 3 | Needs a real per-dot sprite down-counter/shifter pipeline — assessed in Session 8, dedicated-session item. |
| BG Serial In | 2 | Serial-in behavior implemented + regression-tested (Session 8); remaining gap is the $2001 apply-latency band, entangled with the 10-even_odd_timing calibration. |
| $2004 Stress / $2007 Stress | 2 / 2 | OAMADDR-overflow reads / read-buffer fill timing. |
| 2002 Flag Clear Timing | 1 | Flags weren't cleared on the correct PPU cycle. |

(`All NOP instructions` and `Palette RAM Quirks` fixed in Session 8.)

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
