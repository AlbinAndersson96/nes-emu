# CPU Interrupt Handling Reference

How NMI, IRQ, and BRK are dispatched, how NMI can hijack an in-progress BRK or IRQ
service sequence, and the polling-granularity subtlety that governs exactly when an
interrupt signal becomes "visible" to the CPU. This document describes confirmed,
verified behavior. For the full investigation history (hypotheses tried, ruled out,
and still-open questions), see `docs/investigations/cpu_interrupt_debug_log.md`.

## Micro-op model

`src/cpu/mod.rs` executes instructions and interrupt sequences one bus cycle at a
time via a micro-op queue (`MicroOp`, `src/cpu/mod.rs`). An interrupt service
sequence — whether entered via BRK, a genuine NMI, or a genuine IRQ — always takes
the same 7-cycle shape:

```
T1  opcode fetch (BRK only — NMI/IRQ preempt in place of a fetch, see below)
T2  padding byte fetch (BRK) / dummy read (NMI, IRQ)
T3  PushPcHi
T4  PushPcLo
T5  PushP        — pushes P; FLAG_B set for BRK, clear for NMI/IRQ
T6  VectorFetch  — reads the vector low byte; checks pending_nmi for hijack
T7  VectorFetchHi — reads the vector high byte, sets PC
```

`PushP`'s B-flag value is fixed by *which* event initiated the sequence (BRK sets
it, NMI/IRQ clear it) and is **not** touched by any later hijack — see below.

## NMI hijacking BRK or IRQ

Real 6502 hardware doesn't special-case "NMI during BRK" — the interrupt sequence
above is generic. NMI hijacking a BRK or IRQ that's already begun servicing only
ever redirects the **vector fetch (T6)** to `$FFFA`; it never un-does the push that
already happened at T3-T5. Concretely:

- If NMI's edge is detected (`pending_nmi == true`) by the time `VectorFetch` (T6)
  runs, the vector address is redirected from `$FFFE`/whatever to `$FFFA`, and
  `pending_nmi` is consumed.
- The **pushed P retains whatever B value the original event set** — B=1 if the
  hijacked sequence was BRK, B=0 if it was IRQ. This is confirmed against
  `cpu_interrupts_v2/2-nmi_and_brk`'s own `readme.txt`, which documents
  `36 00 00 — NMI interrupting BRK, with B bit set on stack` as the *correct*
  result, not a bug. (Fixing this was a mistake in an earlier session — see the
  debug log's first entry for the retraction.)

### NMI hijacking an in-progress IRQ dispatch (not just BRK)

`cpu_interrupts_v2/3-nmi_and_irq` exercises a related but distinct case: NMI
arriving while a *genuine IRQ* (not BRK) is mid-dispatch. Traced and confirmed for
this ROM's row 0/row 1 (`docs/investigations/cpu_interrupt_debug_log.md`, the
`nmi_irq_row0_arbitration_trace` entries):

1. The IRQ line (`irq_pending`) becomes visible at a CPU instruction's dispatch
   check. If unmasked (`FLAG_I` clear) and the queue is empty, the CPU performs two
   dummy reads (T1-T2) *instead of* fetching the pending opcode — the interrupted
   instruction is not executed and its address is preserved on the stack (PC isn't
   advanced during a dummy read).
2. The interrupt sequence proceeds normally: `PushPcHi`/`PushPcLo`/`PushP` (T3-T5),
   pushing the *original, not-yet-executed* instruction's return address and P
   (with B=0, since this is an IRQ dispatch).
3. If NMI's edge lands anywhere in T3-T6 (verified: T3 in the traced case), it
   hijacks the vector fetch exactly as it would for BRK — `$FFFE`'s target is
   swapped for `$FFFA`. **The IRQ handler's own code never executes** — only the
   NMI handler runs, and it reads whatever P the IRQ dispatch already pushed.
4. NMI's own handler eventually `RTI`s back to the address that was pushed —
   which is the *original preempted instruction*, not the IRQ handler — so that
   instruction finally executes for real, for the first time, after the interrupt
   round-trip.

This means a single readable byte captured shortly after such a sequence (e.g. via
`PLA`/`PHA` inside the NMI handler, `cpu_interrupts_v2/3-nmi_and_irq.s`) reflects
flags from *before* the originally-preempted instruction ran, not after — because
the push already happened before NMI hijacked anything. (An earlier version of this
document recorded a "confirmed contradiction" between this mechanism and that ROM's
expected row-by-row output. It dissolved once the test's source was read correctly:
the readme's row-varying bytes come from *NMI's* row-varying position against the
`LDA #1`/`CLC`/`NOP` window — the NMI handler's `bit SNDCHN` acks the APU IRQ, so
for most rows the IRQ never dispatches at all — plus a real one-cycle emulator bug
in the `$2002` read sampling dot that shifted every row. `3-nmi_and_irq` now passes;
see the debug log's 2026-07-04 entries.)

## Interrupt sequences do not poll interrupts

Real 6502 hardware does not poll the interrupt lines during the 7-cycle interrupt
sequence itself (nesdev `CPU_interrupts`: *"The interrupt sequences themselves do
not perform interrupt polling, meaning at least one instruction from the interrupt
handler will execute before another interrupt is serviced"*). An NMI edge that
arrives too late to win the T6 hijack check therefore stays pending across the rest
of the sequence **and** the handler's first instruction — it is serviced at the
dispatch after that, never before.

Modeled via `Cpu::interrupt_poll_suppressed` (`src/cpu/mod.rs`): set by
`VectorFetchHi` (the final micro-op of every interrupt sequence — BRK, NMI, and IRQ
all funnel through it), consumed by the next queue-empty dispatch, which skips both
the `pending_nmi` direct-service check and the IRQ dispatch check for exactly that
one dispatch. The pending flags survive untouched, so the interrupt fires one
instruction later.

The same suppression models the DMA case: an interrupt that first asserts during an
OAM/DMC DMA stall missed the stalled instruction's poll point, so the run loop
delivers it at DMA end and calls `Cpu::suppress_next_interrupt_poll()` — the first
post-DMA instruction executes before service (`4-irq_and_dma`'s long "8" band). An
IRQ already pending when the DMA began services immediately after it.

This was the missing "third timing rule" for `cpu_interrupts_v2/2-nmi_and_brk`
rows 8-9 ("NMI after SEC at beginning of IRQ handler" — the handler's `SEC` must
run before the just-missed NMI preempts). With it, that test **passes in full**.

## Interrupt-polling granularity (the deferred-edge fix)

Real 6502 hardware samples the interrupt lines going into an instruction's *last*
cycle, using line state from *before* that cycle begins. An edge that occurs
exactly on that last cycle is therefore invisible to that poll — it can only
affect the dispatch *after* the next one, not the very next one.

Before this was modeled, this emulator made an edge landing on an instruction's
last cycle visible immediately (one dispatch too early). The fix
(`SystemClock::step`, `src/system.rs` — shared by the real run loop and the ROM
test harness) ticks the PPU one cycle at a time after each CPU tick and defers
delivery of `cpu.nmi()` by exactly one extra tick when the edge lands on the
final sub-cycle of that tick's delta:

```rust
for i in 0..remaining {
    if bus.tick_ppu(1) {
        if i + 1 == remaining {
            new_deferred = true;   // last cycle: defer one more tick
        } else {
            got_nmi = true;        // any earlier cycle: visible now
        }
    }
}
if deferred_nmi { got_nmi = true; }
deferred_nmi = new_deferred;
if got_nmi { cpu.nmi(); }
```

This is NMI-specific and deliberately **not** applied to IRQ. NMI is edge-triggered
and the defer models exactly when that edge becomes externally visible. IRQ is
level-triggered and re-sampled fresh every cycle (`SystemClock::step` ticks the
APU one cycle at a time; a level FIRST asserting on an instruction's final cycle
is flagged via `Cpu::irq_on_last_cycle` for the taken-branch last-clock ignore,
but is otherwise visible at the next dispatch as before) — applying the same
one-tick defer to IRQ was tried and
**confirmed to regress** a previously-correct case
(`cpu_interrupts_v2/3-nmi_and_irq` row 0): delaying IRQ's visibility by one cycle
let an instruction that should have been preempted by IRQ execute cleanly instead,
changing which mechanism (IRQ-dispatch-then-NMI-hijack vs. plain NMI preempt) fired
and producing the wrong captured byte. See the debug log for the full trace.

Verified impact of the NMI defer fix: `ppu/vbl_clear_time` now passes outright
(same underlying mechanism — VBL flag clear timing is NMI-adjacent), and
`cpu_interrupts_v2/2-nmi_and_brk` went from ~2/10 to 8-9/10 rows matching the
ROM's expected table, with zero regressions across the rest of the suite. (The
final 2 rows were later fixed by the interrupt-sequences-don't-poll rule above;
the test now passes in full.)

## VBlank sync (`sync_vbl`) and fixed-cycle delay routines

The blargg `cpu_interrupts_v2` test ROMs use a shared framework
(`tests/roms/cpu/cpu_interrupts_v2/source/common/`) with two building blocks worth
knowing if debugging these tests further:

- **`delay_a_25_clocks`** (`common/delay.s`, mapped to `$E442` in these ROMs) —
  delays exactly `A + 25` CPU cycles (including its own `JSR`/`RTS` overhead), for
  any `A` from 0 to 255. Verified two independent ways during the
  investigation: an isolated `TestBus`-only measurement (the
  `isolate_delay_routines` diagnostic, since removed from
  `src/tests/roms.rs` with the other one-off tracers) and an independent
  hand-trace of the actual 6502 instruction sequence — both agree exactly
  for every value tested, including large loop-heavy ones (`A=215` → 234
  cycles). Not a source of any known bug.
- **`delay_256a_11_clocks_`** (mapped to `$E458`) — a coarser delay built by
  looping `delay_a_25_clocks` calls; costs `256·A + 5` cycles. Also verified exact.
- **`sync_vbl`** (`common/sync_vbl.s`, mapped to `$E200`) — a sophisticated
  multi-iteration precision convergence loop (not a simple "poll once" loop; see
  the source file's own comments), documented to guarantee: *reading `PPUSTATUS`
  29768+ clocks after `sync_vbl` returns will see the VBlank flag set; reading it
  immediately will see it clear.* Verified this contract holds across 10 different
  starting PPU phases, including several straddling VBlank onset exactly
  (the `verify_sync_vbl_contract` diagnostic, since removed from
  `src/tests/roms.rs` with the other one-off tracers) — not a source of any
  known bug either.
- The APU's frame-IRQ fires `frame_reset_delay` (**7 or 8**, parity-dependent, as
  applied at instruction start — 4 unticked instruction cycles + hardware's
  3-cycle (aligned) / 4-cycle (unaligned) post-write-cycle delay) + first `MODE0`
  step after the $4017 write is applied. See the $4017 write handler in
  `src/apu/mod.rs` for the derivation;
  the old flat 4 made the IRQ fire 3 cycles early relative to the instruction
  stream and was the root cause of `4-irq_and_dma`'s 3-row table shift.
- `$4015` reads pre-advance the APU 4 cycles so the frame-IRQ flag (and its
  read-clear side effect) are sampled at the read cycle rather than the
  reading instruction's start (`APU_READ_PREADVANCE`, `src/bus.rs`);
  `Bus::tick_apu` repays the debt internally so total APU time is conserved.
  This fixed `5-branch_delays_irq`'s CK column (a 1-cycle-per-iteration
  sampling-window walk over the flag's set moment, previously uniformly -4).

## Branch instructions and IRQ

Confirmed against all four sub-tables of `cpu_interrupts_v2/5-branch_delays_irq`
(which passes in full):

- An IRQ already visible at a branch's dispatch preempts the branch exactly like
  any other instruction (the pushed PC is the branch's own address).
- An IRQ asserting during a taken page-crossing branch's T1-T3 aborts the T4
  page-fix: the cycle still elapses but the fix is not applied, the page-wrong
  PC is pushed, and the full 7-cycle interrupt sequence follows (handler entry
  at branch T1 + 11).
- A taken non-page-crossing branch ignores an IRQ on its last clock (T3) — but
  only one that FIRST asserts on that clock; one asserted during T1/T2 services
  normally right after the branch. Modeled by `Cpu::irq_on_last_cycle()` (called
  by the run loop, which ticks the APU one cycle at a time) plus
  `branch_delay_irq`; the deferred IRQ then fires after the next instruction.

These three subsystems being independently proven correct is what let
`3-nmi_and_irq` finally be resolved: once each piece was individually verified,
the remaining discrepancy narrowed to the one-cycle `$2002` read-sampling-dot bug
(see the note under "NMI hijacking an in-progress IRQ dispatch" above). With that
fixed, all of `cpu_interrupts_v2` (tests 1–5 plus the combined suite) passes, and
all 159 blargg CPU ROM tests are green.
