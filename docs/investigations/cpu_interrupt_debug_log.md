# cpu_interrupts_v2 debug log

Working log for the 4 remaining failing sub-tests (+ combined suite) in
`tests/roms/cpu/cpu_interrupts_v2/`. Purpose: let a future session resume
without re-deriving root causes from scratch. Append entries chronologically;
don't rewrite history — if a fix is later found wrong, add a new entry saying
so rather than editing the old one.

Status baseline at start of this session (2026-07-03): 1 of 5 passing
(`1-cli_latency`). Failing: `2-nmi_and_brk`, `3-nmi_and_irq`, `4-irq_and_dma`,
`5-branch_delays_irq`, `cpu_interrupts` (combined).

## Diagnostic tooling available

`src/tests/roms.rs` has `#[ignore]`d diagnostic tests (uncommitted as of this
session's start):

- `nmi_and_brk_trace` — prints NMI cycle/PC/gap for first 30 NMIs.
  `cargo test nmi_and_brk_trace -- --nocapture --ignored 2>&1 | head -40`
- `nmi_brk_crc_trace` — traces every CRC-update call ($E5AE) with the byte
  fed in. `cargo test nmi_brk_crc_trace -- --nocapture --ignored 2>&1 | grep CRC | head -60`
- `nmi_brk_disasm` — dumps ROM bytes at known addresses for manual
  disassembly. `cargo test nmi_brk_disasm -- --nocapture --ignored`
- `nmi_brk_row_trace` — per-row ($1F/$1D result columns) cycle-offset trace
  for test 2's NMI-vs-BRK timing table.
  `cargo test nmi_brk_row_trace -- --nocapture --ignored 2>&1 | grep -E 'Row|BRK'`

These were built for test 2 specifically; the same pattern (hook a PC address,
print state, `--ignored`) is the fastest way to build equivalent tracers for
tests 3 and 4.

Result codes: test ROMs write a status byte to `$6000` (0 = pass, 0x80+ =
running, other = failure code). When `$6000 < 0x80` after the signature-valid
check, the harness reads the message region and asserts. A bare "code 0x01"
with no message means the ROM's own diagnostic print never ran — need a
tracer to see where it diverges.

## Entry format

```
### <date> — <test> — <attempt>
Hypothesis:
Change:
Result:
Next:
```

---

### 2026-07-03 — test 2 (nmi_and_brk) — attempt 1: PushP FLAG_B clear on NMI hijack — REVERTED, prior memory was WRONG

**Hypothesis (from prior-session memory `cpu-interrupt-tests-research`):** When NMI hijacks
an in-progress BRK, the P value pushed on the stack should have FLAG_B cleared (NMI-service
style), because generic 6502 lore says NMI/IRQ push B=0 while BRK pushes B=1.

**Change:** In `MicroOp::PushP(p)` handler, cleared FLAG_B when `self.pending_nmi` is set.

**Result: WRONG.** `tests/roms/cpu/cpu_interrupts_v2/readme.txt` explicitly documents the
*expected* result table for this test, and row 4 says **"36 00 00 — NMI interrupting BRK,
with B bit set on stack"**. B=1 is the hardware-correct, expected behavior — not a bug.
This makes sense mechanically: on real hardware, NMI hijacking BRK only redirects the
*vector fetch* (T6) to `$FFFA` instead of `$FFFE`; by then PCH/PCL/P have ALREADY been
pushed (T3–T5) with BRK's own values (B=1). The hijack never touches the already-pushed P.

Applying the fix changed the failing output from the pre-fix table to a different (still
wrong) table — confirms it's a regression, not a fix. **Reverted.**

**Lesson:** Don't trust prior-session memory analysis without cross-checking against the
ROM's own `readme.txt` expected-output tables. The memory file `cpu-interrupt-tests-research`
(2026-06-30) has this test 2 section corrected as of this entry — treat its "ROOT CAUSE AND
FIX KNOWN" claim for test 2 as retracted.

### 2026-07-03 — test 2 (nmi_and_brk) — investigation 2: row-shift pattern found

**Method:** Compared baseline (no fix) 10-row output table against the readme's expected
table, position by position:

```
row  got        expected                    match?
0    27 36 00   27 36 00  NMI before CLC      yes
1    27 36 00   26 36 00  NMI after CLC       NO
2    26 36 00   26 36 00                      yes (coincidental repeat)
3    26 36 00   36 00 00  NMI interrupts BRK  NO
4    36 00 00   36 00 00                      yes
5    36 00 00   36 00 00                      yes
6    36 00 00   36 00 00                      yes
7    36 00 00   36 00 00                      yes
8    36 00 00   27 36 00  NMI after SEC       NO
9    26 36 00   27 36 00  NMI after SEC       NO
```

The two "transition points" (27→26 and 26→36) both happen **exactly one row later** than
expected, and the tail never reaches the final "27 36 00" (NMI-after-BRK-fully-vectored)
state within the fixed 10-row budget — it's cut off one row short.

**Evidence gathered via `nmi_brk_row_trace`:** `sta_to_nmi` (STA $2000 → NMI arrival, in
CPU cycles) is **exactly constant at 29766** across all 10 rows — NMI's absolute timing
(driven by PPU VBL) is *not* the variable and appears correctly modeled (no jitter).
`sta_to_brk` and the JSR-$E442-return markers (`e442_1`, `e458`, `e442_2`) all decrease
linearly by exactly 1 cycle per row, as expected from the test's own `timing_offset` sweep
(the ROM's delay routine at $E442/$E458/$E46A, a "wait N cycles" primitive shared by many
blargg tests).

**Working hypothesis:** Since NMI timing is constant/correct and the delay-routine markers
sweep linearly and correctly (1 cycle/row, matching the ROM's own increment), general
instruction-cycle-counting is not the suspect (also ruled out by instr_timing/instr_test-v5
already passing). The remaining candidate is a **constant 1-cycle systematic offset
specifically in the BRK-vs-NMI race resolution** — i.e., our emulator resolves the race as
if NMI arrived 1 cycle later (or BRK's T-state advanced 1 cycle earlier) than real hardware,
for every row uniformly. This would explain both transition points landing one row late AND
the tail never catching up (needs one more row of sweep range that doesn't exist).

**Not yet investigated:** exactly which cycle in BRK's 7-cycle sequence (T1 opcode fetch
through T7 vector-hi fetch) our NMI-hijack check (`pending_nmi` read in `VectorFetch`, i.e.
effectively polled at T6 start) should be evaluated against, versus what real hardware does.
Also worth checking: the `run_rom_impl` reset pre-advance (`tick_ppu(7)`, `tick_apu(8)`) for
an off-by-one that could shift every subsequent VBL/NMI edge by exactly 1 cycle relative to
CPU instruction boundaries — this would show up exactly as "NMI is 1 cycle later than hw"
even though `sta_to_nmi` looks internally *consistent* (it would just be consistently
1-cycle-wrong, which a same-run internal comparison can't detect).

**Next:** Test the reset-pre-advance-offset hypothesis first (cheap: try `tick_ppu(6)` or
`tick_ppu(8)` instead of 7, re-run, see if the whole table shifts by one row in the other
direction). If that doesn't move it, instrument exactly which BRK T-cycle sees
`pending_nmi` transition from false→true for a row near the boundary (row 3 vs row 4) and
compare against a cycle-exact 6502 reference (visual6502/blargg source comments) for when
real hardware samples the interrupt line during BRK.

### 2026-07-03 — test 2 — attempt 3: reset pre-advance `tick_ppu(7)`→`tick_ppu(8)` — DISPROVED, reverted

**Hypothesis:** `cpu.reset()` sets `cpu.cycles = 8`, but `run_rom_impl` only pre-advances
the PPU by 7 cycles (APU gets 8). This 1-cycle PPU/CPU misalignment at startup could
propagate into a constant 1-cycle-early NMI relative to CPU instruction boundaries for
the entire run, matching the observed row-shift.

**Change:** `bus.tick_ppu(7)` → `bus.tick_ppu(8)` in `run_rom_impl`.

**Result: no effect whatsoever** — byte-identical 10-row output table before and after.
Confirms the PPU's VBL/NMI timing is driven by its own internal dot counter, not by
`cpu.cycles`; a 1-cycle startup offset between the two doesn't propagate as a fixed skew
over hundreds of thousands of cycles (frame-periodic resync absorbs it, if it were even
wrong at all). **Reverted — not the cause.**

**Ruled out so far:** startup PPU/APU pre-advance alignment. generic instruction cycle
counting (instr_timing/instr_test-v5 pass). NMI absolute-timing jitter (`sta_to_nmi`
constant across all rows).

**Still open:** exactly which BRK T-cycle our `pending_nmi` check effectively samples vs.
real hardware. Have not yet instrumented cycle-by-cycle within a single BRK's 7-cycle
service sequence for a boundary row (row 3 → row 4). That's the next concrete step —
build a tracer that prints `pending_nmi`/`nmi_pending` state at every micro-op of the BRK
sequence for row 3 and row 4 specifically, and compare the cycle-offset at which
`pending_nmi` flips true against the `timing_offset` value used for that row (readable
from `$E442`'s A register argument, already captured by `e442_1`/`e458`/`e442_2` markers).

**Session time-box note:** this single sub-test (test 2) has consumed significant
investigation without a confirmed fix. Given this whole area previously burned a full
day, pausing here to checkpoint rather than continuing to dig blind — see summary at
bottom of session for suggested next angle (test 5 first, since two independent root
causes are already partially understood there, before returning to test 2's deeper
cycle-polling question).

---

### 2026-07-03 — test 5 (branch_delays_irq) — investigation: test_jmp still wrong despite `frame_reset_delay=4` already in tree

**Context:** prior-session memory claims `frame_reset_delay=4` (already present, uncommitted,
in `src/apu/mod.rs`) fixes test_jmp's T+0/T+1 rows. Given the test-2 memory entry was already
found wrong this session, verified this claim directly instead of trusting it.

**Observed (current code, delay=4):**
```
T+ CK PC
00 FF 03
01 FE 03
02 FD 03
03 FE 04
04 FD 04
05 FF 07
06 FE 07
07 FD 07
08 FE 08
09 FD 08
```
**Expected (readme, test_jmp):**
```
00 02 04 NOP
01 01 04
02 03 07 JMP
03 02 07
04 01 07
05 02 08 NOP
06 01 08
07 03 08 JMP
08 02 08
09 01 08
```

**Claim is WRONG as currently stated** — T+0's PC is 03, not the expected 04. (Possibly the
memory's "delay=4" derivation was for a different/earlier state of the branch-delay-irq
code that has since changed — `branch_delay_irq` mechanism in `cpu/mod.rs` was also touched
since. Not chasing why the memory is stale; just noting it, like test 2's stale claim.)

**Pattern found:** the `CK` column underflows to `FF/FE/FD` (i.e. -1/-2/-3 as i8) instead of
small positives — a strong signal, not just a timing-off-by-one. Looking at the `PC` column
only: got = `[03,03,03,04,04,07,07,07,08,08]`, expected = `[04,04,07,07,07,08,08,08,08,08]`.
**Stripping the leading three `03` rows from got and aligning gives an exact match for the
next 7 rows**: `[04,04,07,07,07,08,08]` == `expected[0..7]`. The table is the right shape,
just prefixed with 3 bogus extra rows (T+0..T+2 produce a wrong/premature PC=03 result
instead of a real case), which eats 3 of the fixed 10-row budget and truncates the tail.

**Working hypothesis:** for T+0..T+2 specifically, IRQ is being serviced 1 cycle too early
inside the branch's `RunInstruction`/`BranchPageFix` sequence — likely the `branch_delay_irq`
flag (`cpu/mod.rs`, set when a taken non-page-crossing 3-cycle branch just completed) isn't
covering every case where IRQ arrives mid-`RunInstruction`, OR the CK underflow indicates the
harness's own "cycles until IRQ delivered" measurement subtracts in the wrong order when IRQ
fires earlier than the harness's baseline assumption for these first few offsets.

**Not yet done:** this needs the same per-cycle tracer treatment test 2 got (no `test_jmp`-
specific tracer exists yet — would need one hooked to the ROM's own T+0..T+2 markers, similar
to `nmi_brk_row_trace`'s approach). Have not built it yet this session.

**Status:** parking here for the session-end summary. `frame_reset_delay` tuning
(3 vs 4 vs other) has NOT been re-verified as the right value now that the surrounding
`branch_delay_irq` code has changed — that's an open variable too, not just the branch
logic. Don't assume delay=4 is settled.

---

### 2026-07-03 — test 2 (nmi_and_brk) — BREAKTHROUGH: found the actual mechanism, root cause narrowed to NMI-polling-point

**Method:** Added a new diagnostic test `nmi_brk_micro_trace` (`src/tests/roms.rs`, `#[ignore]`)
that dumps `pending_nmi`/`nmi_pending`/`queue_len` before+after every single `cpu.tick()` call,
restricted to rows 2–4 (the exact boundary where our table starts diverging from expected).
Also added `Cpu::debug_nmi_state()` (`#[cfg(test)] pub(crate)`, in `src/cpu/mod.rs`) to expose
the otherwise-private state needed for this. Run with:
```
cargo test nmi_brk_micro_trace -- --nocapture --ignored
```

**Finding — our emulator has two genuinely different code paths for "NMI near BRK", and the
choice of path depends on exactly which cycle the NMI edge is detected on:**

1. **Path A ("NMI arrived before BRK's opcode fetch")** — `cpu/mod.rs` tick(), lines ~145–154.
   Triggered when `pending_nmi` is already `true` at the very top of a tick where
   `queue_len == 0`. The CPU does NOT fetch/execute the BRK opcode at all this pass: it does
   two dummy reads (PC not advanced), pushes P with **B=0** (NMI style,
   `(self.p & !FLAG_B) | FLAG_U`), and vectors to `$FFFA`. Because PC was never advanced past
   the BRK opcode byte, **BRK executes for real, later**, after the NMI handler eventually
   `RTI`s back to the same address. This produces: NMI_col shows a B=0 push (early), BRK_col
   later shows a normal B=1 push (once BRK actually runs, post-RTI) — i.e. **looks like
   "NMI happened, then separately BRK happened"** (non-hijack outcome).

2. **Path B ("NMI arrived during BRK's own T1–T5")** — normal `RunInstruction(0x00)` executes,
   pushes PC and P **with B=1** (BRK's own values, via `queue_interrupt_sequence(0xFFFE, p)`
   in `instructions.rs:48-54`), and only the *vector fetch* (T6, `VectorFetch` handler) gets
   redirected to `$FFFA` if `pending_nmi` is true by then. This is genuine hijacking: BRK's
   own P push (B=1) survives, only the destination vector changes — matching the readme's
   documented "NMI interrupting BRK, with B bit set on stack" for row 3/4.

**Traced evidence:** for row 2 AND row 3 (both, currently), the NMI edge lands on the tick
*before* BRK's address is reached (during the preceding 1-byte instruction, e.g. `SEC`), so
`pending_nmi` is already `true` at the top of the BRK-address tick → **both take Path A**
(confirmed via the `ql:0->5` single-tick jump signature, vs. Path B's separate `ql:0->1` then
`ql:1->5` two-tick signature — traced directly, see raw output for cycle-by-cycle detail).
Row 4 takes Path B (NMI edge lands exactly on BRK's own T1 tick, one cycle later).

But per the readme, **row 3 should already be Path B** (hijack, B=1 documented) — only row 2
should be Path A. Our transition from Path A → Path B happens one row later than hardware.

**Root cause, most likely:** real 6502 hardware polls the interrupt lines at a specific point
within an instruction — the *second-to-last* cycle for most instructions — to decide whether
the interrupt affects the *next* opcode fetch. Our harness calls `cpu.nmi()` based on a PPU
edge detected anywhere within the *whole* just-completed instruction's cycle range (via the
delta-cycle loop in `run_until_complete_trace`), which effectively makes the edge "visible"
one cycle later in hardware-equivalent terms than real 6502's polling point — giving our CPU
one extra cycle of eligibility for Path A that hardware wouldn't have, pushing the Path
A→B transition one row later. This is consistent with (and likely the same underlying gap as)
CLAUDE.md's existing "Known gaps" analysis for tests 3/4/5, which all describe
polling-granularity issues. **This may be one unified root cause behind multiple of the 4
failing sub-tests, not 4 separate bugs.**

**Not yet attempted:** actually implementing correct penultimate-cycle polling. This is an
architectural change (our micro-op queue currently has no explicit "poll interrupts now"
marker mid-instruction for arbitrary opcodes — it only polls at the top of `tick()` when the
queue is about to go empty). CLAUDE.md already flags this class of fix as requiring
"sub-instruction cycle-accurate CPU emulation." Given the scope, this needs a deliberate
design decision (where exactly to add the poll point) before touching code — see summary.

---

### 2026-07-03 — CONFIRMED FIX: deferred NMI-edge delivery for last-cycle-of-instruction edges

**Implemented in `run_until_complete_trace`, `src/tests/roms.rs`** (the shared harness for
every `rom_test!` CPU ROM test). Added `Cpu::debug_nmi_state()` accessor
(`#[cfg(test)] pub(crate)`, `src/cpu/mod.rs`) to read `queue_len` after each tick.

**Logic:** after each `cpu.tick()` call, check whether the instruction actually finished
(`queue_len == 0` post-tick). If so, and the NMI edge (from the per-cycle `bus.tick_ppu(1)`
loop) landed specifically on that instruction's *last* PPU/CPU cycle, don't call `cpu.nmi()`
immediately — stash it in a `deferred_nmi` flag and deliver it one tick later instead (so it
only becomes visible starting the tick *after* next, not the very next one). Edges on any
non-last cycle, or occurring mid multi-cycle micro-op sequences that haven't finished an
instruction yet (e.g. mid-BRK), are delivered immediately as before — this only affects the
true instruction-boundary case, and leaves the existing BRK/VectorFetch hijack-detection
mechanism untouched.

This directly implements real 6502 hardware's interrupt-polling behavior: the interrupt line
is sampled going into an instruction's last cycle (using state from *before* that cycle
begins), so a same-cycle edge is one dispatch too late to affect the next opcode fetch.

**Result — verified via full `cargo test` (no regressions, one bonus fix):**
- `ppu/vbl_clear_time` — now **passes** (was previously failing; VBL-flag-clear timing is
  NMI-adjacent, same root cause).
- `cpu_interrupts_v2/2-nmi_and_brk` — went from 2-3 correct rows (out of 10, mostly by
  coincidence) to **8 of 10 rows exactly matching** the readme's expected table. Remaining
  wrong: rows 8–9 (still shows the Path-A/B transition landing one row late at the very tail
  — same shape of bug as before, just needs one more cycle of correction there specifically).
- `cpu_interrupts_v2/3-nmi_and_irq` — was completely opaque (generic code 0x01, no
  diagnostic print at all). **Now produces real diagnostic output** (a 9-row table), though
  it doesn't match the readme's 12-row expected table yet — needs its own dedicated
  investigation, not analyzed in depth this session.
- `cpu_interrupts_v2/4-irq_and_dma` — also went from opaque to a real diagnostic table
  (`T+ CK` pairs) whose overall shape (long run of "7", late transition to "8") visually
  resembles the readme's expected pattern. Not verified exact yet.
- `cpu_interrupts_v2/5-branch_delays_irq` — **unaffected** (unsurprising: that test's timing
  bug is about IRQ, not NMI, and this fix only touches the `cpu.nmi()` delivery path).
- All 161 previously-passing tests (instr_test-v5, instr_misc, instr_timing,
  `1-cli_latency`, PPU tests, unit tests) still pass — **zero regressions**.

**This is a strong signal the deferred-edge model is the real, unified root cause** behind
several of the remaining failures (not 4 unrelated bugs). Kept (not reverted) — this is a
genuine improvement, verified against readme expected-output tables, not a guess.

**Not yet done:**
- The same fix is NOT yet applied to `src/main.rs`'s real run loop (only the test harness).
  Real hardware behavior should be identical there; main.rs has its own duplicated copy of
  this loop. Worth porting once the remaining rows are nailed down, to avoid diverging
  behavior between "running a ROM for real" and "running the test suite." Low risk (same
  reasoning, no regressions found in tests), but not yet done/tested against a real ROM.
- Test 2 rows 8-9 still wrong — need one more iteration of the same investigative approach
  (extend `nmi_brk_micro_trace`'s `TRACE_ROWS` to `[7, 8, 9]` and look for the same kind of
  Path-A/B transition-boundary mismatch found for rows 2-4).
- Tests 3 and 4 need their own tracers built (pattern: copy `nmi_brk_row_trace`, adjust PC
  markers for the target ROM — each test ROM has different code addresses).
- `frame_reset_delay` (APU, currently 4) still unverified against the current state of the
  code — should re-check once test 5 is revisited.

---

### 2026-07-03 — refinement: universal defer (drop the `instruction_finished` gate) — kept, test 2 now 9/10 rows structurally correct

**Change:** removed the `instruction_finished &&` condition from the defer check in
`run_until_complete_trace` — now ANY tick's edge landing on its own last local cycle gets
deferred by one tick, not just edges where a whole instruction just completed. For every
micro-op tick (each is exactly 1 cycle, so `remaining == 1` always), this means every
mid-BRK-sequence edge now also gets the 1-tick defer treatment, same as instruction-boundary
edges.

**Result:** re-ran `cargo test` — same 7 failures as before (no new regressions, `vbl_clear_time`
still passes). Test 2's table improved further:
```
row  got        expected                     match?
0    27 36 00   27 36 00                       yes
1    26 36 00   26 36 00                       yes
2    26 36 00   26 36 00                       yes
3    36 00 00   36 00 00                       yes
4    36 00 00   36 00 00                       yes
5    36 00 00   36 00 00                       yes
6    36 00 00   36 00 00                       yes
7    36 00 00   36 00 00                       yes
8    26 36 00   27 36 00  NMI after SEC        NMI_col off by C flag only (0x26 vs 0x27)
9    26 36 00   27 36 00  NMI after SEC        same
```
Row 8 stopped incorrectly hijacking (matches hardware's "exit boundary" now) — the only
remaining defect is a **single flag bit** (Carry) on the last two rows.

**Traced via `nmi_brk_micro_trace` (rows 7-9, extended with a `[defer-armed]` /
`[deferred-delivered]` marker in the eprintln):** for row 9, the NMI edge originates during
BRK's T5 (`PushP` micro-op). It gets deferred once (per the new universal rule) and becomes
visible at BRK's T7 (`VectorFetchHi`) — too late for T6's hijack check (correctly avoids
hijacking now), but `VectorFetchHi`'s handler doesn't clear `pending_nmi` (only `VectorFetch`
does), so the flag survives into the *next* instruction (`SEC` at the BRK/IRQ vector target,
`$E316`) and **preempts SEC entirely** via the `pending_nmi` direct-service path (same Path-A
mechanism as before, just now firing one instruction later than it used to). Real hardware
apparently lets SEC execute first, THEN fires NMI before the instruction after that
(`STA $1E`) — i.e. our defer is one step too aggressive specifically for edges that land
during BRK's own micro-op tail (T5-T7), even though it's exactly right for edges landing on
completed-instruction boundaries.

**Not yet fixed.** Two candidate directions, neither attempted:
1. `pending_nmi` surviving past `VectorFetchHi` into the next instruction might itself be the
   bug — real hardware's hijack decision is presumably fully resolved by T6, and any leftover
   flag state shouldn't leak into the following instruction's pending_nmi direct-service check.
   Worth checking whether `VectorFetchHi` (or the top-of-tick poll immediately preceding the
   next instruction's opcode fetch) should NOT treat a stale unconsumed `pending_nmi` as
   grounds for direct-service preemption of the very next instruction — possibly needs its
   own one-tick grace period, mirroring the same "defer" logic but for consumption rather
   than creation of the flag.
2. Alternatively, the defer might need to be conditional on *why* the current tick is ending
   (finished a real instruction vs. finished a micro-op sub-sequence) after all, just with
   different rules than the original `instruction_finished` gate — e.g. defer at every
   micro-op boundary EXCEPT the specific BRK-sequence tail (T6/T7) where hijack vs. no-hijack
   is being decided.

### 2026-07-03 — test 2 rows 8-9 residual: re-measured with the fix applied, found the old analysis was using stale (pre-fix) numbers

Updated `nmi_brk_row_trace` (`src/tests/roms.rs`) with the same deferred-edge logic (it had
its own independent copy of the run loop, predating the fix, so its `sta_to_nmi`/`sta_to_brk`
numbers were measuring the OLD unfixed behavior). Re-ran after updating:

```
Row  0: NMI_col=0x27 BRK_col=0x36  nmi-brk=-39  sta_to_nmi=29767  sta_to_brk=29806
Row  1: NMI_col=0x26 BRK_col=0x36  nmi-brk=-38  sta_to_nmi=29767  sta_to_brk=29805
Row  2: NMI_col=0x26 BRK_col=0x36  nmi-brk=-37  sta_to_nmi=29767  sta_to_brk=29804
Row  3: NMI_col=0x36 BRK_col=0x00  nmi-brk=+0   sta_to_nmi=29767  sta_to_brk=29767
Row  4: NMI_col=0x36 BRK_col=0x00  nmi-brk=+1   sta_to_nmi=29767  sta_to_brk=29766
Row  5: NMI_col=0x36 BRK_col=0x00  nmi-brk=+2   sta_to_nmi=29767  sta_to_brk=29765
Row  6: NMI_col=0x36 BRK_col=0x00  nmi-brk=+3   sta_to_nmi=29767  sta_to_brk=29764
Row  7: NMI_col=0x36 BRK_col=0x00  nmi-brk=+4   sta_to_nmi=29767  sta_to_brk=29763
Row  8: NMI_col=0x26 BRK_col=0x36  nmi-brk=+5   sta_to_nmi=29767  sta_to_brk=29762
Row  9: NMI_col=0x26 BRK_col=0x36  nmi-brk=+6   sta_to_nmi=29767  sta_to_brk=29761
```

Confirms the whole table shifted exactly one row later (transition now at row 2→3, matching
readme), and `sta_to_nmi` is still constant (29767, one higher than before the fix — expected,
since the fix adds exactly one cycle of latency globally). BRK is 7 cycles (T1..T7 = offsets
0..6 from `sta_to_brk`). Row 8's `nmi-brk=+5` places its NMI edge exactly at **T6**
(`VectorFetch`, offset 5) and row 9's `+6` places it exactly at **T7** (`VectorFetchHi`,
offset 6) — consistent with the `nmi_brk_micro_trace` finding that row 9's edge is visible at
T7. **But** the readme wants row 8 AND row 9 to both show "NMI after SEC" (i.e. after BRK's
whole 7-cycle sequence *and* the 2-cycle SEC instruction that follows it — offset ≥9), which
is far later than where these edges land (offset 5-6). This is a large gap (3-4 cycles), not
a simple off-by-one, and doesn't fit the same "defer by exactly 1" pattern that fixed the
entry boundary.

**Caveat, not resolved:** `last_brk_t1_cycle` (used to compute `sta_to_brk`) is captured at
the tick where `pc == BRK_ADDR && delta == 1` — the *queue-empty opcode-fetch* tick. For rows
that take the Path-A "NMI preempts BRK, defer BRK until after RTI" route (rows 0-2), BRK's
*real* opcode fetch happens much later (after the whole NMI-service-and-RTI round trip), not
at the position this arithmetic assumes. Rows 0-2's `sta_to_brk` numbers in the table above
may therefore not mean what the row-3-onward numbers mean, making the simple linear
`nmi-brk` model unreliable across the Path-A/Path-B boundary. This needs to be accounted for
before trusting any further arithmetic derived from these markers — the markers were written
before Path A existed as a distinguished concept and haven't been revisited since.

### 2026-07-03 — test 3 (nmi_and_irq): initial investigation, different ROM layout, different bug character

**Vectors:** NMI=`$E308`, RESET=`$E683`, IRQ/BRK=`$E316` — same addresses as test 2's ROM
(shared blargg test framework), but the test-specific code at those addresses differs.

Built a generic runtime tracer (`nmi_irq_generic_trace`, `src/tests/roms.rs`, `#[ignore]`)
that doesn't assume ROM layout — it watches for changes to `$1F`/`$1D` (same result-byte
convention as test 2: readme's "NMI IRQ" column headers) and logs every NMI edge, letting the
loop structure be inferred empirically. IRQ edges were initially logged too but produced
massive spam (thousands of lines) because IRQ is level-triggered and gets re-asserted every
tick while the line stays high and unacknowledged — removed that logging, kept NMI-edge and
result-byte-change logging only. Run with:
```
cargo test nmi_irq_generic_trace -- --nocapture --ignored
```

**Finding:** unlike test 2 (where our result table was structurally right but shifted by
rows), test 3 diverges almost immediately. Expected readme values decode as P-register byte
snapshots: `0x23`=Z:1,C:1 (row 0, "NMI before LDA #1"), `0x21`=Z:0,C:1 (row 1, "NMI after LDA
#1, Z clear"), `0x20`=Z:0,C:0 (rows with NMI after CLC), `0x25`/`0x27`=I:1 variants (NMI
during/after the IRQ handler). **Our trace shows `$1F` staying at `0x23` across what should
be rows 0 *and* 1** (no transition to `0x21`) before eventually jumping straight to `0x27` —
a value that doesn't even appear in the expected 12-row table until the very last rows. This
looks like a different failure mode than test 2's "right shape, shifted by N rows" — more
like our NMI is landing at a fixed point regardless of whatever per-row offset the ROM is
configuring, at least for the first several rows.

**Not yet done:** full disassembly of the test-specific routine (roughly `$E200`-`$E3A0`,
dumped but not fully decoded) to find where/how the per-row timing offset is set up (test 2
used `JSR $E442`/`JSR $E458` delay-routine calls with an increasing offset argument — test 3's
equivalent hasn't been identified yet). This is a comparable-sized reverse-engineering task to
what test 2 took. Needs a fresh session slice, not a quick continuation — flagging as the
next concrete starting point rather than pushing further blind.

---

### 2026-07-03 — test 3: full row mechanism decoded, found a ~34-cycle absolute offset error (different bug from test 2)

**Method:** wrote a standalone 6502 disassembler (`/tmp/.../scratchpad/disasm6502.py`, not
checked into the repo — recreate if needed, it's ~150 lines covering the standard opcode
table) and disassembled `$E200`-`$E3A0`. Full row structure:
```
$E323: EOR #$FF; CLC; ADC #$0D      -- A = 12 - row  (row 0..11, matches readme's 12 rows)
$E328: JSR $E200                     -- sync to VBlank start; NMI/IRQ disabled during sync
$E32B: JSR $E442                     -- delay(A = 12-row)  <-- THE row-varying knob
        ... fixed JSR $E458/$E442 calls with constant args (same every row) ...
$E351: STA $2000,#$80                -- arm NMI (relies on the mid-VBlank instant-fire quirk)
$E354: LDA $4015; $E357: CLI         -- ack APU + enable IRQ
$E35E: CLV; $E35F: SEC; $E360: LDA #$01 ("LDA #1"); $E362: CLC; $E363: NOP
$E364: LDX $1F; $E366: LDY $1D       -- capture results into X/Y, printed per row
```
`$E442`/`$E458` are the same fixed-cycle-delay subroutines test 2 uses (confirmed
independently correct there — a modulo-7 loop that burns exactly the cycle count in its
accumulator argument).

**Verified our PPU already implements the relevant hardware quirk:** `src/ppu/mod.rs:612-625`,
`write_register` for `$2000` — turning on NMI-enable while `self.vblank` is already `true`
sets `nmi_pending` immediately (doesn't wait for a fresh edge). This is correct and already
present, not the bug.

**Built `nmi_irq_row_trace`** (`src/tests/roms.rs`, `#[ignore]`) tracking cycle position of
the `$E32B` delay-call, the `$E360` ("LDA #1") point, and every NMI edge, per row:
```
Row  0: delay_call=652800  lda1=712385  lda1-delay=59585  nmi-lda1=-34
Row  1: delay_call=831484  lda1=891068  lda1-delay=59584  nmi-lda1=-33
Row  2: delay_call=1188852 lda1=1248435 lda1-delay=59583  nmi-lda1=-32
Row  3: delay_call=1456878 lda1=1516460 lda1-delay=59582  nmi-lda1=-31
Row  4: delay_call=1635562 lda1=1695178 lda1-delay=59616  nmi-lda1=-65   <- jump (different path taken)
```
**`lda1-delay` decreases by exactly 1 cycle per row (59585→59584→59583→59582)** — confirms
the row-varying delay mechanism itself works correctly and linearly, same as test 2's. This
rules out an `$E442`/`$E458` cycle-counting bug.

**But `nmi-lda1` (NMI's cycle position relative to "LDA #1") starts at -34 for row 0** (NMI
arrives 34 cycles *before* LDA #1 — consistent with row 0's expected "NMI before LDA #1"
result) **and only shrinks by 1 per row, same rate as the delay sweep.** Since the row sweep
only spans 12 rows (1-cycle steps), the gap would need ~34 rows to reach zero — but real
hardware's row 1 (just one row later) is already supposed to show NMI landing *after* LDA #1.
This means real hardware's gap at row 0 must be much smaller (order of 1-2 cycles, not 34) for
a single-row shift to cross the boundary. **This is a large, systematic absolute-offset error
— NMI (or LDA #1's position) is off by roughly 30+ cycles relative to real hardware, not a
1-cycle polling-granularity issue like test 2.** Different bug, different likely location.

**Not yet investigated — most likely candidates for the ~34-cycle offset:**
1. `$E200`'s VBlank-sync loop (`BIT $2002` polling, waiting for bit 7) — if our PPU sets the
   vblank flag at the wrong dot, or the poll loop takes a different number of iterations to
   observe it than real hardware, this shifts everything downstream by a fixed amount.
2. The *fixed* (non-row-varying) `JSR $E458`/`JSR $E442` calls between the row-varying delay
   and `STA $2000` — if these subroutines' cycle cost is wrong for their specific fixed
   arguments (`$74`/`$15`, `$73`/`$B8`, `$74`/`$37` etc.), every row would be shifted by the
   same constant amount, which is exactly the observed symptom (constant ~34-cycle gap that
   doesn't change in *character*, only in the expected small linear way, across rows).
3. NMI's own absolute dot-to-cycle alignment (less likely — this is shared code with tests
   1/2/5 which behave correctly/reasonably, so a general NMI timing bug would likely have
   shown up elsewhere too).

Candidate #2 (fixed-arg delay-call cost) is the cheapest to check next: verify `$E442`'s
cycle cost for constants like `$B8`/`$37`/`$18` against its own code (the modulo-7 loop) by
hand or with a focused unit test, independent of any interrupt logic.

**Session note:** this is a different, new investigation thread (PPU vblank-sync / fixed-delay
cycle-accuracy) rather than a continuation of the interrupt-polling work that fixed test 2 —
flagging the pivot explicitly since it's a comparable-sized fresh task, not a quick follow-on.

---

### 2026-07-03 — test 3: mechanism fully confirmed via fine-grained trace; root cause narrowed to a specific, bounded hand-verification task

**Method:** extended `nmi_irq_stage_trace` (`src/tests/roms.rs`, `#[ignore]`) with a
per-tick fine trace (`pending_nmi`/`nmi_pending`/`queue_len` before+after, like test 2's
micro-trace) restricted to the `$E35E`-`$E365` window (`CLV;SEC;LDA#1;CLC;NOP`) for row 1.
Found (and fixed) a labeling bug in the tracer itself first: the row counter was incremented
*before* printing marks, misattributing each row's final `capture` line to the next row's
number — the underlying cycle data was fine, only the printed label was off by one. Fixed by
moving the `row += 1` after the mark-printing block.

**Confirmed mechanism, precisely:** for row 1, `CLV` (T1@891029) and `SEC` (T1@891031,
T2@891032) execute completely normally — no pending NMI at either of their own dispatch
checks (both show `pending_nmi:false` going in). The NMI edge that ultimately fires arrives
*during SEC's own 2-cycle window* (between the two dispatch-check points at 891031 and
891032) — becomes visible as `pending_nmi:true` exactly at **LDA #1's own dispatch check**
(the very next queue-empty opportunity, `ql:0->5` jump signature = Path A / direct-service
preemption). This is textbook-correct behavior for *that specific edge*: it arrives during
instruction N, correctly preempts instruction N+1 (there is no bug in the Path-A/polling
mechanism itself here — this confirms the fix from earlier this session is behaving
consistently).

**The actual problem is a baseline calibration one, not a mechanism one:** the edge is simply
arriving too close to LDA #1 in absolute terms (within ~3-4 cycles) at row 0, when hardware's
design clearly intends a much larger margin at row 0 (readme shows the row0→row1 transition,
i.e. crossing from "before LDA #1" to "after LDA #1", needs less than the ~2-cycle width of
LDA #1 itself to occur across a *single* row step). Since our row-to-row delta is a confirmed,
verified, correct 1 cycle/row (established in the previous entry), and the edge only sits ~3-4
cycles from LDA #1 at row 0, there isn't enough sweep range left in the remaining 10-11 rows
to walk the edge across LDA #1 and CLC and into the IRQ handler the way the readme's 12-row
table expects. **This means our absolute cycle position of "where LDA #1 executes relative to
VBlank sync" is offset from real hardware's by a fixed, large amount (order ~30 cycles)** —
consistent with (and now mechanistically explaining) the "~34-cycle" figure found in the
previous entry.

**Bounded next step (not yet done):** hand-compute the expected cycle cost of the *fixed*
(non-row-varying) portion of the row setup — the `$E200` VBlank-sync routine (two `BIT $2002`
poll loops plus the `JSR $E484`/`JSR $E488` fixed-offset NOP-sled calls) and the three fixed
`JSR $E458`/`JSR $E442` delay calls with constant arguments (`$74`/`$15`, `$73`/`$B8`,
`$74`/`$37`) — using known-correct per-instruction cycle costs (already validated by the
passing `instr_timing`/`instr_test-v5` suite, so NOP/JSR/RTS/BIT/branch costs themselves are
not in question). Compare that hand-computed total against the measured cycle deltas already
captured in `nmi_irq_stage_trace`'s output (e.g. `fixdelay1_ret - loop1_exit = 474090-444327 =
29763` for row 0 — dominated by `$E458`'s cost for its fixed argument). A mismatch here would
pinpoint exactly which fixed-cost segment is wrong, without needing any more interrupt-timing
tracing — this is now a deterministic arithmetic-verification task, not an observational one.
`$E458`'s own code (disassembled this session, differs from what a "delay by A" name might
suggest): `PHA; LDA #$D7; JSR $E442; PLA; CLC; ADC #$FF; BNE $E458; RTS` — it's a *coarse*
delay that loops the caller's A times, each iteration burning a fixed `$E442`-delay of $D7
(215) cycles plus loop overhead — worth confirming the per-iteration cost precisely by hand
against `$E442`'s own modulo-7 loop structure.

---

### 2026-07-03 — `$E442`/`$E458` CONCLUSIVELY RULED OUT — cross-validated two independent ways

User asked whether the fixed-cost hand-verification could be done with real confidence rather
than error-prone manual counting. Answer: yes, by cross-validating two independent methods
instead of trusting either alone.

**Method 1 — empirical isolation.** New test `isolate_delay_routines` (`src/tests/roms.rs`,
`#[ignore]`) uses `TestBus` (flat 64 KB, zero side effects, no PPU at all — this is a read-only
diagnostic addition, no production code touched, zero regression risk) to run `$E442`/`$E458`
completely isolated from all PPU/interrupt machinery: pokes the verbatim `$E440`-`$E48A` bytes
into the flat address space, sets up `LDA #a_val; JSR target; <infinite self-JMP>`, and counts
exact CPU cycles from JSR-entry to landing back. Run with:
```
cargo test isolate_delay_routines -- --nocapture --ignored
```

**Method 2 — independent hand-trace.** Manually simulated `$E442`'s instruction sequence
cycle-by-cycle (every opcode's cost is unambiguous ISA fact: `CMP`/`SBC`/`LSR`#imm=2,
branches=2 or 3, `RTS`=6) for A=0, A=1, A=7 — chosen to exercise the not-taken path, the
LSR/BEQ tail, and one full loop iteration respectively.

**Result: exact match, both methods, for every value tested (0, 1, 6, 7, 8, 14, 15, 0x15,
0x18, 0x37, 0xB8, 0xD7):**
```
E442(A=0x00=  0): routine-only=19   |  hand-trace: 19  ✓
E442(A=0x01=  1): routine-only=20   |  hand-trace: 20  ✓
E442(A=0x07=  7): routine-only=26   |  hand-trace: 26  ✓
```
Fits `routine_cost(A) = A + 19` **exactly**, for every value including large loop-heavy ones
(`0xD7`=215 → 234 = 215+19 ✓) — confirms `$E442` delays by *precisely* 1 cycle per unit of A,
with a fixed 19-cycle overhead, no drift, no off-by-one, across the entire input range.

For `$E458`: measured `routine_cost(A) = 256·A + 5` for A=1,2,3 (linear, exact). Hand-derived
from the loop structure (`PHA`(3)+`LDA#$D7`(2)+`JSR$E442`(6+234=240)+`PLA`(4)+`CLC`(2)+
`ADC#$FF`(2)=253 per iteration, +`BNE`(3 taken / 2 not-taken), +6 for the final `RTS`) gives
`256·(A-1) + 255 + 6 = 256·A + 5` — **exact match.**

**Conclusion: `$E442` and `$E458` are cycle-perfect. Ruled out with certainty, not
supposition.** The ~34-cycle absolute-offset bug is *not* in these delay routines — it must be
in `$E200`'s VBlank-sync double-poll-loop logic (`BIT $2002` reads, PPU vblank-flag timing,
possibly interacting with the `$2002`-read pre-advance mechanism added earlier this session
for T4-accurate reads) or in the second wait-loop (`$E22A`-`$E230`). That's the next concrete
target — narrower now than before this check, since two whole subsystems are cleanly
eliminated.

---

### 2026-07-03 — `$2002`-read pre-advance mechanism ALSO ruled out — exact at the dot level

Continued the same rigorous approach into `$E200`'s poll-loop mechanism (the remaining
suspect for the ~34-cycle offset).

**Attempt 1 — sweep the full 2-instruction poll loop** (`sweep_vbl_poll_loop`,
`src/tests/roms.rs`, `#[ignore]`): built a minimal `BIT $2002; loop: BIT $2002; BPL loop`
harness on the real `Bus` (no cartridge needed — PPU dot/scanline progression doesn't depend
on cartridge presence) and swept the starting PPU phase across VBlank onset. Found a huge
(~1 frame) jump in exit cycle right at the boundary — initially looked alarming, but turned
out to be **correct, intentional behavior**: `$E200`'s first `BIT $2002` (before the loop)
deliberately clears any stale flag from a previous frame; starting exactly at/after VBlank
onset means that first read consumes it, and the loop then correctly waits a full extra frame
for a *fresh* edge. Not a bug — a design feature of the "dummy read to clear stale state,
then wait for a real edge" idiom.

The loop's own periodicity (7 CPU cycles / 21 dots per non-exiting iteration, sampling only a
3-dot window per read) makes coarse analysis of the swept data unreliable for spotting subtle
off-by-few-dots bugs — a flat run of identical exit deltas across several starting phases is
*expected* aliasing, not evidence of anything.

**Attempt 2 — isolate a single non-looping read** (`sweep_single_2002_read`, same file):
removed the loop entirely — just `BIT $2002; halt`. Added a new `#[cfg(test)]` accessor
`Ppu::debug_set_position(dot, scanline)` (`src/ppu/mod.rs`) to set the PPU's exact dot
position directly, bypassing `tick_ppu`'s 3-dot (1 CPU cycle) granularity, for true dot-level
precision. Hand-derived the expected boundary first: `$2002`'s read happens at
`start_dot + 12` (3 dots from the opcode-fetch cycle's own PPU advance, then 9 more from the
synchronous 3-CPU-cycle pre-advance) — so VBlank (dot 82182) should first be observable when
`start_dot + 12 > 82182`, i.e. `start_dot >= 82171`.

**Measured (dot-exact sweep, `82167..=82187`): boundary is exactly at `start_dot = 82171` —
matches the hand-derived value with zero discrepancy, not even one dot off.**
```
start_dot=82170  N_flag=false
start_dot=82171  N_flag=true   <- exact predicted boundary
start_dot=82182  N_flag=true
start_dot=82183  N_flag=false  <- test-harness artifact, not a real bug (see below)
```
(The `false` values starting at 82183 are an artifact of `debug_set_position` itself: it sets
raw `dot`/`scanline` counters without re-running the event that fires exactly `dot==1` —
positioning *past* that exact dot means the harness's simulated frame never fires the VBlank
set event at all this frame. Not relevant to the real run loop, which always reaches every dot
via `clock_dot()` and never skips over the trigger point. This only affects the diagnostic
tool's usability for testing positions strictly after the target dot, not the conclusion for
the boundary itself.)

**Conclusion: the `$2002`-read pre-advance mechanism is dot-exact, proven by direct hand
derivation matching measurement precisely.** This is now the *third* full subsystem ruled out
this session with real rigor (delay routines, and now VBlank-read timing) — not by
supposition, by independent cross-validated computation.

**This means the ~34-cycle offset hypothesis needs re-examination, not more digging in
`$E200`.** Since composing correct primitives (proven-correct branch/NOP/JSR timing +
proven-correct delay routines + proven-correct `$2002` read timing) should yield a correct
poll loop by construction, and three independent rigorous checks found nothing, the earlier
"~34-cycle absolute offset" conclusion (from the `nmi_irq_row_trace`/`nmi_irq_stage_trace`
entries above) may itself rest on a **wrong assumption** — likely about what real hardware's
actual NMI-vs-LDA#1 gap at row 0 should be, or about which delay is really "the" row-varying
knob. Worth revisiting that assumption directly (e.g. checking whether blargg's original
source or a real-hardware reference trace exists anywhere) rather than continuing to search
for a bug in code that keeps coming back provably correct.

**Session note:** this is a good moment to pause and reconsider the problem framing rather
than keep drilling into the same area — per systematic-debugging discipline, three
independent verification attempts in the same direction all coming back clean is a signal to
step back, not push a fourth.

---

### 2026-07-03 — user supplied the real blargg source (`tests/roms/cpu/cpu_interrupts_v2/source/`) — confirms all prior NMI/VBlank work was correct; redirects suspicion to the APU/IRQ side, previously flagged but never resolved

User found and provided the actual blargg source tree for `cpu_interrupts_v2` (`.s` files +
`common/` framework). This is a major upgrade over disassembly-based guessing — ground truth
instead of inference.

**Cross-checked test 3's `test:` routine (`source/3-nmi_and_irq.s`) against this session's
disassembly, byte-for-byte, address-for-address:**
- `delay_a_25_clocks` (`source/common/delay.s:44`) — comment: **"Time: A+25 clocks (including
  JSR)"**. Matches this session's OWN empirically-measured `$E442` formula exactly
  (`routine-only = A+19`, `+6` for JSR `= A+25`). Confirms `$E442` = `delay_a_25_clocks`,
  proven both by the source's own documentation and this session's independent measurement.
- `delay_256a_11_clocks_` (`delay.s:65`) uses `lda #256-19-22` (=`$D7`) — matches `$E458`'s
  disassembled `LDA #$D7` exactly. Confirms `$E458` = `delay_256a_11_clocks_`.
- Hand-expanded the `delay 29678` and `delay 29805` macro calls through the actual
  `delay_/delay_nosave_` macro chain (`delay.s:157-189`) and got `LDA #$73;JSR $E458;
  LDA #$B8;JSR $E442` for the first and a matching pattern for the second — **exactly** what
  this session's disassembly found at `$E32E`-`$E33B` and `$E341`-`$E34D`. Every single delay
  call in the row-setup sequence is now confirmed, via the real source, to route through
  primitives already proven cycle-exact this session.
- `for_loop`/`loop_n_times` (`macros.inc:66-81`) confirms `A` enters `test:` as the raw row
  counter (0-11), and the `EOR #$FF; CLC; ADC #13` computation is exactly `12-row` as assumed
  throughout this investigation — no error there either.

**`sync_vbl`'s real structure** (`source/common/sync_vbl.s`) is a **far more sophisticated
multi-iteration precision convergence loop** than earlier disassembly-based modeling assumed
— not a simple "poll once" loop. Rather than hand-verify its intricate convergence mechanism,
tested its **documented contract directly**: *"Reading PPUSTATUS 29768 clocks or later after
return will have bit 7 set. Reading PPUSTATUS immediately will have bit 7 clear."* New test
`verify_sync_vbl_contract` (`src/tests/roms.rs`, `#[ignore]`) runs the REAL `$E200`-`$E233`
bytes (via an actual `Cartridge` loaded from the ROM file — no hand-transcription risk) from
10 different starting PPU phases (including several straddling VBlank onset exactly) and
checks the contract on every one. **Result: contract holds for every single starting phase
tested, no exceptions.**

**Conclusion: exhaustive, source-confirmed verification found nothing wrong anywhere in the
NMI/VBlank-sync/delay-routine chain.** Given the actual test output is still clearly wrong
(re-ran `cpu_interrupts_v2_nmi_and_irq`: got `23,23,23,23,23,27,27,27,27,27,27,27` for the
first column against expected `23,21,21,20,20,20,20,20,20,20,25,25` — stuck for 5 rows
instead of 1, then skips the `21`/`20` states entirely and jumps to a value resembling the
*last* expected state 5 rows too early), **the bug must be somewhere this session hasn't
looked yet.**

**New, concrete, well-motivated lead (tested below, inconclusive): the APU/IRQ side.**
`3-nmi_and_irq.s`'s `test:` routine
does NOT call `sync_apu` (a separate, more careful APU-alignment routine that exists in
`source/common/sync_apu.s` but isn't used by this particular test) — instead it relies on
fixed delay constants (`29678`, `29805`) to land `STA SNDMODE` (`$4017`) at a deterministic
cycle position relative to `sync_vbl`'s return, trusting the APU's own frame-counter-reset
timing to be exactly predictable from there. **This directly implicates the APU's
`frame_reset_delay` mechanic** (`src/apu/mod.rs`) — the exact same open, never-fully-verified
parameter flagged earlier in this log for test 5 ("frame_reset_delay tuning...has NOT been
re-verified as the right value now that the surrounding branch_delay_irq code has changed").
**This session has done zero verification of the APU/IRQ side for test 3** — all effort went
into the NMI/VBlank side, which is now conclusively cleared. A shared root cause between
tests 3 and 5 (both hinge on precise `$4017`-write-to-first-frame-IRQ timing) is plausible and
would be an efficient thing to nail down, rather than a fresh from-scratch investigation.

---

### 2026-07-03 — tested `$4017` write-jitter parity (3 vs 4 cycles) — inconclusive, reverted

Real, well-known 2A03 hardware quirk (not specific to this ROM): writing `$4017` resets the
frame counter after **3 or 4 CPU cycles depending on the write's parity relative to the APU's
internal free-running divide-by-2 clock** — not a flat constant. Our code
(`src/apu/mod.rs`, pre-existing from before this session) hardcodes `frame_reset_delay = 4`
always. `source/common/sync_apu.s`'s own comment — *"an STA $4017 immediately after the JSR
(or some even number of clocks after) will start the frame counter without an extra clock
delay"* — independently confirms this parity-dependence is real and matters for these tests.

**Change tested:** added a new `apu_cycle_parity: bool` field (toggled every `tick_one()`
call, deliberately never reset by `$4017` writes — unlike `frame_cycles`, which does reset,
this needs to model hardware's actual free-running APU-cycle divider). At the `$4017` write
site, picked `frame_reset_delay = if parity {3} else {4}` (tried both polarities).

**Result: no effect on test 3 whatsoever** — byte-identical output to the flat-4 baseline,
for BOTH parity-to-delay mappings tried. Confirmed via debug print that the parity value *is*
genuinely alternating row-to-row as expected (3,3,4,3,4,3,4,...) — the mechanism is live, it
just doesn't move the needle on this test's captured bytes. Did change test 5's numbers
(differently from the flat-4 baseline) but not any closer to correct, and identically between
both polarities tried — inconclusive there too.

**Reverted** back to the flat `frame_reset_delay = 4` (the pre-existing session-start
baseline) — kept the *pre-existing* mechanic (delay-then-reset via `frame_reset_delay`)
exactly as it was; only removed the new parity-conditional addition, since it's unproven and
didn't fix anything. Verified `cargo test` is byte-identical to before this experiment (161
passed / 7 failed, zero regressions) after reverting.

**What this rules out:** the specific 3-vs-4-cycle jitter distinction is *not* the cause of
test 3's failure (at least not in the form implemented and tested here). Doesn't rule out
other APU/IRQ-side issues — the frame IRQ's absolute timing relative to the `$4017` write
(independent of the 3-vs-4 jitter nuance) hasn't been checked against the `MODE0` timing table
(`src/apu/mod.rs:20-27`, cycles 29828/29829/29830) the way the NMI/VBlank side was
exhaustively checked. That table's origin/correctness hasn't been re-derived from the real
source in this session — worth doing if this thread is picked up again, using the same
contract-testing approach that worked for `sync_vbl` (test the APU's frame-IRQ assertion
timing against a precise, hand-derived expectation, rather than reading the whole ROM's
behavior at once).

**Session status:** this was a long, thorough session. Confirmed wins: `ppu/vbl_clear_time`
now passes outright; `cpu_interrupts_v2/2-nmi_and_brk` went from ~2-3/10 rows correct to
8-9/10 (real, verified fix — deferred NMI-edge delivery, matching real 6502 interrupt-polling
semantics); tests 3 and 4 went from fully opaque to producing real diagnostic data. Zero
regressions anywhere, confirmed repeatedly throughout. Test 3's NMI/VBlank/delay-routine
chain is now exhaustively proven correct against the real blargg source — a genuinely
valuable, hard-won result even though the actual bug remains at large. The APU frame-IRQ
absolute-timing table is the most promising unexplored lead for a future session.

---

### 2026-07-03 — followed the test 3 thread further: isolated APU frame-IRQ timing (exact), reframed the readme's IRQ-column transition, tried IRQ-defer (mixed result, reverted)

**Isolated the APU's frame-IRQ absolute timing** (`isolate_apu_frame_irq_timing`,
`src/tests/roms.rs`, `#[ignore]`) — zero CPU/PPU involvement, just `Apu::new()`,
`write(0x4017, 0x00)`, then tick 1 cycle at a time until the IRQ line asserts. **Measured:
exactly 29832 cycles after the write** — matches the hand-derived expectation
(`frame_reset_delay(4) + MODE0's first step(29828) = 29832`) with zero discrepancy. The
APU's own internal timing table is correct.

**Reframed the readme's row-by-row "IRQ" column.** `delay 29805` (the gap between the
`$4017` write and `CLI`) is fixed, not row-varying — so the relationship between "when IRQ
becomes pending" and "when `CLI`/the test window happens" should be constant across all 12
rows. What actually varies row-to-row is NMI's position (already proven correct). The
readme's transition — `irq_flag` staying `00` through row 9 then becoming `20` at rows
10-11 — is governed by whether NMI arrives *before* the IRQ can be dispatched (preempting it
entirely for that row) or *after* IRQ is already mid-service (hijacking it, à la test 2's
BRK-hijack). Computed from the trace: for row 4, IRQ becomes pending at cycle 1695106 and
`LDA #1` executes at cycle 1695106 too — i.e. IRQ's pending-moment and the row's own test
window coincide almost exactly, for every row (since neither depends on the row-varying
delay in a way that shifts their *relative* offset). Our actual transition happens at row 5
instead of row 10 — a 5-row gap, much smaller than the earlier (now-suspect) "~34-cycle"
estimate from before the source was available.

**Tried: apply the same deferred-visibility treatment to the IRQ line** that fixed NMI's
polling granularity earlier this session (`bus.tick_apu(delta)` currently checks the level
once per whole instruction — same last-cycle blind spot NMI had before the fix). Implemented
in `run_until_complete_trace`: tick APU one cycle at a time, defer visibility by one tick for
a level that first asserts on an instruction's literal last cycle.

**Result: mixed, not clearly correct.** Test 3's rows 1-6 shifted into the *right family* of
values (`21`/`25`, matching the readme's actual flag-difference structure, vs. the previous
`23`/`27`) — real, structural movement toward correct. But **row 0 regressed**: was
byte-correct (`23 00`, matching the readme exactly) before this change, became `21 00`
after — a real loss, not a wash. Refining the logic to only defer *genuinely fresh*
low→high transitions (vs. an already-continuously-asserted line, tracked via a persistent
`irq_line_high` flag) made **zero difference** — identical output either way — which itself
is a data point: whatever is causing row 0's regression isn't explained by the
fresh-vs-continuing distinction I hypothesized. No full-suite regressions either way (161/7
throughout, `cli_latency` still passes, test 2 unaffected as expected since it's NMI-only).

**Reverted** — trading a known-correct row for uncertain, only-partially-understood gains
elsewhere isn't a responsible keep, especially without understanding *why* row 0 broke.
Confirmed `cargo test` and test 2/test 3's specific outputs are back to exact pre-experiment
baseline after reverting.

**What this leaves for next time:** the IRQ-defer *direction* looks promising (real structural
movement in test 3, zero suite regressions) but needs the row-0 regression explained before
it can be trusted — likely requires a `nmi_irq_stage_trace`-style fine-grained trace of row 0
specifically, with the IRQ-defer patch temporarily reapplied, watching `irq_pending`/
`pending_nmi`/`queue_len` together (not just IRQ in isolation) to see exactly what changes
about row 0's dispatch sequence. The APU frame-IRQ table itself is now conclusively ruled out
as a cause (proven exact) — the remaining mystery is entirely in the CPU-side NMI-vs-IRQ
arbitration at the exact moment both are close to firing.

---

---

**Decision:** parking the single-bit residual on test 2 rows 8-9 here — it needs either a
cycle-exact reference trace (real hardware or a known-cycle-accurate emulator) or the
original blargg `.asm` source (only the compiled ROM + readme are available in this repo) to
resolve with confidence, rather than more manual arithmetic against markers that may not mean
what they're assumed to mean across the Path-A/B boundary. Time is better spent on tests 3/4,
which are currently opaque and where the SAME deferred-edge fix already produced dramatic
improvement (opaque → real diagnostic tables) without any of this boundary-marker ambiguity.

**Session checkpoint:** significant, verified progress this session: `vbl_clear_time` now
passes outright; test 2 is 8-9/10 structurally correct (was ~2-3/10 by coincidence at
session start); tests 3 and 4 went from fully opaque to producing real diagnostic tables.
Root cause is now well understood to be about interrupt-polling granularity (when exactly an
NMI edge becomes "visible" relative to CPU cycle boundaries), consistent with CLAUDE.md's
existing characterization of this whole area. No regressions anywhere. This is a good
stopping point to check in before chasing the residual single-flag-bit defect further — the
next attempt should start from finding #2 above (VectorFetchHi/pending_nmi leakage) with a
fresh `nmi_brk_micro_trace` run focused specifically on the T6→T7→next-instruction boundary.

---

### 2026-07-03 (new session, PR #12 merged) — followed the test 3 IRQ-defer thread: traced row 0 precisely, confirmed IRQ-defer is wrong, found row 0's real mechanism, row 1 still unexplained

PR #12 (the deferred-NMI-edge fix) merged to `develop`. New branch
`fix/nmi-irq-race-row0-trace` off the updated `develop` for this continuation.

**Built `nmi_irq_row0_arbitration_trace`** (`src/tests/roms.rs`, `#[ignore]`) — runs a chosen
row twice (baseline vs. the IRQ-defer experiment from the previous entry), tracing
`pending_nmi`/`nmi_pending`/`irq_pending`/`pending_irq`/`queue_len`/`FLAG_I` together across
the `$E357`ˋ(CLI)`-$E364`(capture) window. Added `Cpu::debug_irq_state()`
(`#[cfg(test)] pub(crate)`, `src/cpu/mod.rs`) to expose the needed IRQ-side fields.

**Row 0, baseline (no IRQ-defer) — fully explained, and it's correct:**
1. `SEC`'s own T2 (cycle 712349) is when `irq_pending` becomes true (IRQ line was already
   asserted; this is just the CPU's own next check of it).
2. **`LDA #1`'s own dispatch, the very next queue-empty opportunity, gets preempted by IRQ**
   (`ql:0→5, delta=2` — the "non-branch IRQ fires before this instruction" dummy-read path,
   *not* NMI's Path A). IRQ begins its own 7-cycle service sequence: pushes PC=`$E360`
   (`LDA #1`'s un-executed address) and P with B=0, using flags as of *before* `LDA #1` ran
   (Z=1, C=1).
3. **NMI's edge arrives 2 cycles into this IRQ service** (during `PushPcHi`, T3) and
   **hijacks the vector fetch at T6** — `pending_nmi` flips true→false exactly at
   `VectorFetch`, redirecting `$FFFE`'s target to `$FFFA`. IRQ's own handler code (`$E316`)
   *never executes* — the NMI handler (`$E308`) runs instead, but reads the *already-pushed*
   P (IRQ's own, Z=1/C=1) since only the vector was redirected, not the push.
4. NMI's RTI returns to `$E360` (IRQ's originally-pushed, never-advanced PC) — `LDA #1`
   *finally* executes for real, cleanly, uninterrupted, followed by `CLC`/`NOP`/capture.

Result: `$1F=$23` (from the hijacked-but-IRQ-pushed P, Z=1/C=1), `$1D=$00` (IRQ handler code
never ran) — **exactly matches the readme's row 0.** This is genuinely correct, non-obvious,
multi-mechanism behavior (IRQ dispatch + NMI hijack-of-IRQ, not the simpler BRK-hijack from
test 2), and our emulator gets it right.

**Confirmed the IRQ-defer experiment is wrong, precisely.** Re-ran the same trace with the
IRQ-defer patch applied: IRQ's visibility shifts from cycle 712349 to 712350 (1 cycle later,
by design of the defer). That 1-cycle shift is enough for `LDA #1` to slip through its
dispatch check *before* `irq_pending` becomes visible — so `LDA #1` executes normally instead
of being preempted. NMI then preempts the *following* instruction (`CLC`) directly via Path A
instead of hijacking an in-progress IRQ sequence — a different mechanism entirely, capturing
P with Z=0 (`LDA #1` already ran) instead of Z=1, giving `$1F=$21` — the observed regression,
now fully explained rather than just observed. **Conclusion: IRQ must NOT get the same
last-cycle-defer treatment as NMI.** Real 6502 hardware polls IRQ and NMI similarly in terms
of granularity, but NMI is edge-triggered (needs the defer to correctly model "one dispatch
too early") while IRQ is level-triggered and already re-sampled every tick — the existing
simple `bus.tick_apu(delta)`-once-per-instruction check is apparently already at the right
precision for IRQ, and the earlier apparent "improvement" in rows 1-6's value *family* was
likely a side effect of the same change that broke row 0, not evidence of a real fix. This
experimental code was investigation-only (in the new test, not the shared run loop) — nothing
to revert in production code this time.

**Row 1, baseline — same mechanism, same wrong-for-row-1 result.** Traced row 1 the same way:
IRQ preempts `LDA #1` at the identical relative point (`SEC`'s T2 immediately precedes
`LDA #1`'s dispatch, same as row 0 — confirming IRQ's pending-time doesn't shift by row,
as expected since `delay 29805` is row-invariant). NMI again hijacks the IRQ service's vector
fetch. Result: `$1F=$23` again — **should be `$21`** per the readme (row 1 = "NMI occurs
after LDA #1, Z clear"). Since IRQ's preempt-of-`LDA #1` timing is fixed and doesn't vary by
row, and NMI's hijack-of-IRQ mechanism doesn't depend on *which* row's NMI-position it is (as
long as NMI arrives anywhere within IRQ's T1-T6 window, which it does for many consecutive
rows), **this mechanism alone can't produce the readme's row-by-row transition at all** — it
would produce the same `$23` for every row where NMI arrives within that window, which
contradicts both the readme (which wants `21`→`21`→`20`×7→`25`×2) and our own actual full-row
output (which does eventually change, to `$27`, around row 5). There's a piece of this puzzle
not yet accounted for — most likely: at some row, NMI's own position moves *early enough* to
preempt *before* IRQ gets its chance (a pure NMI Path-A capture, no IRQ dispatch at all,
which would naturally vary with `LDA #1`/`CLC` position exactly as the readme's `21`/`20`
progression implies) — meaning the row-0/row-1 mechanism just traced might not even be the
*primary* one degrading the readme's row-1-through-9 expectations; it may only be relevant
at the specific boundary rows. Have not yet traced where/when NMI stops hijacking IRQ
mid-service and starts preempting it directly instead (analogous to test 2's row 3↔4 Path
A/B boundary) — that's the concrete next step.

**Session status:** `cargo test` unchanged (161/7, zero regressions) — all new work is
diagnostic-only (`#[ignore]`d tests, `#[cfg(test)]` accessors). No production code changes
this session (the IRQ-defer experiment lives only in the throwaway trace test).

---

### 2026-07-03 (same session) — searched for the Path-A/hijack boundary across all 12 rows, found a genuine unresolved contradiction between the confirmed mechanism and the readme

**Built `nmi_irq_all_rows_summary`** (`src/tests/roms.rs`, `#[ignore]`) — single pass through
all 12 rows, identifying at each row whether `LDA #1`/`CLC`/`NOP` gets preempted by IRQ
(`irq_pending` already true at dispatch) or NMI (`pending_nmi` already true at dispatch), plus
the captured `$1F`/`$1D`. Result:

```
row  0: preempt=IRQ(irq_pending already true) at pc=0xe360  $1F=0x23 $1D=0x00
row  1: preempt=IRQ(irq_pending already true) at pc=0xe360  $1F=0x23 $1D=0x00
row  2: preempt=IRQ(irq_pending already true) at pc=0xe360  $1F=0x23 $1D=0x00
row  3: preempt=IRQ(irq_pending already true) at pc=0xe360  $1F=0x23 $1D=0x00
row  4: preempt=IRQ(irq_pending already true) at pc=0xe360  $1F=0x27 $1D=0x23
row  5-10: preempt=IRQ(irq_pending already true) at pc=0xe360  $1F=0x27 $1D=0x23
```

**IRQ preempts `LDA #1` identically for every single row, 0 through 10** — confirms IRQ's
timing relative to `LDA #1` genuinely doesn't shift with row (as established earlier: `delay
29805` is fixed, so this is expected and mechanically consistent). The actual transition
(row 3→4) is in whether NMI's hijack lands within IRQ's T1-T6 window (rows 0-3: yes, `$1D`
stays `$00`, IRQ handler code never runs) or misses it (rows 4+: no, IRQ's own handler runs
to completion, writing `$1D=$23`, and NMI ends up interrupting something later giving
`$1F=$27`). This is internally consistent with the row-shift rate established earlier in the
session (NMI's edge arrives ~1 cycle later per row relative to the fixed IRQ-dispatch point:
gap is +2 cycles at row 0, +3 at row 1, etc., eventually exceeding the ~6-cycle T1-T6 window
around row 4).

**The genuine problem: this mechanism cannot produce the readme's row 0→1 transition at
all.** Readme wants `23`(row 0)→`21`(row 1, "Z clear" — i.e. *after* `LDA #1` executed)→`21`→
`20`×7→`25`×2. But since IRQ preempts `LDA #1` at the *identical* relative point for every
row (proven above), and the flags going into `LDA #1` are provably identical every row (`Z=1`
is established once by `LDA #0` several instructions earlier and nothing between there and
`LDA #1` touches `Z`), **the P value captured by this mechanism must be identical for every
row where it applies — it cannot legitimately produce `$23` for row 0 and `$21` for row 1.**
Confirmed row 1's hijack lands at the identical micro-op (T6, `VectorFetch`) as row 0's, via
the same fine-grained trace technique, ruling out a mechanical difference between the two.

**This directly contradicts what's mechanically possible given the confirmed, dot-exact
correct facts:** IRQ's absolute firing time (proven, cycle 29832), NMI's absolute timing
(proven, dot-exact contract test), and the row-to-row linear delay sweep (proven, exact
`A+19`/`256·A+5` formulas). All three individually check out, and their *documented*
interaction (IRQ dispatches, NMI hijacks the vector if it arrives within T1-T6) is exactly
what CLAUDE.md predicted as the known gap for this test — but even granting that mechanism
is right, it cannot reproduce the readme's actual row 0/row 1 distinction. Either:
1. Real hardware's actual rule for "how late can NMI arrive and still fully preempt (not
   merely hijack the vector)" is more permissive than what's modeled — e.g. maybe NMI can
   fully take over (re-doing its own push) even a cycle or two into the dummy-read/push phase,
   not just up to the vector-fetch cycle. This would need external verification (real 6502
   documentation beyond what's derivable from this ROM's `readme.txt`+source) to pin down
   precisely, since it's a claim about mid-sequence NMI takeover semantics, not something the
   `sync_vbl`-style "test the contract" trick can verify without a hardware reference.
2. There's a real emulator bug in this specific interaction that hasn't been found despite
   three independent rigorous checks of the surrounding pieces — possible but increasingly
   unlikely given how much has been individually verified.

**Genuinely stuck here** — this isn't "haven't looked hard enough," it's "the confirmed facts
mechanically contradict the expected output," which per systematic-debugging discipline means
stop guessing and get better reference data (real hardware trace, or the actual NMI/IRQ
arbitration timing diagrams from a primary 6502 hardware reference) before spending more
tokens on this specific angle. `cargo test`: 161/7, zero regressions, all new work
diagnostic-only.

---

### 2026-07-03 (new session) — got the primary hardware reference this log asked for; confirmed T6-check is correct as-is; found and fixed a stale diagnostic tracer that was actively misleading; read real blargg source for tests 2 and 5; found a concrete, non-physical numeric lead for test 5's `frame_reset_delay` (not applied — needs mechanistic derivation, not curve-fitting)

Picked up exactly where the previous entry parked: needed the primary 6502 hardware reference
for NMI/IRQ hijack semantics. Fetched nesdev wiki's `CPU_interrupts` page directly (raw HTML,
not just an AI-summarized fetch — verified byte-for-byte via a second, "reproduce verbatim"
fetch, since precision matters here).

**Ground truth, verbatim from nesdev:** "if NMI is asserted during the first four ticks of a
BRK instruction, the BRK instruction will execute normally at first ... but execution will
branch to the NMI vector." The tick-by-tick table places `*** At this point, the signal status
determines which interrupt vector is used ***` directly **before** line 5 (`push P on stack`)
— i.e. the checkpoint is the T4/T5 boundary, sampled using state as of the end of T4. This
matches CLAUDE.md/this log's long-standing characterization exactly.

**Tested moving the hijack check from `VectorFetch` (T6, current code) to `PushP` (T5),
matching this literally.** Result: **regressed** test 2 from 8/10 rows to 7/10 — row 7 (`36 00
00`, correctly hijacking before this change) flipped to non-hijacking. Root-caused precisely
via a corrected micro-trace (see next finding): our harness's NMI-edge delivery already has
**two compounding delay stages** baked in — (1) the outer loop's own structural ordering
(`cpu.tick()` runs, *then* the corresponding PPU dots are ticked and `cpu.nmi()` is called,
for the *same* iteration — so an edge delivered this iteration can never be seen by code
running earlier in this same `cpu.tick()` call), and (2) the "universal defer" (every micro-op
is exactly 1 cycle, so every edge gets the last-cycle-defer treatment, adding one more tick).
Together these add **two** ticks of latency between an edge's geometric arrival and its
visibility as `pending_nmi`, not one. Checking at T6 (existing code) happens to land exactly
on the tick where a T4-arriving edge becomes visible, given this two-stage delay — i.e. **the
existing T6 check is already correctly compensating for the harness's own delivery mechanics
for this specific case; nesdev's idealized cycle numbers don't map 1:1 onto our tick count.**
**Reverted** the PushP-based check; kept only an expanded comment on `VectorFetch` explaining
why T6 (not T5) is correct here, referencing this entry.

**Found the tracer that led to the wrong initial diagnosis was stale.** While investigating
the regression, `nmi_brk_micro_trace` was giving self-contradictory results (showed row 7's
PC landing at the NMI vector — hijack — while the *real* test harness's own output showed
non-hijack for the same row). Found why: `nmi_brk_micro_trace` still had the
`instruction_finished &&` gate on its defer logic that an *earlier* session's "universal
defer" refinement (see the "refinement: universal defer" entry above) removed from the real
shared harness (`run_until_complete_trace`) — this tracer has its own independent copy of the
run loop (same issue `nmi_brk_row_trace` had, previously fixed) and was never updated to
match. **Fixed** (removed the stale gate, `src/tests/roms.rs`) — this is a real, standalone
improvement: the tracer was actively producing misleading output before this fix. Kept.

**Tested demoting a leftover `pending_nmi` at `VectorFetchHi`** (candidate fix #1 from the
"refinement: universal defer" entry's two open directions) — if an NMI arrived too late to
hijack (missed the T6 check) and is still sitting in `pending_nmi` at T7, demote it back to
`nmi_pending` so it gets one more promotion cycle before it's eligible to preempt anything,
instead of immediately preempting the interrupt handler's first instruction. **No effect on
test 2's output whatsoever.** Traced why with the now-fixed tracer: for row 9 specifically,
the edge doesn't arrive early enough to be sitting in `pending_nmi` *at* `VectorFetchHi`'s own
tick at all — it arrives right around T6/T7's own last cycle, gets deferred, and only becomes
`pending_nmi` at the very top of the **next** instruction's own dispatch tick (`$E316`'s T1),
where it's consumed immediately by the *same* tick's `if self.pending_nmi` check before my
`VectorFetchHi` demotion code ever had anything to catch. **Reverted** (no effect either way,
but wrong mechanism — nothing to keep).

**Read the real blargg source for test 2** (`tests/roms/cpu/cpu_interrupts_v2/source/2-nmi_and_brk.s`,
confirmed available in-repo, not previously read carefully this deep). This nails down exactly
what rows 8-9 need: the `irq:` handler's **first instruction is literally `SEC`**
(`irq: sec / sta <irq_temp / pla / pha / sta <irq_flag / lda <irq_temp / rti`), and the
readme's row 8-9 comment "NMI after SEC at beginning of IRQ handler" means the handler's own
`SEC` must fully execute before NMI preempts the **second** instruction (`sta <irq_temp`) —
not that NMI preempts the handler's dispatch entirely. This is a **third, distinct timing
rule** — a normal "instruction boundary" edge should preempt the very next instruction it's
visible before, but this specific case needs the vector target's first instruction to run
anyway despite the edge already being visible. No mechanically-justified model for this was
found or implemented this session (candidate #1 from the earlier entry, tested above, doesn't
produce it; a working model would need to explain *why* only the vector-target's very first
instruction gets this extra grace and nothing else does). **Genuinely a distinct open
question** — recommend chasing with the same "primary reference or the C source's expected
behavior stated as unambiguous fact" approach that worked for the hijack-window question above
(this session found that reference; a similarly authoritative one for *this* sub-question would
probably resolve it quickly) rather than more guess-and-trace cycles.

**Read the real blargg source + full expected tables for test 5** (`5-branch_delays_irq.s`,
readme comment header has all 4 sub-tests' exact expected `T+ CK PC` tables — previously this
log only had `test_jmp`'s table via readme excerpts; now have all 4, plus the source). Current
`test_jmp` output has 3 bogus leading rows (`PC=03` where hardware never produces that) before
matching the readme exactly for the remaining 7 — a shift, not scattered noise.

**`begin`'s setup routine calls `jsr sync_apu` before its own `$4017` writes.** Read
`sync_apu.s`: its entire documented purpose (comment: "so that an STA $4017 immediately after
... will start the frame counter without an extra clock delay") is to guarantee a
**deterministic, jitter-free** parity for whatever `$4017` write follows it — i.e. tests using
it (test 5 here, and test 3 via `sync_vbl`) should always land on the same one of the two
possible `frame_reset_delay` values, not whichever the earlier (disproven) parity-toggle
experiment was modeling. Our code hardcodes `frame_reset_delay = 4` unconditionally.

**Swept `frame_reset_delay` from 3 to 7 (test 5's `test_jmp`), tracking how the leading bogus-row
count changes:** 3→4 leading bad rows, 4(baseline)→3, 5→2, 6→1, **7→0 (`PC` column now matches
the readme exactly, all 10 rows)**. But at `frame_reset_delay=7`, the `CK` column is then
**uniformly off by a constant −4** from the readme's values (e.g. ours `0xFE=-2` where readme
wants `2` — `-2 = 2-4` — checked across all 10 rows, exact constant offset, not row-varying).

**Did NOT apply `frame_reset_delay=7`, reverted to 4.** Real hardware's documented jitter for
this write is only 3-4 cycles (`docs` and this log's own earlier sourced comment); `7` isn't a
physically real value for this specific mechanism. Getting `PC` to match by pushing this one
knob to a non-physical value, while `CK` stays broken by a suspiciously round constant (−4),
strongly suggests there's a **separate, genuine ~3-4-cycle fixed-cost bug** somewhere in the
setup/measurement chain (`shell.inc`'s `setb`/`print_a`/`loop_n_times` macros, or the `irq:`
handler's own `ldx #7: dex / delay(29831-13) / bit $4015 / bit $4015 / bvc` polling loop, none
of which have been hand-verified this session) that, if found and fixed properly, would let
`frame_reset_delay` stay at its physically-correct value while *also* fixing `CK`. Applying 7
now would be exactly the "curve-fit a constant without understanding the mechanism" trap this
log's own discipline notes warn against (see the disproven parity-jitter experiment above —
same lesson).

**Concrete next step for test 5 (not yet done):** hand-derive the exact cycle cost of
`shell.inc`'s `setb`/`print_a`/`loop_n_times` macros and the `irq:` handler's own delay/poll
chain (same rigor as the `$E442`/`$E458` cross-validation earlier in this log — isolated
`TestBus` measurement + independent hand-trace, cross-checked) to find where the real ~4-cycle
discrepancy lives, rather than adjusting `frame_reset_delay`. `sync_apu.s` and `2-nmi_and_brk.s`
are both now fully read this session; `shell.inc`, `shell_misc.s`, `print.s`, and `testing.s`
(where `setb`/`print_a`/`loop_n_times` presumably live) have not been read yet.

**Session status:** `cargo test`: 177 passed / 9 failed (baseline unchanged from session
start — `power_up_palette` + all 5 `cpu_interrupts_v2` sub-tests, pre-existing). Net code
change: one real fix (stale tracer gate removed) + one comment expansion (`VectorFetch`,
explaining why T6 not T5) — zero behavior change to production code. All hijack-check and
`frame_reset_delay` experiments tested, root-caused, and reverted with reasoning recorded
above so a future session doesn't retry the same disproven paths. Two genuinely new, sourced
findings to build from: (1) rows 8-9 of test 2 need a third distinct timing rule (vector
target's first instruction always executes before a just-missed hijack can preempt) that
isn't modeled yet and has no working hypothesis; (2) test 5's `frame_reset_delay` is very
likely *not* the actual bug — the real ~4-cycle discrepancy is probably in unread shell
macros or the IRQ handler's own delay chain.

---

### 2026-07-04 (new session) — test 2 (nmi_and_brk) FIXED AND PASSING: the "third timing rule" was already documented on nesdev — interrupt sequences do not poll interrupts

**Hypothesis:** The previous session's open question — why does the vector target's first
instruction (`SEC` at `$E316`) execute before a just-missed NMI can preempt it (test 2
rows 8-9) — is answered verbatim by the same nesdev `CPU_interrupts` page that resolved the
hijack-window question: **"The interrupt sequences themselves do not perform interrupt
polling, meaning at least one instruction from the interrupt handler will execute before
another interrupt is serviced."** This is not a per-instruction grace quirk — it's a general
rule: the 7-cycle interrupt sequence never polls, so an edge that misses the T6 hijack check
sits pending across the entire remainder of the sequence AND the handler's first
instruction's dispatch, only being serviced at the dispatch after that.

**Change (`src/cpu/mod.rs`):** new `interrupt_poll_suppressed` flag, set by `VectorFetchHi`
(the final micro-op of every interrupt sequence — BRK, NMI, and IRQ all funnel through it),
consumed by the next queue-empty dispatch: for exactly that one dispatch, both the
`pending_nmi` direct-service check and the IRQ dispatch check are skipped (the pending flags
themselves survive untouched — the interrupt still fires, one instruction later).

**Result: `cpu_interrupts_v2/2-nmi_and_brk` PASSES** — all 10 rows exactly match the
readme, including the previously-wrong rows 8-9 (now `27 36 00`: the handler's `SEC` runs
first, so the captured P has C=1). Full `cargo test`: **178 passed / 8 failed, zero
regressions** (previous baseline 177/9). Remaining failures: `power_up_palette` (PPU,
hardware-specific), 3 `sprite_hit_roms` timing tests (separate PPU investigation, not this
log's scope), and `cpu_interrupts_v2` 3/4/5 + combined.

Tests 3, 4, 5 outputs are byte-identical before/after this change (verified) — this rule
doesn't interact with their failure modes. First implementation attempt had zero effect for
an embarrassing reason worth recording: the flag was gated at dispatch but never *set* in
`VectorFetchHi` — a diff-your-own-edit sanity check (`grep -n poll_suppressed`) caught it.

---
