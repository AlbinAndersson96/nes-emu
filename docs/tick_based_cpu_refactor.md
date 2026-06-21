# Tick-Based CPU Refactor Design

## Why this refactor is needed

The current `Cpu::step()` executes a complete instruction in a single call and returns a
bulk cycle count. Interrupt polling happens **only at the start of each step** — before the
opcode is even fetched. On real hardware, polling happens at the **end of every bus cycle**.
This granularity difference causes four blargg ROM tests to fail:

| Test | What it checks |
|------|---------------|
| `cpu_interrupts_v2/2-nmi_and_brk` | NMI arriving during BRK's vector-fetch cycles hijacks the vector to $FFFA |
| `cpu_interrupts_v2/3-nmi_and_irq` | Same hijack inside the IRQ-service sequence itself |
| `cpu_interrupts_v2/4-irq_and_dma` | IRQ polled mid-DMA rather than after the full 513-cycle OAM stall |
| `cpu_interrupts_v2/5-branch_delays_irq` | IRQ pending at cycle T3 of a page-crossing branch aborts the page-fix cycle |

All other tests pass (154/159 total). The refactor does not change any observable behaviour
for instructions where interrupts arrive between instructions — only the sub-instruction
cases above are affected.

---

## Current architecture

```
run loop:
  cycles = cpu.step(bus)          // entire instruction, all bus cycles at once
  bus.tick_ppu(cycles)            // advance PPU by bulk count
  if bus.tick_apu(cycles): cpu.irq()  // advance APU, assert IRQ if line is high

cpu.step():
  if nmi_pending: service NMI
  if irq_pending && !FLAG_I: service IRQ
  opcode = bus.read(pc++)
  cycles = instructions::execute(cpu, bus, opcode)  // all micro-ops at once
  return cycles
```

### What "interrupt polling" means on real hardware

The 6502 checks NMI/IRQ status at the **penultimate clock** of each instruction (the cycle
that also initiates the next opcode fetch). The result of that check determines what happens
next: fetch a new instruction, or start the interrupt-service sequence.

One consequence: an interrupt that asserts after the penultimate clock of an instruction is
not seen until the penultimate clock of the *following* instruction.  The current model
approximates this correctly for back-to-back instructions.  It only breaks down when the
signal changes **inside** a multi-cycle instruction.

---

## Target architecture

```
run loop:
  cpu.tick(bus)            // exactly ONE bus cycle
  bus.tick_ppu(1)
  if bus.tick_apu(1): cpu.irq()

cpu.tick():
  execute next micro-op for the current instruction
  at the end of each micro-op, poll interrupts
```

### Performance note

The NES runs at 1.789 MHz, so ~1.789 million calls to `tick()` per second of emulated
time.  That is fast in Rust; no performance concern.

---

## Proposed data structure

### Micro-op queue (recommended)

Store the instruction's remaining bus cycles as a small queue of tagged operations:

```rust
#[derive(Clone, Copy)]
enum MicroOp {
    ReadPc,                        // fetch byte from PC, increment PC
    ReadAddr(u16),                 // read a byte from a fixed address
    WriteAddr(u16, u8),            // write a byte to a fixed address
    ReadStack,                     // pop from stack
    WriteStack(u8),                // push to stack
    PollInterrupt,                 // explicit interrupt-check point (between micro-ops)
    Compute(fn(&mut Cpu)),         // register-only operation, no bus access
    BranchPageFix,                 // special: page-fix cycle, CANCELLED if IRQ pending
    VectorFetch(u16),              // special: vector read, HIJACKABLE by NMI
}

struct Cpu {
    // ... existing registers ...
    queue: [MicroOp; 8],
    queue_len: u8,
    queue_idx: u8,
    /// Scratch storage shared across micro-ops within one instruction.
    scratch: [u8; 4],
    /// Which micro-op produced the effective address (for RMW).
    pending_nmi: bool,
    pending_irq: bool,
}
```

`Cpu::tick(bus)` pops one entry from `queue`, executes it, then checks
`pending_nmi`/`pending_irq` against the polling rules.

### Alternative: explicit state machine

Each instruction is an arm in a large `match (state, opcode)` with states `T2`, `T3`, … up
to `T7`.  More lines of code but slightly more predictable performance.  The micro-op queue
is recommended because it re-uses addressing-mode sequences across instructions and avoids a
combinatorial state enum.

---

## The four interrupt behaviours to implement

### 1. NMI hijacks BRK / IRQ-service vector fetch (tests 2 & 3)

BRK and the IRQ service sequence share the same 7-cycle template:

```
T1  fetch opcode (BRK=0x00) or dummy read (IRQ)
T2  read PC+1 (dummy for IRQ) or padding byte for BRK
T3  push PCH
T4  push PCL
T5  push P  (FLAG_B=1 for BRK; FLAG_B=0 for IRQ)
T6  read vector low  ← NMI can hijack HERE
T7  read vector high ← and HERE
```

NMI hijack rule: **if `nmi_pending` is true when T6 starts, redirect the vector reads to
$FFFA/$FFFB instead of $FFFE/$FFFF**.  The P byte already pushed (FLAG_B clear for IRQ,
set for BRK) is not changed.

Implementation in `tick()`:

```rust
MicroOp::VectorFetch(addr) => {
    let real_addr = if cpu.pending_nmi {
        // Redirect low byte to NMI vector; T7 reads +1 from the same base.
        cpu.pending_nmi = false;
        cpu.nmi_redirect = true;
        0xFFFA
    } else {
        addr
    };
    cpu.scratch[cpu.scratch_idx] = bus.read(real_addr);
    cpu.scratch_idx += 1;
}
```

Queue the two `VectorFetch` ops back-to-back.  After T7, assemble the 16-bit vector from
`scratch` and jump.

### 2. Branch page-fix abort (test 5)

A taken branch that crosses a page normally has four cycles:
```
T1  fetch opcode
T2  fetch signed offset, evaluate condition, add offset to PCL (ignore carry)
T3  spurious read at the page-wrong PC; fix PCH if page crossed
T4  (page-fix cycle) — this is the cycle that can be aborted
```

Abort rule: **if `irq_pending` (or `nmi_pending`) is asserted at the END of T3, skip T4 and
service the interrupt instead.  The PC pushed to the interrupt stack is the page-wrong
address from T3, not the corrected address**.

Implementation:
```rust
MicroOp::BranchPageFix => {
    if cpu.pending_irq || cpu.pending_nmi {
        // Abort: skip page fix. PC is already the wrong-page address.
        cpu.queue_len = 0; // clear remaining ops
        cpu.queue_interrupt();
    } else {
        // Normal page fix
        cpu.pc = (cpu.pc & 0x00FF) | (cpu.branch_target_hi << 8);
    }
}
```

The branch helper sets `cpu.branch_target_hi` before queuing `BranchPageFix`.

### 3. IRQ mid-DMA (test 4)

#### OAM DMA

OAM DMA halts the CPU for 513 (or 514 on an odd cycle) clock cycles.  On real hardware the
CPU is halted mid-instruction; the IRQ is polled at each cycle of the halt.  The IRQ fires
immediately after the DMA completes on the first cycle the CPU would normally see an
interrupt.

Implementation: instead of returning a bulk stall from `bus.oam_dma()`, replace the
current `oam_dma_stall` field with a DMA-in-progress flag:

```rust
struct Bus {
    oam_dma_active: bool,
    oam_dma_cycles_left: u16,
    // ...
}
```

In the run loop, when `oam_dma_active`, call `bus.tick_dma()` instead of `cpu.tick()`:

```rust
loop {
    if bus.oam_dma_active {
        bus.tick_dma();        // advance DMA by 1 cycle (copy one byte every 2 cycles)
        bus.tick_ppu(1);
        if bus.tick_apu(1) { cpu.irq(); }  // IRQ can assert during DMA
    } else {
        cpu.tick(bus);
        bus.tick_ppu(1);
        if bus.tick_apu(1) { cpu.irq(); }
    }
}
```

After `oam_dma_cycles_left` reaches 0, `oam_dma_active` clears and the next `cpu.tick()`
will pick up the pending IRQ normally.

#### DMC DMA

The 4-cycle DMC halt is similar but shorter.  Currently it is applied as a post-instruction
adjustment.  With the tick loop, add a `dmc_dma_cycles_left: u8` field to `Bus` and handle
it exactly like OAM DMA above.

---

## Code changes required

### `src/cpu/mod.rs`

| Change | Notes |
|--------|-------|
| Remove `irq_inhibit_next` | Replaced by cycle-level polling at T_penultimate |
| Remove `irq_deferred_blocked` | Replaced by direct RTI handling below |
| Remove `step()` | Replaced by `tick()` |
| Add `queue: [MicroOp; 8]`, `queue_len: u8`, `queue_idx: u8` | Micro-op queue |
| Add `scratch: [u8; 4]`, `scratch_idx: u8` | Intra-instruction scratch space |
| Add `branch_target_hi: u8` | Used by `BranchPageFix` |
| Add `nmi_redirect: bool` | Set on vector hijack |
| `tick(bus)` executes one micro-op and calls `poll_interrupts()` at the end | |

`poll_interrupts()` (called after each micro-op):
```rust
fn poll_interrupts(&mut self) {
    // NMI is edge-triggered; latch it.
    // IRQ is level-triggered; only proceed if FLAG_I is clear.
    // If the queue is empty (instruction just completed), start interrupt service.
}
```

**RTI special case**: RTI restores `P` from the stack at T4.  The new `FLAG_I` takes effect
immediately for interrupt polling.  In the tick model, after the T4 pop-P micro-op runs and
`FLAG_I` is restored to 1, the subsequent `poll_interrupts()` call will see `FLAG_I=1` and
will not start an IRQ service.  No separate `irq_deferred_blocked` flag needed.

**CLI/SEI/PLP latency**: these instructions change `FLAG_I` but the change is invisible to
interrupt polling until the NEXT instruction's penultimate cycle.  Implement by having the
`Compute` micro-op for CLI/SEI/PLP write to a `shadow_flag_i: Option<bool>` instead of
`cpu.p` directly.  `poll_interrupts()` uses the current `cpu.p` (old value), and the final
micro-op of the instruction commits `shadow_flag_i` to `cpu.p`.

### `src/cpu/instructions.rs`

Replace `execute(cpu, bus, opcode) -> u8` with `queue_instruction(cpu, opcode)` which
populates `cpu.queue` with the micro-ops for the given opcode.  No bus accesses happen
here; all accesses happen in `tick()` when the micro-ops are consumed.

The addressing-mode helpers (`addr_absolute_x`, etc.) become `queue_addr_absolute_x`
functions that push address-calculation micro-ops instead of executing them.

The total number of micro-ops queued must equal the documented cycle count for each
instruction (which is what the current `execute()` returns — that return value becomes a
simple assertion in debug builds).

### `src/tests/roms.rs` run loop

```rust
fn run_until_complete(bus: &mut Bus, cpu: &mut Cpu) {
    loop {
        // check completion ...

        if bus.oam_dma_active || bus.dmc_dma_cycles_left > 0 {
            bus.tick_dma();
        } else {
            cpu.tick(bus);
        }
        if bus.tick_ppu(1) { cpu.nmi(); }
        if bus.tick_apu(1) { cpu.irq(); }
    }
}
```

### `src/bus.rs`

- Remove `oam_dma_stall: u16`.
- Add `oam_dma_active: bool`, `oam_dma_cycles_left: u16`, `oam_dma_page: u8`,
  `oam_dma_byte: u8` (index).
- `oam_dma()` sets `oam_dma_active = true` and initialises the counters instead of copying
  256 bytes immediately.
- New `tick_dma()` copies one byte (every 2 cycles) and decrements `oam_dma_cycles_left`.
- Replace `dmc_dma_stall: u8` with `dmc_dma_cycles_left: u8`; `tick_dma()` handles it.

---

## Migration strategy

The refactor can be done incrementally against the test suite:

1. **Skeleton**: introduce `tick()` and `queue` alongside the existing `step()`.  Make
   `step()` loop over `tick()` internally.  All 154 existing tests must still pass after
   this step.

2. **Branch page-fix abort**: implement `BranchPageFix` and add it only to the branch
   handler.  Enables test `5-branch_delays_irq`.

3. **NMI vector hijack**: implement `VectorFetch` with the NMI redirect.  Add it to `brk()`
   and the interrupt-service sequence.  Enables tests `2-nmi_and_brk` and `3-nmi_and_irq`.

4. **DMA cycle-by-cycle**: replace bulk OAM/DMC stall with the tick-loop DMA.  Enables test
   `4-irq_and_dma`.

5. **Remove `step()` and the `irq_inhibit_next`/`irq_deferred_blocked` flags**: clean up
   once all 5 tests pass.

Each step keeps the test count ≥ 154; regressions are caught immediately.

---

## Files to change

| File | Type of change |
|------|---------------|
| `src/cpu/mod.rs` | Major: new fields, `tick()`, `poll_interrupts()`, remove `step()` |
| `src/cpu/instructions.rs` | Major: `execute()` → `queue_instruction()` |
| `src/bus.rs` | Moderate: replace bulk DMA stall with cycle-by-cycle fields and `tick_dma()` |
| `src/tests/roms.rs` | Minor: run loop uses `tick()` instead of `step()` |
| `src/main.rs` | Minor: same run-loop change as `roms.rs` (if main has one) |
