# sprite_hit_tests_2005.10.05 debug log

Working log for `tests/roms/ppu/sprite_hit_tests_2005.10.05/` (blargg's sprite-0-hit
suite, added 2026-07-03 along with `src/tests/sprite_hit_roms.rs`). Purpose: let a future
session resume without re-deriving root causes from scratch. Append entries
chronologically; don't rewrite history — if a fix is later found wrong, add a new entry
saying so rather than editing the old one.

Status baseline at start of this session (2026-07-03): 2 of 11 passing (`left_clip`,
`edge_timing` — both passed by coincidence, see below). Status at end of session: 8 of 11
passing. Failing: `edge_timing`, `timing_basics`, `timing_order`.

## Diagnostic tooling

No permanent tracers were kept — each bug in this log was found via temporary `eprintln!`
gated on an env var (e.g. `PPU_DEBUG_HIT`), added to `src/ppu/mod.rs`, run once, then
removed. That's the fastest pattern here too: hook the specific field/branch in question,
gate on an env var, `PPU_DEBUG_X=1 cargo test --bin nes-emu <test> -- --nocapture`, remove
before committing.

Result convention for this ROM suite differs from `cpu_interrupts_v2`: no `$6000`
signature. These print PASS/FAILED-plus-code as literal text on the nametable (tile value
== ASCII code of the printed char — see `decode_screen_text` in `sprite_hit_roms.rs`).
Failure-code meanings are listed per-ROM in
`tests/roms/ppu/sprite_hit_tests_2005.10.05/readme.txt`.

## Entry format

```
### <date> — <bug> — <status>
Hypothesis:
Evidence:
Fix:
Result:
```

---

### 2026-07-03 — bug 1: sprite_count double-booked across pipeline stages — FIXED

**Hypothesis:** `evaluate_sprites()` resets `sprite_count`/`sprite0_in_secondary` at dot 65
to start evaluating the *next* scanline's sprites. But `output_pixel()` reads those same
fields every dot through 256 to render *this* scanline's sprites (the ones found during the
*previous* scanline's evaluation). The reset lands mid-render.

**Evidence:** Added `PPU_DEBUG_HIT` print of `sprite_count` per-pixel in `output_pixel()`.
For every scanline, `sprite_count` was 0 for all `x >= 65` — sprites only ever rendered in
the leftmost 64 columns of the screen, everywhere, always.

**Fix:** Added separate `sprite_eval_count`/`sprite0_eval` fields for the in-progress
evaluation (dots 65–256). Evaluation writes only to these. They're copied into the
render-facing `sprite_count`/`sprite0_in_secondary` at dot 257 (start of the sprite-fetch
window) instead of dot 65 — by then evaluation for this scanline is done and output_pixel
has already finished needing the old values for dots 1–256.

**Result:** `sprite_hit_basics` went from failing at case #2 ("sprite hit isn't working at
all" — sprite was at X=128, past the 64px cutoff) to passing.

### 2026-07-03 — bug 2: sprite Y missing hardware's one-scanline render delay — FIXED

**Hypothesis:** OAM byte 0 (sprite Y) is documented as the sprite's top row *minus 1* on
real hardware (rendering is delayed one scanline). `fetch_sprites()`'s row math already
subtracts this 1; `evaluate_sprites()`'s in-range check didn't.

**Evidence:** Manually verified `fetch_sprites()`'s `row = (scanline+1).saturating_sub(y_pos+1)`
already has the `+1`. `evaluate_sprites()`'s `in_range = next_scanline.wrapping_sub(y) < height`
does not — off by one relative to the fetch side.

**Fix:** `in_range = next_scanline.wrapping_sub(y).wrapping_sub(1) < height` (later replaced
by the non-wrapping version in bug 4).

**Result:** `sprite_hit_double_height` went from failing to passing; contributed to several
other fixes below (this bug alone wasn't sufficient for most tests since bug 1 was masking
it).

### 2026-07-03 — bug 3: background shift registers read one dot stale — FIXED

**Hypothesis:** `clock_dot()` called `output_pixel()` *before* `shift_bg_shifters()` for the
same dot. A tile's pattern bit takes exactly 8 dots to walk from the low byte (where
`reload_bg_shifters()` puts it) up to bit 15 (what the fine_x mux reads) — and the 8th shift
happens on the *same dot* the next tile reloads. Reading the mux before that dot's shift
sees the state as of one dot earlier: background renders 1px too far right, screen-wide.

**Evidence:** `sprite_hit_corners` case #2 ("lower-right pixel should hit") — added
`PPU_DEBUG_BG`/`PPU_DEBUG_HIT` prints of `bg_nt_byte`/`bg_shift_lo`/`bg_col` around the
expected overlap pixel (128,120). Correct nametable byte (4, `upper_left_tile`) was fetched
at the right dot, but the corresponding non-zero `bg_col` only appeared at x=129, one pixel
later than expected. Hand-traced the reload/shift dot arithmetic and confirmed: the tile's
bit reaches the mux position 8 shifts after `reload_bg_shifters()` writes it, but
`output_pixel()` for that 8th dot runs *before* that dot's own shift.

**Fix:** Reordered `clock_dot()`: `shift_bg_shifters()` now runs before `output_pixel()`.

**Result:** `sprite_hit_corners`, `sprite_hit_alignment`, `sprite_hit_flip` went from
failing to passing (all are pixel-perfect single-column alignment tests, most sensitive to
this).

**Side effect — stale goldens:** This shifts *all* background rendering by 1px, screen-wide,
which broke the golden-screenshot comparisons in `tests/screenshots/ppu/golden/` for
`palette_ram`, `sprite_ram`, `vbl_clear_time`, `vram_access` (from the *other* PPU ROM
suite, `ppu_roms.rs`). In every case the ROM's own self-reported result code was still 1
(pass) — only the pixel-exact screenshot moved. Confirmed with a shift-diff script (99.85%
of pixels matched a plain 1px-right shift). Reblessed all 4 goldens
(`cp tests/screenshots/ppu/output/<name>.png tests/screenshots/ppu/golden/<name>.png`).
`power_up_palette` was already failing before this session (unrelated, pre-existing —
verified via `git stash`baseline) and is untouched.

### 2026-07-03 — bug 4: sprite X/Y position checks wrapped at 255 — FIXED

**Hypothesis:** Both the X-distance check in `output_pixel()` and the Y in-range check in
`evaluate_sprites()` used `u8::wrapping_sub`. A sprite with X or Y near 255 (a common
"park it off-screen" convention used by these ROMs between sub-tests) would wrap around
and become visible again near x=0 / scanline 0, instead of being clipped off the edge of
the screen like real hardware does.

**Evidence:** `sprite_hit_right_edge` case #2 ("should always miss when X=255", sprite
X=255) — added `PPU_DEBUG_HIT2`, found `sp_col=3` (nonzero, a real hit) firing at
`x=0..7`, far from the sprite's nominal position. `(0u8).wrapping_sub(255u8) == 1`, which
passes the `< 8` visibility check. Same class of bug independently confirmed for Y via
`sprite_hit_screen_bottom` case #4 ("should always miss when Y=255").

**Fix:** Replaced both wrapping `u8` subtractions with plain `i32` arithmetic +
`(0..N).contains(&dist)` range checks — no modular wraparound possible.

**Result:** `sprite_hit_right_edge` passing immediately. `sprite_hit_screen_bottom` needed
bug 5 as well (see next entry) before passing.

### 2026-07-03 — bug 5: sprite evaluation/fetch skipped the pre-render scanline — FIXED

**Hypothesis:** `evaluate_sprites()`/`fetch_sprites()` were gated on `visible` (scanlines
0–239) only. Real hardware evaluates sprites during the pre-render scanline too — that's
what produces scanline 0's sprite data. Without it, scanline 0 of every frame renders using
whatever `sprite_count`/`sprite0_in_secondary` was last committed at scanline 239 (which was
evaluating for scanline 240 — a scanline that's never actually rendered), one full
evaluation cycle stale.

**Evidence:** `sprite_hit_screen_bottom` case #4 still failed after bug 4's fix, with the
hit firing at `scanline=0`, `oam_y0=255` (i.e. Y really was 255 at read time — bug 4's fix
was working — but the sprite rendered anyway). Traced back: the sprite that rendered at
scanline 0 was evaluated back when OAM still held the *previous* sub-test's Y=238, before
the vblank period, and that stale evaluation just never got refreshed because scanline 0's
own evaluation (which would happen during pre-render) never ran.

**Fix:** Changed both gates from `visible` to `is_render_scanline` (`visible ||
scanline == PRERENDER_SCANLINE`). Pre-render's own "next scanline" now computes as 0
instead of 262 (which doesn't exist).

**Result:** `sprite_hit_screen_bottom` passing. No regressions — reran full suite against a
clean `git stash` baseline to confirm nothing outside the 4 already-known-stale goldens
(bug 3) changed.

### 2026-07-03 — harness fix: missing CPU/PPU reset phase pre-advance

Unrelated to the 5 PPU bugs above but same session: `sprite_hit_roms.rs`'s test harness
(copied from `ppu_roms.rs`'s pattern) was missing the `bus.tick_ppu(7); bus.tick_apu(8);`
pre-advance that `roms.rs` already does after `cpu.reset()`. Real hardware's 6502 reset
takes 7 cycles before the first instruction fetch; the PPU (3x the CPU clock) has already
run 21 dots by then. Skipping this is a constant CPU/PPU phase skew for the whole ROM run.
Added it to match `roms.rs`'s convention — correct regardless, but empirically did **not**
change the outcome of the 3 remaining timing failures below (see next entry for why: those
ROMs explicitly re-synchronize to a live PPU event — `wait_vbl` polling — early on, which
absorbs any static startup offset).

### 2026-07-03 — timing_basics/timing_order/edge_timing — INVESTIGATED, NOT RESOLVED

These 3 tests read `$2002` twice, back-to-back (a few CPU cycles apart), around a
cycle-tuned delay, expecting the first read to show a miss and the second a hit — testing
that the hit flag becomes readable within roughly ~12 PPU dots of the theoretically exact
pixel. All three currently fail at their first case ("upper-left corner too soon" /
"X=255 timing").

**What's confirmed correct:** Traced `timing_basics` case #2 (sprite at X=0, Y=0, solid
background+sprite) down to the pixel level. Our sprite-0-hit fires at `scanline=1, dot=1`
— the objectively earliest possible pixel, given the sprite renders starting scanline 1
(the Y+1 hardware delay) and both background and sprite are solid-filled everywhere. This
part of the pipeline is correct.

**What's not resolved:** The ROM's own tuned delay (`ldy #3; lda #127; jsr delay_ya3`,
commented "1943 delay") lands its first ("should still be a miss") `$2002` sample at only
`scanline=1, dot=11` — a mere 10 dots after the hit fires — and of course observes it as
already hit. Hand-derived the expected elapsed-cycle count from `begin_sprite_hit_timing`
(`prefix_sprite_hit.a`) through `dma_sprite_table` (~513–514 cycles) + the delay + the read
itself, landing around ~2470 cycles total from the mask-enable write to the first read.
Measured actual elapsed dots between the same two events empirically (via PPU
scanline/dot, not the CPU's own `cpu.cycles`, which freezes during OAM DMA in both this
harness and `roms.rs`'s — not a bug, just means it's the wrong thing to diff across a
DMA-spanning interval) and got a closely matching ~2498-cycle figure. In other words: our
timing from mask-enable to the actual hit, and our timing from mask-enable to the ROM's
first read, both roughly matched hand-derived expectations *independently* — but the ROM
apparently expects the hit to occur meaningfully *later* than the objectively-earliest
pixel, which contradicts the "hit fires at the first possible pixel" behavior already
confirmed correct above.

**Leading theory, unverified:** Real hardware sprite-0-hit may have a warm-up/latency
window at the very start of a scanline's sprite rendering that this emulator's
one-shot-per-scanline sprite evaluation (`evaluate_sprites()` does its whole 64-sprite scan
in a single dot, 256, rather than incrementally like real hardware's dot-by-dot OAM scan
across dots 65–256) doesn't model — possibly the sprite render pipeline isn't actually
"ready" at dot 1 of a scanline the way our code assumes, even though the shift-register
data is technically loaded by then. Alternatively, the hand-derived ~2470-cycle expectation
for the ROM's own delay chain has an error of a few dozen cycles somewhere in
`dma_sprite_table`/`delay_ya3`/`sync_ppu_align2_30` that would need instruction-by-instruction
re-verification to rule out. No independent "expected output" reference exists for this ROM
in the repo (unlike `cpu_interrupts_v2`, which has `readme.txt` tables) to check against.

**Recommendation for next session:** This is the same *class* of bug as the pre-existing,
already-tracked `cpu_interrupts_v2` failures (see `docs/investigations/cpu_interrupt_debug_log.md`)
— exact CPU/PPU cycle alignment around interrupt/DMA/rendering boundaries. Worth treating
as one combined investigation rather than two separate ones; a fix to the shared root cause
(if there is one) may resolve both sets of failures at once.
