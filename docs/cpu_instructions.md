# NES CPU Instruction Set Reference

The NES uses a MOS 6502 variant (Ricoh 2A03) with the decimal mode (BCD) disabled.

## Registers

| Register | Width | Description |
|----------|-------|-------------|
| A  | 8-bit | Accumulator — arithmetic and logic target |
| X  | 8-bit | Index register |
| Y  | 8-bit | Index register |
| SP | 8-bit | Stack pointer — offset into page $01 ($0100–$01FF), grows downward |
| PC | 16-bit | Program counter |
| P  | 8-bit | Processor status (flags) |

## Processor Status Flags (P register)

```
Bit:  7  6  5  4  3  2  1  0
Flag: N  V  1  B  D  I  Z  C
```

| Flag | Name | Set when |
|------|------|----------|
| N | Negative | Result bit 7 = 1 |
| V | Overflow | Signed arithmetic overflow |
| 1 | (unused) | Always 1 |
| B | Break | Set when P is pushed by BRK/PHP; clear when pushed by IRQ/NMI |
| D | Decimal | BCD mode — **disabled on NES, has no effect** |
| I | Interrupt Disable | Blocks IRQ (not NMI) when set |
| Z | Zero | Result = 0 |
| C | Carry | Unsigned overflow / borrow (inverted) |

## Addressing Modes

| Mode | Syntax | Bytes | Description |
|------|--------|-------|-------------|
| Implied | `INX` | 1 | Operand implicit to instruction |
| Accumulator | `ASL A` | 1 | Operand is A register |
| Immediate | `LDA #$10` | 2 | Operand is literal byte |
| Zero Page | `LDA $10` | 2 | Address in $0000–$00FF |
| Zero Page,X | `LDA $10,X` | 2 | Zero page address + X (wraps within page) |
| Zero Page,Y | `LDA $10,Y` | 2 | Zero page address + Y (wraps within page) |
| Absolute | `LDA $1234` | 3 | Full 16-bit address |
| Absolute,X | `LDA $1234,X` | 3 | Address + X; +1 cycle on page cross |
| Absolute,Y | `LDA $1234,Y` | 3 | Address + Y; +1 cycle on page cross |
| Relative | `BEQ $10` | 2 | Signed 8-bit offset from PC+2; range −128..+127 |
| (Indirect,X) | `LDA ($10,X)` | 2 | Zero page ptr = ($10+X); read 16-bit address from there |
| (Indirect),Y | `LDA ($10),Y` | 2 | Read 16-bit address from zero page $10; add Y; +1 cycle on page cross |
| (Indirect) | `JMP ($1234)` | 3 | Jump to address stored at $1234 (JMP only) |

> **Page cross**: a +1 cycle penalty applies when the final effective address crosses a 256-byte page boundary (high byte of base ≠ high byte of result). Store instructions (STA, STX, STY) always take the extra cycle.

> **JMP indirect bug**: `JMP ($xxFF)` reads the low byte from `$xxFF` and the high byte from `$xx00` instead of `$(xx+1)00`. Reproduce this exactly.

> **Read-modify-write (RMW)**: ASL, LSR, ROL, ROR, INC, DEC on memory perform three bus operations at the effective address: read → write-back old value → write new value. The intermediate write-back is observable on hardware registers.

> **Dummy reads**: Several addressing modes perform a spurious bus read before reaching the true effective address. The most common cases: (a) any read instruction with Absolute,X or Absolute,Y crosses a page — the CPU reads at `base_hi : (lo + index) & 0xFF` before correcting to the full address; (b) RMW instructions always perform this uncorrected read regardless of page crossing; (c) some zero-page indexed modes read the unindexed zero-page address first. These reads are architecturally visible — they trigger register side-effects (e.g. reading $2002 clears the VBlank flag).

---

## Instructions

### ADC — Add with Carry
```
A = A + M + C
```
Flags: **N V Z C**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Immediate | $69 | 2 | 2 |
| Zero Page | $65 | 2 | 3 |
| Zero Page,X | $75 | 2 | 4 |
| Absolute | $6D | 3 | 4 |
| Absolute,X | $7D | 3 | 4+1 |
| Absolute,Y | $79 | 3 | 4+1 |
| (Indirect,X) | $61 | 2 | 6 |
| (Indirect),Y | $71 | 2 | 5+1 |

Call `CLC` before the first byte of a multi-byte add. V is set when the sign of the result differs from both inputs.

---

### AND — Bitwise AND
```
A = A & M
```
Flags: **N Z**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Immediate | $29 | 2 | 2 |
| Zero Page | $25 | 2 | 3 |
| Zero Page,X | $35 | 2 | 4 |
| Absolute | $2D | 3 | 4 |
| Absolute,X | $3D | 3 | 4+1 |
| Absolute,Y | $39 | 3 | 4+1 |
| (Indirect,X) | $21 | 2 | 6 |
| (Indirect),Y | $31 | 2 | 5+1 |

---

### ASL — Arithmetic Shift Left *(RMW)*
```
C ← [76543210] ← 0
```
Flags: **N Z C**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Accumulator | $0A | 1 | 2 |
| Zero Page | $06 | 2 | 5 |
| Zero Page,X | $16 | 2 | 6 |
| Absolute | $0E | 3 | 6 |
| Absolute,X | $1E | 3 | 7 |

Bit 7 moves into C; 0 fills bit 0. Equivalent to unsigned multiply by 2.

---

### BCC — Branch if Carry Clear
```
if C == 0: PC = PC + 2 + offset
```
Flags: none

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Relative | $90 | 2 | 2 / 3 / 4 |

2 cycles if not taken; 3 if taken, same page; 4 if taken, different page.

---

### BCS — Branch if Carry Set
```
if C == 1: PC = PC + 2 + offset
```
Flags: none

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Relative | $B0 | 2 | 2 / 3 / 4 |

---

### BEQ — Branch if Equal (Zero Set)
```
if Z == 1: PC = PC + 2 + offset
```
Flags: none

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Relative | $F0 | 2 | 2 / 3 / 4 |

---

### BIT — Bit Test
```
Z = (A & M) == 0
N = M[7]
V = M[6]
```
Flags: **N V Z**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Zero Page | $24 | 2 | 3 |
| Absolute | $2C | 3 | 4 |

A is not modified. N and V are loaded directly from bits 7–6 of M, regardless of A.

---

### BMI — Branch if Minus (Negative Set)
```
if N == 1: PC = PC + 2 + offset
```
Flags: none

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Relative | $30 | 2 | 2 / 3 / 4 |

---

### BNE — Branch if Not Equal (Zero Clear)
```
if Z == 0: PC = PC + 2 + offset
```
Flags: none

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Relative | $D0 | 2 | 2 / 3 / 4 |

---

### BPL — Branch if Plus (Negative Clear)
```
if N == 0: PC = PC + 2 + offset
```
Flags: none

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Relative | $10 | 2 | 2 / 3 / 4 |

---

### BRK — Software Interrupt
```
push PC+2, push P (B=1), I=1, PC = [$FFFE]
```
Flags: **I** (set); **B** pushed as 1

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $00 | 1* | 7 |

*BRK is a 2-byte instruction on the real chip; the second byte is a padding/signature byte that is skipped over. The return address pushed is PC+2 (past the padding). The IRQ vector at $FFFE/$FFFF is used.

---

### BVC — Branch if Overflow Clear
```
if V == 0: PC = PC + 2 + offset
```
Flags: none

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Relative | $50 | 2 | 2 / 3 / 4 |

---

### BVS — Branch if Overflow Set
```
if V == 1: PC = PC + 2 + offset
```
Flags: none

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Relative | $70 | 2 | 2 / 3 / 4 |

---

### CLC — Clear Carry
```
C = 0
```
Flags: **C**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $18 | 1 | 2 |

---

### CLD — Clear Decimal
```
D = 0
```
Flags: **D**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $D8 | 1 | 2 |

BCD is disabled on the 2A03; this instruction clears the flag but the flag has no arithmetic effect.

---

### CLI — Clear Interrupt Disable
```
I = 0
```
Flags: **I** (delayed by 1 instruction)

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $58 | 1 | 2 |

The instruction following CLI executes before a pending IRQ is serviced.

---

### CLV — Clear Overflow
```
V = 0
```
Flags: **V**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $B8 | 1 | 2 |

There is no SEV counterpart; V can only be set by ADC/SBC or the BIT instruction (via M[6]).

---

### CMP — Compare Accumulator
```
A - M  (result discarded)
C = (A >= M)
Z = (A == M)
N = (A - M)[7]
```
Flags: **N Z C**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Immediate | $C9 | 2 | 2 |
| Zero Page | $C5 | 2 | 3 |
| Zero Page,X | $D5 | 2 | 4 |
| Absolute | $CD | 3 | 4 |
| Absolute,X | $DD | 3 | 4+1 |
| Absolute,Y | $D9 | 3 | 4+1 |
| (Indirect,X) | $C1 | 2 | 6 |
| (Indirect),Y | $D1 | 2 | 5+1 |

V is **not** affected. Use as unsigned comparison: C=1 means A ≥ M.

---

### CPX — Compare X Register
```
X - M  (result discarded; C/Z/N set same as CMP)
```
Flags: **N Z C**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Immediate | $E0 | 2 | 2 |
| Zero Page | $E4 | 2 | 3 |
| Absolute | $EC | 3 | 4 |

---

### CPY — Compare Y Register
```
Y - M  (result discarded; C/Z/N set same as CMP)
```
Flags: **N Z C**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Immediate | $C0 | 2 | 2 |
| Zero Page | $C4 | 2 | 3 |
| Absolute | $CC | 3 | 4 |

---

### DEC — Decrement Memory *(RMW)*
```
M = M - 1
```
Flags: **N Z**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Zero Page | $C6 | 2 | 5 |
| Zero Page,X | $D6 | 2 | 6 |
| Absolute | $CE | 3 | 6 |
| Absolute,X | $DE | 3 | 7 |

C and V are unaffected. Wraps $00 → $FF.

---

### DEX — Decrement X
```
X = X - 1
```
Flags: **N Z**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $CA | 1 | 2 |

---

### DEY — Decrement Y
```
Y = Y - 1
```
Flags: **N Z**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $88 | 1 | 2 |

---

### EOR — Bitwise Exclusive OR
```
A = A ^ M
```
Flags: **N Z**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Immediate | $49 | 2 | 2 |
| Zero Page | $45 | 2 | 3 |
| Zero Page,X | $55 | 2 | 4 |
| Absolute | $4D | 3 | 4 |
| Absolute,X | $5D | 3 | 4+1 |
| Absolute,Y | $59 | 3 | 4+1 |
| (Indirect,X) | $41 | 2 | 6 |
| (Indirect),Y | $51 | 2 | 5+1 |

`EOR #$FF` inverts all bits (bitwise NOT on A).

---

### INC — Increment Memory *(RMW)*
```
M = M + 1
```
Flags: **N Z**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Zero Page | $E6 | 2 | 5 |
| Zero Page,X | $F6 | 2 | 6 |
| Absolute | $EE | 3 | 6 |
| Absolute,X | $FE | 3 | 7 |

C and V are unaffected. Wraps $FF → $00.

---

### INX — Increment X
```
X = X + 1
```
Flags: **N Z**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $E8 | 1 | 2 |

---

### INY — Increment Y
```
Y = Y + 1
```
Flags: **N Z**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $C8 | 1 | 2 |

---

### JMP — Jump
```
PC = address
```
Flags: none

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Absolute | $4C | 3 | 3 |
| (Indirect) | $6C | 3 | 5 |

**Hardware bug**: `JMP ($xxFF)` reads low byte from `$xxFF` and high byte from `$xx00` (not `$(xx+1)00`).

---

### JSR — Jump to Subroutine
```
push PC+2 (high then low), PC = address
```
Flags: none

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Absolute | $20 | 3 | 6 |

The pushed address is PC+2, which is one byte before the next instruction. RTS reads this address and increments it by 1, making the effective return address correct.

---

### LDA — Load Accumulator
```
A = M
```
Flags: **N Z**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Immediate | $A9 | 2 | 2 |
| Zero Page | $A5 | 2 | 3 |
| Zero Page,X | $B5 | 2 | 4 |
| Absolute | $AD | 3 | 4 |
| Absolute,X | $BD | 3 | 4+1 |
| Absolute,Y | $B9 | 3 | 4+1 |
| (Indirect,X) | $A1 | 2 | 6 |
| (Indirect),Y | $B1 | 2 | 5+1 |

---

### LDX — Load X Register
```
X = M
```
Flags: **N Z**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Immediate | $A2 | 2 | 2 |
| Zero Page | $A6 | 2 | 3 |
| Zero Page,Y | $B6 | 2 | 4 |
| Absolute | $AE | 3 | 4 |
| Absolute,Y | $BE | 3 | 4+1 |

Note: the indexed mode uses **Y**, not X.

---

### LDY — Load Y Register
```
Y = M
```
Flags: **N Z**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Immediate | $A0 | 2 | 2 |
| Zero Page | $A4 | 2 | 3 |
| Zero Page,X | $B4 | 2 | 4 |
| Absolute | $AC | 3 | 4 |
| Absolute,X | $BC | 3 | 4+1 |

---

### LSR — Logical Shift Right *(RMW)*
```
0 → [76543210] → C
```
Flags: **N** (always 0) **Z C**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Accumulator | $4A | 1 | 2 |
| Zero Page | $46 | 2 | 5 |
| Zero Page,X | $56 | 2 | 6 |
| Absolute | $4E | 3 | 6 |
| Absolute,X | $5E | 3 | 7 |

Bit 0 moves into C; 0 fills bit 7. N is always cleared. Equivalent to unsigned divide by 2.

---

### NOP — No Operation
```
(nothing)
```
Flags: none

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $EA | 1 | 2 |

---

### ORA — Bitwise OR
```
A = A | M
```
Flags: **N Z**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Immediate | $09 | 2 | 2 |
| Zero Page | $05 | 2 | 3 |
| Zero Page,X | $15 | 2 | 4 |
| Absolute | $0D | 3 | 4 |
| Absolute,X | $1D | 3 | 4+1 |
| Absolute,Y | $19 | 3 | 4+1 |
| (Indirect,X) | $01 | 2 | 6 |
| (Indirect),Y | $11 | 2 | 5+1 |

---

### PHA — Push Accumulator
```
[SP] = A, SP = SP - 1
```
Flags: none

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $48 | 1 | 3 |

---

### PHP — Push Processor Status
```
[SP] = P (with B=1, bit5=1), SP = SP - 1
```
Flags: none

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $08 | 1 | 3 |

The B flag (bit 4) and bit 5 are both pushed as 1. The real P register is not changed.

---

### PLA — Pull Accumulator
```
SP = SP + 1, A = [SP]
```
Flags: **N Z**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $68 | 1 | 4 |

---

### PLP — Pull Processor Status
```
SP = SP + 1, P = [SP]
```
Flags: **all** restored (I delayed by 1 instruction; B and bit 5 from stack are ignored)

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $28 | 1 | 4 |

Bits 4 (B) and 5 from the popped byte are discarded; the real B flag is not a physical flip-flop.

---

### ROL — Rotate Left *(RMW)*
```
C ← [76543210] ← C  (old C fills bit 0, old bit 7 goes to C)
```
Flags: **N Z C**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Accumulator | $2A | 1 | 2 |
| Zero Page | $26 | 2 | 5 |
| Zero Page,X | $36 | 2 | 6 |
| Absolute | $2E | 3 | 6 |
| Absolute,X | $3E | 3 | 7 |

---

### ROR — Rotate Right *(RMW)*
```
C → [76543210] → C  (old C fills bit 7, old bit 0 goes to C)
```
Flags: **N Z C**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Accumulator | $6A | 1 | 2 |
| Zero Page | $66 | 2 | 5 |
| Zero Page,X | $76 | 2 | 6 |
| Absolute | $6E | 3 | 6 |
| Absolute,X | $7E | 3 | 7 |

---

### RTI — Return from Interrupt
```
SP = SP+1, P = [SP]
SP = SP+1, PCL = [SP]
SP = SP+1, PCH = [SP]
```
Flags: **all** restored immediately (no delay, unlike PLP)

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $40 | 1 | 6 |

The PC popped is the exact address of the next instruction (unlike RTS, no +1). I flag takes effect immediately (no delay).

---

### RTS — Return from Subroutine
```
SP = SP+1, PCL = [SP]
SP = SP+1, PCH = [SP]
PC = PC + 1
```
Flags: none

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $60 | 1 | 6 |

Increments the pulled address by 1 to compensate for how JSR pushes PC+2 (one byte early).

---

### SBC — Subtract with Carry
```
A = A - M - (1 - C)   ≡   A = A + ~M + C
```
Flags: **N V Z C**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Immediate | $E9 | 2 | 2 |
| Zero Page | $E5 | 2 | 3 |
| Zero Page,X | $F5 | 2 | 4 |
| Absolute | $ED | 3 | 4 |
| Absolute,X | $FD | 3 | 4+1 |
| Absolute,Y | $F9 | 3 | 4+1 |
| (Indirect,X) | $E1 | 2 | 6 |
| (Indirect),Y | $F1 | 2 | 5+1 |

Call `SEC` before the first byte of a multi-byte subtract. C=1 after means no borrow (A ≥ M). Internally identical to `ADC` with M bit-inverted; implement as `ADC(M ^ 0xFF)`.

---

### SEC — Set Carry
```
C = 1
```
Flags: **C**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $38 | 1 | 2 |

---

### SED — Set Decimal
```
D = 1
```
Flags: **D**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $F8 | 1 | 2 |

BCD has no effect on the 2A03; D can be set/cleared but changes no arithmetic behavior.

---

### SEI — Set Interrupt Disable
```
I = 1
```
Flags: **I** (delayed by 1 instruction)

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $78 | 1 | 2 |

If I was 0, an IRQ pending at the moment SEI executes is still serviced after the next instruction.

---

### STA — Store Accumulator
```
M = A
```
Flags: none

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Zero Page | $85 | 2 | 3 |
| Zero Page,X | $95 | 2 | 4 |
| Absolute | $8D | 3 | 4 |
| Absolute,X | $9D | 3 | 5 |
| Absolute,Y | $99 | 3 | 5 |
| (Indirect,X) | $81 | 2 | 6 |
| (Indirect),Y | $91 | 2 | 6 |

Indexed modes always take the extra cycle (no page-cross shortcut, unlike loads).

---

### STX — Store X Register
```
M = X
```
Flags: none

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Zero Page | $86 | 2 | 3 |
| Zero Page,Y | $96 | 2 | 4 |
| Absolute | $8E | 3 | 4 |

Note: the indexed mode uses **Y**, not X.

---

### STY — Store Y Register
```
M = Y
```
Flags: none

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Zero Page | $84 | 2 | 3 |
| Zero Page,X | $94 | 2 | 4 |
| Absolute | $8C | 3 | 4 |

---

### TAX — Transfer A to X
```
X = A
```
Flags: **N Z**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $AA | 1 | 2 |

---

### TAY — Transfer A to Y
```
Y = A
```
Flags: **N Z**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $A8 | 1 | 2 |

---

### TSX — Transfer Stack Pointer to X
```
X = SP
```
Flags: **N Z**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $BA | 1 | 2 |

---

### TXA — Transfer X to A
```
A = X
```
Flags: **N Z**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $8A | 1 | 2 |

---

### TXS — Transfer X to Stack Pointer
```
SP = X
```
Flags: **none**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $9A | 1 | 2 |

Unlike TAX/TSX, this does **not** set N or Z.

---

### TYA — Transfer Y to A
```
A = Y
```
Flags: **N Z**

| Mode | Opcode | Bytes | Cycles |
|------|--------|-------|--------|
| Implied | $98 | 1 | 2 |

---

## Quick Opcode Table

| $x\ $y | $x0 | $x1 | $x2 | $x4 | $x5 | $x6 | $x8 | $x9 | $xA | $xC | $xD | $xE |
|--------|-----|-----|-----|-----|-----|-----|-----|-----|-----|-----|-----|-----|
| **$0x** | BRK | ORA (zp,X) | — | — | ORA zp | ASL zp | PHP | ORA # | ASL A | — | ORA abs | ASL abs |
| **$1x** | BPL rel | ORA (zp),Y | — | — | ORA zp,X | ASL zp,X | CLC | ORA abs,Y | — | — | ORA abs,X | ASL abs,X |
| **$2x** | JSR abs | AND (zp,X) | — | BIT zp | AND zp | ROL zp | PLP | AND # | ROL A | BIT abs | AND abs | ROL abs |
| **$3x** | BMI rel | AND (zp),Y | — | — | AND zp,X | ROL zp,X | SEC | AND abs,Y | — | — | AND abs,X | ROL abs,X |
| **$4x** | RTI | EOR (zp,X) | — | — | EOR zp | LSR zp | PHA | EOR # | LSR A | JMP abs | EOR abs | LSR abs |
| **$5x** | BVC rel | EOR (zp),Y | — | — | EOR zp,X | LSR zp,X | CLI | EOR abs,Y | — | — | EOR abs,X | LSR abs,X |
| **$6x** | RTS | ADC (zp,X) | — | — | ADC zp | ROR zp | PLA | ADC # | ROR A | JMP (abs) | ADC abs | ROR abs |
| **$7x** | BVS rel | ADC (zp),Y | — | — | ADC zp,X | ROR zp,X | SEI | ADC abs,Y | — | — | ADC abs,X | ROR abs,X |
| **$8x** | — | STA (zp,X) | STX # | STY zp | STA zp | STX zp | DEY | — | TXA | STY abs | STA abs | STX abs |
| **$9x** | BCC rel | STA (zp),Y | — | STY zp,X | STA zp,X | STX zp,Y | TYA | STA abs,Y | TXS | — | STA abs,X | — |
| **$Ax** | LDY # | LDA (zp,X) | LDX # | LDY zp | LDA zp | LDX zp | TAY | LDA # | TAX | LDY abs | LDA abs | LDX abs |
| **$Bx** | BCS rel | LDA (zp),Y | — | LDY zp,X | LDA zp,X | LDX zp,Y | CLV | LDA abs,Y | TSX | LDY abs,X | LDA abs,X | LDX abs,Y |
| **$Cx** | CPY # | CMP (zp,X) | — | CPY zp | CMP zp | DEC zp | INY | CMP # | DEX | CPY abs | CMP abs | DEC abs |
| **$Dx** | BNE rel | CMP (zp),Y | — | — | CMP zp,X | DEC zp,X | CLD | CMP abs,Y | — | — | CMP abs,X | DEC abs,X |
| **$Ex** | CPX # | SBC (zp,X) | — | CPX zp | SBC zp | INC zp | INX | SBC # | NOP | CPX abs | SBC abs | INC abs |
| **$Fx** | BEQ rel | SBC (zp),Y | — | — | SBC zp,X | INC zp,X | SED | SBC abs,Y | — | — | SBC abs,X | INC abs,X |

Cells marked `—` are unofficial/illegal opcodes not required for basic emulation.

---

## Implementation Notes

### Flag update helpers
Most instructions set N and Z the same way — write a shared helper:
```
fn set_nz(p: &mut u8, value: u8) {
    p = (p & !0b1000_0010)
      | (value & 0x80)          // N = bit 7
      | if value == 0 { 0x02 } else { 0 }; // Z
}
```

### Overflow (V) flag for ADC/SBC
V is set when the sign of the result is wrong for the signs of the inputs:
```
// ADC
let v = (!(a ^ m) & (a ^ result)) & 0x80 != 0;
// SBC (after m = m ^ 0xFF)
// same formula as ADC applied to the inverted operand
```

### Stack
The stack lives at `$0100..=$01FF`. SP points to the next free slot (pre-decrement push, post-increment pull):
```
push(val): mem[0x0100 | SP as u16] = val; SP = SP.wrapping_sub(1);
pull():    SP = SP.wrapping_add(1); mem[0x0100 | SP as u16]
```

### Branch cycle count
```
cycles = 2;
if branch_taken {
    cycles += 1;
    if (old_pc & 0xFF00) != (new_pc & 0xFF00) { cycles += 1; }
}
```

### Interrupt vectors

| Vector | Address | Triggered by |
|--------|---------|--------------|
| NMI | $FFFA/$FFFB | PPU V-blank (if enabled) |
| RESET | $FFFC/$FFFD | Power-on / reset pin |
| IRQ/BRK | $FFFE/$FFFF | IRQ line or BRK instruction |
