use super::TestBus;
use crate::cpu::{Cpu, FLAG_B, FLAG_C, FLAG_I, FLAG_N, FLAG_U, FLAG_V, FLAG_Z};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make() -> (Cpu, TestBus) {
    let mut cpu = Cpu::new();
    cpu.pc = 0x0200;
    (cpu, TestBus::new())
}

fn w(bus: &mut TestBus, addr: u16, bytes: &[u8]) {
    for (i, &b) in bytes.iter().enumerate() {
        bus.mem[addr as usize + i] = b;
    }
}

// ---------------------------------------------------------------------------
// LDA
// ---------------------------------------------------------------------------

#[test]
fn lda_imm_sets_a_and_flags() {
    let (mut cpu, mut bus) = make();
    w(&mut bus, 0x0200, &[0xA9, 0x42]); // LDA #$42
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x42);
    assert_eq!(cycles, 2);
    assert!(!cpu.flag(FLAG_Z));
    assert!(!cpu.flag(FLAG_N));
}

#[test]
fn lda_imm_zero_sets_z() {
    let (mut cpu, mut bus) = make();
    w(&mut bus, 0x0200, &[0xA9, 0x00]);
    cpu.step(&mut bus);
    assert!(cpu.flag(FLAG_Z));
    assert!(!cpu.flag(FLAG_N));
}

#[test]
fn lda_imm_negative_sets_n() {
    let (mut cpu, mut bus) = make();
    w(&mut bus, 0x0200, &[0xA9, 0x80]);
    cpu.step(&mut bus);
    assert!(!cpu.flag(FLAG_Z));
    assert!(cpu.flag(FLAG_N));
}

#[test]
fn lda_zp() {
    let (mut cpu, mut bus) = make();
    bus.mem[0x0010] = 0x37;
    w(&mut bus, 0x0200, &[0xA5, 0x10]); // LDA $10
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x37);
    assert_eq!(cycles, 3);
}

#[test]
fn lda_zp_x() {
    let (mut cpu, mut bus) = make();
    cpu.x = 0x05;
    bus.mem[0x0015] = 0xAB;
    w(&mut bus, 0x0200, &[0xB5, 0x10]); // LDA $10,X  → reads $15
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.a, 0xAB);
    assert_eq!(cycles, 4);
}

#[test]
fn lda_zp_x_wraps() {
    let (mut cpu, mut bus) = make();
    cpu.x = 0x01;
    bus.mem[0x0000] = 0xCC;
    w(&mut bus, 0x0200, &[0xB5, 0xFF]); // $FF + 1 wraps to $00
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0xCC);
}

#[test]
fn lda_abs() {
    let (mut cpu, mut bus) = make();
    bus.mem[0x0400] = 0x55;
    w(&mut bus, 0x0200, &[0xAD, 0x00, 0x04]); // LDA $0400
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x55);
    assert_eq!(cycles, 4);
}

#[test]
fn lda_abs_x_no_page_cross() {
    let (mut cpu, mut bus) = make();
    cpu.x = 0x01;
    bus.mem[0x0401] = 0x77;
    w(&mut bus, 0x0200, &[0xBD, 0x00, 0x04]); // LDA $0400,X
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x77);
    assert_eq!(cycles, 4);
}

#[test]
fn lda_abs_x_page_cross_adds_cycle() {
    let (mut cpu, mut bus) = make();
    cpu.x = 0x01;
    bus.mem[0x0500] = 0x99;
    w(&mut bus, 0x0200, &[0xBD, 0xFF, 0x04]); // LDA $04FF,X → $0500
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x99);
    assert_eq!(cycles, 5);
}

#[test]
fn lda_abs_y_page_cross_adds_cycle() {
    let (mut cpu, mut bus) = make();
    cpu.y = 0x02;
    bus.mem[0x0501] = 0x11;
    w(&mut bus, 0x0200, &[0xB9, 0xFF, 0x04]); // LDA $04FF,Y → $0501
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x11);
    assert_eq!(cycles, 5);
}

#[test]
fn lda_indirect_x() {
    let (mut cpu, mut bus) = make();
    cpu.x = 0x04;
    bus.mem[0x0014] = 0x00; // ptr lo
    bus.mem[0x0015] = 0x03; // ptr hi → $0300
    bus.mem[0x0300] = 0xBB;
    w(&mut bus, 0x0200, &[0xA1, 0x10]); // LDA ($10,X)
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.a, 0xBB);
    assert_eq!(cycles, 6);
}

#[test]
fn lda_indirect_y_no_page_cross() {
    let (mut cpu, mut bus) = make();
    cpu.y = 0x01;
    bus.mem[0x0020] = 0x00;
    bus.mem[0x0021] = 0x03; // base = $0300
    bus.mem[0x0301] = 0xDE;
    w(&mut bus, 0x0200, &[0xB1, 0x20]); // LDA ($20),Y
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.a, 0xDE);
    assert_eq!(cycles, 5);
}

#[test]
fn lda_indirect_y_page_cross() {
    let (mut cpu, mut bus) = make();
    cpu.y = 0x01;
    bus.mem[0x0020] = 0xFF;
    bus.mem[0x0021] = 0x03; // base = $03FF
    bus.mem[0x0400] = 0xEF;
    w(&mut bus, 0x0200, &[0xB1, 0x20]);
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.a, 0xEF);
    assert_eq!(cycles, 6);
}

// ---------------------------------------------------------------------------
// LDX / LDY
// ---------------------------------------------------------------------------

#[test]
fn ldx_imm() {
    let (mut cpu, mut bus) = make();
    w(&mut bus, 0x0200, &[0xA2, 0x10]);
    cpu.step(&mut bus);
    assert_eq!(cpu.x, 0x10);
}

#[test]
fn ldx_zp_y() {
    let (mut cpu, mut bus) = make();
    cpu.y = 0x02;
    bus.mem[0x0012] = 0x42;
    w(&mut bus, 0x0200, &[0xB6, 0x10]); // LDX $10,Y
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.x, 0x42);
    assert_eq!(cycles, 4);
}

#[test]
fn ldy_imm() {
    let (mut cpu, mut bus) = make();
    w(&mut bus, 0x0200, &[0xA0, 0x20]);
    cpu.step(&mut bus);
    assert_eq!(cpu.y, 0x20);
}

// ---------------------------------------------------------------------------
// STA / STX / STY
// ---------------------------------------------------------------------------

#[test]
fn sta_zp() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x5A;
    w(&mut bus, 0x0200, &[0x85, 0x30]);
    let cycles = cpu.step(&mut bus);
    assert_eq!(bus.mem[0x0030], 0x5A);
    assert_eq!(cycles, 3);
}

#[test]
fn sta_abs() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x7F;
    w(&mut bus, 0x0200, &[0x8D, 0x00, 0x04]);
    cpu.step(&mut bus);
    assert_eq!(bus.mem[0x0400], 0x7F);
}

#[test]
fn stx_zp() {
    let (mut cpu, mut bus) = make();
    cpu.x = 0x11;
    w(&mut bus, 0x0200, &[0x86, 0x40]);
    cpu.step(&mut bus);
    assert_eq!(bus.mem[0x0040], 0x11);
}

#[test]
fn sty_zp() {
    let (mut cpu, mut bus) = make();
    cpu.y = 0x22;
    w(&mut bus, 0x0200, &[0x84, 0x50]);
    cpu.step(&mut bus);
    assert_eq!(bus.mem[0x0050], 0x22);
}

// ---------------------------------------------------------------------------
// Transfers
// ---------------------------------------------------------------------------

#[test]
fn tax_transfers_and_sets_flags() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x80;
    w(&mut bus, 0x0200, &[0xAA]);
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.x, 0x80);
    assert!(cpu.flag(FLAG_N));
    assert!(!cpu.flag(FLAG_Z));
    assert_eq!(cycles, 2);
}

#[test]
fn tay() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x00;
    w(&mut bus, 0x0200, &[0xA8]);
    cpu.step(&mut bus);
    assert_eq!(cpu.y, 0x00);
    assert!(cpu.flag(FLAG_Z));
}

#[test]
fn txa() {
    let (mut cpu, mut bus) = make();
    cpu.x = 0x55;
    w(&mut bus, 0x0200, &[0x8A]);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x55);
}

#[test]
fn tya() {
    let (mut cpu, mut bus) = make();
    cpu.y = 0x33;
    w(&mut bus, 0x0200, &[0x98]);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x33);
}

#[test]
fn tsx() {
    let (mut cpu, mut bus) = make();
    cpu.sp = 0xFD;
    w(&mut bus, 0x0200, &[0xBA]);
    cpu.step(&mut bus);
    assert_eq!(cpu.x, 0xFD);
    assert!(cpu.flag(FLAG_N));
}

#[test]
fn txs_no_flags() {
    let (mut cpu, mut bus) = make();
    cpu.x = 0x42;
    cpu.p = FLAG_U | FLAG_I; // clear N,Z
    w(&mut bus, 0x0200, &[0x9A]);
    cpu.step(&mut bus);
    assert_eq!(cpu.sp, 0x42);
    assert!(!cpu.flag(FLAG_N));
    assert!(!cpu.flag(FLAG_Z));
}

// ---------------------------------------------------------------------------
// ADC
// ---------------------------------------------------------------------------

#[test]
fn adc_no_carry_in_or_out() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x10;
    w(&mut bus, 0x0200, &[0x69, 0x20]); // ADC #$20
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x30);
    assert!(!cpu.flag(FLAG_C));
    assert!(!cpu.flag(FLAG_V));
}

#[test]
fn adc_carry_out() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0xFF;
    w(&mut bus, 0x0200, &[0x69, 0x01]);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.flag(FLAG_C));
    assert!(cpu.flag(FLAG_Z));
}

#[test]
fn adc_carry_in() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x10;
    cpu.p |= FLAG_C;
    w(&mut bus, 0x0200, &[0x69, 0x10]);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x21);
    assert!(!cpu.flag(FLAG_C));
}

#[test]
fn adc_signed_overflow_positive() {
    // 0x7F + 0x01 = 0x80 — positive + positive = negative → overflow
    let (mut cpu, mut bus) = make();
    cpu.a = 0x7F;
    w(&mut bus, 0x0200, &[0x69, 0x01]);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x80);
    assert!(cpu.flag(FLAG_V));
    assert!(cpu.flag(FLAG_N));
}

#[test]
fn adc_signed_overflow_negative() {
    // 0x80 + 0x80 = 0x00 with carry — negative + negative = positive → overflow
    let (mut cpu, mut bus) = make();
    cpu.a = 0x80;
    w(&mut bus, 0x0200, &[0x69, 0x80]);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.flag(FLAG_V));
    assert!(cpu.flag(FLAG_C));
}

#[test]
fn adc_no_overflow_when_signs_differ() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x50;
    w(&mut bus, 0x0200, &[0x69, 0xD0]); // 0x50 + (-48) = 8, no overflow
    cpu.step(&mut bus);
    assert!(!cpu.flag(FLAG_V));
}

// ---------------------------------------------------------------------------
// SBC
// ---------------------------------------------------------------------------

#[test]
fn sbc_simple() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x50;
    cpu.p |= FLAG_C; // borrow = 0
    w(&mut bus, 0x0200, &[0xE9, 0x10]); // SBC #$10
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x40);
    assert!(cpu.flag(FLAG_C)); // no borrow
    assert!(!cpu.flag(FLAG_V));
}

#[test]
fn sbc_borrow() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x00;
    cpu.p |= FLAG_C;
    w(&mut bus, 0x0200, &[0xE9, 0x01]);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0xFF);
    assert!(!cpu.flag(FLAG_C)); // borrow occurred
    assert!(cpu.flag(FLAG_N));
}

#[test]
fn sbc_overflow() {
    // 0x50 - 0xB0 — positive minus negative should overflow to negative
    let (mut cpu, mut bus) = make();
    cpu.a = 0x50;
    cpu.p |= FLAG_C;
    w(&mut bus, 0x0200, &[0xE9, 0xB0]);
    cpu.step(&mut bus);
    assert!(cpu.flag(FLAG_V));
}

// ---------------------------------------------------------------------------
// AND / ORA / EOR
// ---------------------------------------------------------------------------

#[test]
fn and_imm() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0xF0;
    w(&mut bus, 0x0200, &[0x29, 0x0F]);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.flag(FLAG_Z));
}

#[test]
fn ora_imm() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x0F;
    w(&mut bus, 0x0200, &[0x09, 0xF0]);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0xFF);
    assert!(cpu.flag(FLAG_N));
}

#[test]
fn eor_imm() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0xFF;
    w(&mut bus, 0x0200, &[0x49, 0xFF]);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.flag(FLAG_Z));
}

// ---------------------------------------------------------------------------
// ASL / LSR / ROL / ROR
// ---------------------------------------------------------------------------

#[test]
fn asl_acc() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x81;
    w(&mut bus, 0x0200, &[0x0A]);
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x02);
    assert!(cpu.flag(FLAG_C));
    assert!(!cpu.flag(FLAG_N));
    assert_eq!(cycles, 2);
}

#[test]
fn asl_zp() {
    let (mut cpu, mut bus) = make();
    bus.mem[0x0010] = 0x40;
    w(&mut bus, 0x0200, &[0x06, 0x10]);
    let cycles = cpu.step(&mut bus);
    assert_eq!(bus.mem[0x0010], 0x80);
    assert!(cpu.flag(FLAG_N));
    assert_eq!(cycles, 5);
}

#[test]
fn lsr_acc() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x03;
    w(&mut bus, 0x0200, &[0x4A]);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x01);
    assert!(cpu.flag(FLAG_C));
}

#[test]
fn rol_acc_no_carry() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x80;
    cpu.p &= !FLAG_C;
    w(&mut bus, 0x0200, &[0x2A]);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.flag(FLAG_C));
    assert!(cpu.flag(FLAG_Z));
}

#[test]
fn rol_acc_with_carry() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x40;
    cpu.p |= FLAG_C;
    w(&mut bus, 0x0200, &[0x2A]);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x81);
    assert!(!cpu.flag(FLAG_C));
    assert!(cpu.flag(FLAG_N));
}

#[test]
fn ror_acc() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x01;
    cpu.p |= FLAG_C;
    w(&mut bus, 0x0200, &[0x6A]);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x80);
    assert!(cpu.flag(FLAG_C));
    assert!(cpu.flag(FLAG_N));
}

// ---------------------------------------------------------------------------
// INC / DEC / INX / DEX / INY / DEY
// ---------------------------------------------------------------------------

#[test]
fn inc_zp() {
    let (mut cpu, mut bus) = make();
    bus.mem[0x0020] = 0x7F;
    w(&mut bus, 0x0200, &[0xE6, 0x20]);
    let cycles = cpu.step(&mut bus);
    assert_eq!(bus.mem[0x0020], 0x80);
    assert!(cpu.flag(FLAG_N));
    assert_eq!(cycles, 5);
}

#[test]
fn inc_wraps() {
    let (mut cpu, mut bus) = make();
    bus.mem[0x0020] = 0xFF;
    w(&mut bus, 0x0200, &[0xE6, 0x20]);
    cpu.step(&mut bus);
    assert_eq!(bus.mem[0x0020], 0x00);
    assert!(cpu.flag(FLAG_Z));
}

#[test]
fn dec_zp() {
    let (mut cpu, mut bus) = make();
    bus.mem[0x0030] = 0x01;
    w(&mut bus, 0x0200, &[0xC6, 0x30]);
    cpu.step(&mut bus);
    assert_eq!(bus.mem[0x0030], 0x00);
    assert!(cpu.flag(FLAG_Z));
}

#[test]
fn inx_wraps() {
    let (mut cpu, mut bus) = make();
    cpu.x = 0xFF;
    w(&mut bus, 0x0200, &[0xE8]);
    cpu.step(&mut bus);
    assert_eq!(cpu.x, 0x00);
    assert!(cpu.flag(FLAG_Z));
}

#[test]
fn dex() {
    let (mut cpu, mut bus) = make();
    cpu.x = 0x01;
    w(&mut bus, 0x0200, &[0xCA]);
    cpu.step(&mut bus);
    assert_eq!(cpu.x, 0x00);
    assert!(cpu.flag(FLAG_Z));
}

#[test]
fn iny() {
    let (mut cpu, mut bus) = make();
    cpu.y = 0x7F;
    w(&mut bus, 0x0200, &[0xC8]);
    cpu.step(&mut bus);
    assert_eq!(cpu.y, 0x80);
    assert!(cpu.flag(FLAG_N));
}

#[test]
fn dey() {
    let (mut cpu, mut bus) = make();
    cpu.y = 0x00;
    w(&mut bus, 0x0200, &[0x88]);
    cpu.step(&mut bus);
    assert_eq!(cpu.y, 0xFF);
    assert!(cpu.flag(FLAG_N));
}

// ---------------------------------------------------------------------------
// CMP / CPX / CPY
// ---------------------------------------------------------------------------

#[test]
fn cmp_equal() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x42;
    w(&mut bus, 0x0200, &[0xC9, 0x42]);
    cpu.step(&mut bus);
    assert!(cpu.flag(FLAG_Z));
    assert!(cpu.flag(FLAG_C));
    assert!(!cpu.flag(FLAG_N));
}

#[test]
fn cmp_greater() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x10;
    w(&mut bus, 0x0200, &[0xC9, 0x05]);
    cpu.step(&mut bus);
    assert!(!cpu.flag(FLAG_Z));
    assert!(cpu.flag(FLAG_C));
    assert!(!cpu.flag(FLAG_N));
}

#[test]
fn cmp_less() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x05;
    w(&mut bus, 0x0200, &[0xC9, 0x10]);
    cpu.step(&mut bus);
    assert!(!cpu.flag(FLAG_Z));
    assert!(!cpu.flag(FLAG_C));
    assert!(cpu.flag(FLAG_N));
}

#[test]
fn cpx_imm() {
    let (mut cpu, mut bus) = make();
    cpu.x = 0x20;
    w(&mut bus, 0x0200, &[0xE0, 0x20]);
    cpu.step(&mut bus);
    assert!(cpu.flag(FLAG_Z));
    assert!(cpu.flag(FLAG_C));
}

#[test]
fn cpy_imm() {
    let (mut cpu, mut bus) = make();
    cpu.y = 0x01;
    w(&mut bus, 0x0200, &[0xC0, 0x02]);
    cpu.step(&mut bus);
    assert!(!cpu.flag(FLAG_C));
}

// ---------------------------------------------------------------------------
// BIT
// ---------------------------------------------------------------------------

#[test]
fn bit_zp_sets_n_v_from_memory() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0xFF;
    bus.mem[0x0010] = 0xC0; // bits 6 and 7 set
    w(&mut bus, 0x0200, &[0x24, 0x10]);
    let cycles = cpu.step(&mut bus);
    assert!(cpu.flag(FLAG_N));
    assert!(cpu.flag(FLAG_V));
    assert!(!cpu.flag(FLAG_Z));
    assert_eq!(cycles, 3);
}

#[test]
fn bit_zp_sets_z_when_and_zero() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x0F;
    bus.mem[0x0010] = 0xF0;
    w(&mut bus, 0x0200, &[0x24, 0x10]);
    cpu.step(&mut bus);
    assert!(cpu.flag(FLAG_Z));
}

// ---------------------------------------------------------------------------
// Flag instructions
// ---------------------------------------------------------------------------

#[test]
fn clc_sec() {
    let (mut cpu, mut bus) = make();
    cpu.p |= FLAG_C;
    w(&mut bus, 0x0200, &[0x18, 0x38]); // CLC, SEC
    cpu.step(&mut bus);
    assert!(!cpu.flag(FLAG_C));
    cpu.step(&mut bus);
    assert!(cpu.flag(FLAG_C));
}

#[test]
fn cli_sei() {
    let (mut cpu, mut bus) = make();
    w(&mut bus, 0x0200, &[0x58, 0x78]); // CLI, SEI
    cpu.step(&mut bus);
    assert!(!cpu.flag(FLAG_I));
    cpu.step(&mut bus);
    assert!(cpu.flag(FLAG_I));
}

#[test]
fn clv() {
    let (mut cpu, mut bus) = make();
    cpu.p |= FLAG_V;
    w(&mut bus, 0x0200, &[0xB8]);
    cpu.step(&mut bus);
    assert!(!cpu.flag(FLAG_V));
}

// ---------------------------------------------------------------------------
// Branches
// ---------------------------------------------------------------------------

#[test]
fn bne_not_taken() {
    let (mut cpu, mut bus) = make();
    cpu.p |= FLAG_Z; // Z set → BNE not taken
    w(&mut bus, 0x0200, &[0xD0, 0x10]);
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.pc, 0x0202);
    assert_eq!(cycles, 2);
}

#[test]
fn bne_taken_same_page() {
    let (mut cpu, mut bus) = make();
    cpu.p &= !FLAG_Z;
    w(&mut bus, 0x0200, &[0xD0, 0x10]); // branch to 0x0212
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.pc, 0x0212);
    assert_eq!(cycles, 3);
}

#[test]
fn bne_taken_negative_offset() {
    let (mut cpu, mut bus) = make();
    cpu.p &= !FLAG_Z;
    w(&mut bus, 0x0200, &[0xD0, 0xFE]); // -2 → branch back to 0x0200
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.pc, 0x0200);
    assert_eq!(cycles, 3);
}

#[test]
fn beq_taken_page_cross() {
    // Instruction at $02FE consumes to PC=$0300; offset -128 targets $0280 → page 3→2 cross
    let (mut cpu, mut bus) = make();
    cpu.pc = 0x02FE;
    cpu.p |= FLAG_Z;
    w(&mut bus, 0x02FE, &[0xF0, 0x80]); // BEQ -128 → $0280
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.pc, 0x0280);
    assert_eq!(cycles, 4);
}

#[test]
fn bcc_taken() {
    let (mut cpu, mut bus) = make();
    cpu.p &= !FLAG_C;
    w(&mut bus, 0x0200, &[0x90, 0x04]); // BCC +4 → $0206
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.pc, 0x0206);
    assert_eq!(cycles, 3);
}

#[test]
fn bcs_not_taken() {
    let (mut cpu, mut bus) = make();
    cpu.p &= !FLAG_C;
    w(&mut bus, 0x0200, &[0xB0, 0x10]); // BCS +16, C=0 → not taken
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.pc, 0x0202);
    assert_eq!(cycles, 2);
}

#[test]
fn bmi_bpl() {
    let (mut cpu, mut bus) = make();
    cpu.p |= FLAG_N;
    w(&mut bus, 0x0200, &[0x30, 0x04]); // BMI +4
    cpu.step(&mut bus);
    assert_eq!(cpu.pc, 0x0206);
}

#[test]
fn bvc_bvs() {
    let (mut cpu, mut bus) = make();
    cpu.p &= !FLAG_V;
    w(&mut bus, 0x0200, &[0x50, 0x04]); // BVC +4
    cpu.step(&mut bus);
    assert_eq!(cpu.pc, 0x0206);
}

// ---------------------------------------------------------------------------
// JMP
// ---------------------------------------------------------------------------

#[test]
fn jmp_absolute() {
    let (mut cpu, mut bus) = make();
    w(&mut bus, 0x0200, &[0x4C, 0x00, 0x04]); // JMP $0400
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.pc, 0x0400);
    assert_eq!(cycles, 3);
}

#[test]
fn jmp_indirect() {
    let (mut cpu, mut bus) = make();
    bus.mem[0x0400] = 0x00;
    bus.mem[0x0401] = 0x05; // → $0500
    w(&mut bus, 0x0200, &[0x6C, 0x00, 0x04]); // JMP ($0400)
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.pc, 0x0500);
    assert_eq!(cycles, 5);
}

#[test]
fn jmp_indirect_page_wrap_bug() {
    // JMP ($02FF): lo from $02FF, hi from $0200 (not $0300)
    let (mut cpu, mut bus) = make();
    bus.mem[0x02FF] = 0x80; // lo
    bus.mem[0x0200] = 0x04; // hi (bugged — wraps within page)
    bus.mem[0x0300] = 0xFF; // should NOT be read
    w(&mut bus, 0x0200, &[0x6C, 0xFF, 0x02]);
    cpu.step(&mut bus);
    // The JMP instruction reads from $0200 for hi, but our program is also at $0200.
    // Let's place it at a different PC.
    let (mut cpu2, mut bus2) = make();
    cpu2.pc = 0x0100;
    bus2.mem[0x02FF] = 0x80;
    bus2.mem[0x0200] = 0x04; // hi from wrapped address
    bus2.mem[0x0300] = 0xFF; // NOT read
    w(&mut bus2, 0x0100, &[0x6C, 0xFF, 0x02]);
    cpu2.step(&mut bus2);
    assert_eq!(cpu2.pc, 0x0480);
}

// ---------------------------------------------------------------------------
// JSR / RTS
// ---------------------------------------------------------------------------

#[test]
fn jsr_rts_roundtrip() {
    let (mut cpu, mut bus) = make();
    // JSR $0400
    w(&mut bus, 0x0200, &[0x20, 0x00, 0x04]);
    // RTS at $0400
    bus.mem[0x0400] = 0x60;
    let sp_before = cpu.sp;
    cpu.step(&mut bus); // JSR
    assert_eq!(cpu.pc, 0x0400);
    assert_eq!(cpu.sp, sp_before.wrapping_sub(2));
    cpu.step(&mut bus); // RTS
    assert_eq!(cpu.pc, 0x0203); // returns to instruction after JSR
    assert_eq!(cpu.sp, sp_before);
}

#[test]
fn jsr_pushes_pc_minus_one() {
    let (mut cpu, mut bus) = make();
    w(&mut bus, 0x0200, &[0x20, 0x00, 0x04]);
    let sp = cpu.sp;
    cpu.step(&mut bus);
    let hi = bus.mem[0x0100 + sp as usize];
    let lo = bus.mem[0x0100 + sp.wrapping_sub(1) as usize];
    let saved_pc = ((hi as u16) << 8) | lo as u16;
    assert_eq!(saved_pc, 0x0202); // PC after JSR operand, minus 1
}

// Real 6502 hardware drives the address bus on every cycle of every
// instruction, including cycles whose result is discarded ("internal
// operations"). JSR's cycle 3 is a dummy read of the current stack address
// before the two pushes. Every CPU cycle must correspond to exactly one
// `Bus::read`/`Bus::write` call — the DMC DMA halt-detection logic hooks
// into those calls to decide when to steal a cycle, so a cycle with no bus
// access is invisible to it and the DMA timing drifts (see the DMC DMA
// cycle accuracy note in CLAUDE.md).
#[test]
fn jsr_bus_access_sequence() {
    let (mut cpu, mut bus) = make();
    w(&mut bus, 0x0200, &[0x20, 0x00, 0x04]); // JSR $0400
    let sp = cpu.sp; // 0xFD
    bus.trace.clear();
    cpu.step(&mut bus);
    assert_eq!(
        bus.trace,
        vec![
            (0x0200, false),                            // T1 opcode fetch
            (0x0201, false),                            // T2 fetch ADL
            (0x0100 | sp as u16, false),                // T3 dummy internal stack read
            (0x0100 | sp as u16, true),                 // T4 push PCH
            (0x0100 | sp.wrapping_sub(1) as u16, true), // T5 push PCL
            (0x0202, false),                            // T6 fetch ADH
        ]
    );
    assert_eq!(cpu.pc, 0x0400);
}

#[test]
fn rts_bus_access_sequence() {
    let (mut cpu, mut bus) = make();
    w(&mut bus, 0x0200, &[0x20, 0x00, 0x04]); // JSR $0400
    bus.mem[0x0400] = 0x60; // RTS
    cpu.step(&mut bus); // JSR
    let sp = cpu.sp; // 0xFB
    bus.trace.clear();
    cpu.step(&mut bus); // RTS
    assert_eq!(
        bus.trace,
        vec![
            (0x0400, false),                             // T1 opcode fetch
            (0x0401, false),                             // T2 dummy fetch of next byte
            (0x0100 | sp as u16, false),                 // T3 dummy internal S++ read
            (0x0100 | sp.wrapping_add(1) as u16, false), // T4 pop PCL
            (0x0100 | sp.wrapping_add(2) as u16, false), // T5 pop PCH
            (0x0202, false), // T6 dummy read at popped PC (pre-increment)
        ]
    );
    assert_eq!(cpu.pc, 0x0203);
}

// ---------------------------------------------------------------------------
// Stack: PHA / PLA / PHP / PLP
// ---------------------------------------------------------------------------

#[test]
fn pha_pla() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0xAB;
    w(&mut bus, 0x0200, &[0x48, 0xA9, 0x00, 0x68]); // PHA, LDA #0, PLA
    let sp = cpu.sp;
    cpu.step(&mut bus); // PHA
    assert_eq!(bus.mem[0x0100 + sp as usize], 0xAB);
    cpu.step(&mut bus); // LDA #0
    assert_eq!(cpu.a, 0x00);
    cpu.step(&mut bus); // PLA
    assert_eq!(cpu.a, 0xAB);
    assert!(!cpu.flag(FLAG_Z));
    assert!(cpu.flag(FLAG_N));
}

#[test]
fn php_sets_b_and_u_on_stack() {
    let (mut cpu, mut bus) = make();
    cpu.p = FLAG_U | FLAG_I;
    w(&mut bus, 0x0200, &[0x08]); // PHP
    let sp = cpu.sp;
    cpu.step(&mut bus);
    let stacked = bus.mem[0x0100 + sp as usize];
    assert_ne!(stacked & FLAG_B, 0);
    assert_ne!(stacked & FLAG_U, 0);
}

#[test]
fn plp_clears_b_sets_u() {
    let (mut cpu, mut bus) = make();
    // Push a value with B set; PLP should clear it
    w(&mut bus, 0x0200, &[0x28]); // PLP
    let sp = cpu.sp;
    cpu.sp = sp.wrapping_sub(1);
    bus.mem[0x0100 + sp as usize] = FLAG_B | FLAG_C | FLAG_U;
    cpu.step(&mut bus);
    assert!(!cpu.flag(FLAG_B));
    assert!(cpu.flag(FLAG_C));
    assert!(cpu.flag(FLAG_U));
}

#[test]
fn pha_bus_access_sequence() {
    // PHA is already cycle-accurate (T1 opcode, T2 dispatcher dummy fetch,
    // T3 push) — regression check for the JSR/RTS/RTI/PLA/PLP silent-cycle
    // audit rather than a fix target.
    let (mut cpu, mut bus) = make();
    w(&mut bus, 0x0200, &[0x48]); // PHA
    let sp = cpu.sp;
    bus.trace.clear();
    cpu.step(&mut bus);
    assert_eq!(
        bus.trace,
        vec![(0x0200, false), (0x0201, false), (0x0100 | sp as u16, true)]
    );
}

#[test]
fn pla_bus_access_sequence() {
    let (mut cpu, mut bus) = make();
    w(&mut bus, 0x0200, &[0x68]); // PLA
    cpu.push(&mut bus, 0x42);
    let sp = cpu.sp;
    bus.trace.clear();
    cpu.step(&mut bus);
    assert_eq!(
        bus.trace,
        vec![
            (0x0200, false),                             // T1 opcode fetch
            (0x0201, false),                             // T2 dummy fetch of next byte
            (0x0100 | sp as u16, false),                 // T3 dummy internal S++ read
            (0x0100 | sp.wrapping_add(1) as u16, false), // T4 pop A
        ]
    );
    assert_eq!(cpu.a, 0x42);
}

#[test]
fn plp_bus_access_sequence() {
    let (mut cpu, mut bus) = make();
    w(&mut bus, 0x0200, &[0x28]); // PLP
    cpu.push(&mut bus, FLAG_C);
    let sp = cpu.sp;
    bus.trace.clear();
    cpu.step(&mut bus);
    assert_eq!(
        bus.trace,
        vec![
            (0x0200, false),
            (0x0201, false),
            (0x0100 | sp as u16, false),
            (0x0100 | sp.wrapping_add(1) as u16, false),
        ]
    );
    assert!(cpu.flag(FLAG_C));
}

// ---------------------------------------------------------------------------
// BRK / RTI
// ---------------------------------------------------------------------------

#[test]
fn brk_pushes_state_and_sets_i() {
    let (mut cpu, mut bus) = make();
    cpu.p = FLAG_U;
    bus.mem[0xFFFE] = 0x00;
    bus.mem[0xFFFF] = 0x04; // IRQ vector → $0400
    w(&mut bus, 0x0200, &[0x00, 0x00]); // BRK + padding
    let sp = cpu.sp;
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.pc, 0x0400);
    assert!(cpu.flag(FLAG_I));
    assert_eq!(cycles, 7);
    // Stacked P should have B set
    let stacked_p = bus.mem[0x0100 + sp.wrapping_sub(2) as usize];
    assert_ne!(stacked_p & FLAG_B, 0);
    // Stacked PC should be 0x0202 (past padding byte)
    let hi = bus.mem[0x0100 + sp as usize] as u16;
    let lo = bus.mem[0x0100 + sp.wrapping_sub(1) as usize] as u16;
    assert_eq!((hi << 8) | lo, 0x0202);
}

#[test]
fn rti_restores_state() {
    let (mut cpu, mut bus) = make();
    // RTI pops: P (first), then PC lo, then PC hi
    // With sp=0xFA, pop increments sp before read:
    //   pop P  → sp=0xFB, reads mem[0x01FB]
    //   pop lo → sp=0xFC, reads mem[0x01FC]
    //   pop hi → sp=0xFD, reads mem[0x01FD]
    cpu.sp = 0xFA;
    bus.mem[0x01FB] = FLAG_U | FLAG_C; // P
    bus.mem[0x01FC] = 0x02; // PC lo
    bus.mem[0x01FD] = 0x03; // PC hi → $0302
    w(&mut bus, 0x0200, &[0x40]); // RTI
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.pc, 0x0302);
    assert!(cpu.flag(FLAG_C));
    assert!(!cpu.flag(FLAG_B));
    assert_eq!(cycles, 6);
}

#[test]
fn rti_bus_access_sequence() {
    let (mut cpu, mut bus) = make();
    cpu.sp = 0xFA;
    bus.mem[0x01FB] = FLAG_U | FLAG_C; // P
    bus.mem[0x01FC] = 0x02; // PC lo
    bus.mem[0x01FD] = 0x03; // PC hi -> $0302
    w(&mut bus, 0x0200, &[0x40]); // RTI
    let sp = cpu.sp; // 0xFA
    bus.trace.clear();
    cpu.step(&mut bus);
    assert_eq!(
        bus.trace,
        vec![
            (0x0200, false),                             // T1 opcode fetch
            (0x0201, false),                             // T2 dummy fetch of next byte
            (0x0100 | sp as u16, false),                 // T3 dummy internal S++ read
            (0x0100 | sp.wrapping_add(1) as u16, false), // T4 pop P
            (0x0100 | sp.wrapping_add(2) as u16, false), // T5 pop PCL
            (0x0100 | sp.wrapping_add(3) as u16, false), // T6 pop PCH
        ]
    );
    assert_eq!(cpu.pc, 0x0302);
}

// ---------------------------------------------------------------------------
// NOP
// ---------------------------------------------------------------------------

#[test]
fn nop() {
    let (mut cpu, mut bus) = make();
    w(&mut bus, 0x0200, &[0xEA]);
    let pc_before = cpu.pc;
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.pc, pc_before + 1);
    assert_eq!(cycles, 2);
}

// ---------------------------------------------------------------------------
// Stack wrap-around
// ---------------------------------------------------------------------------

#[test]
fn stack_wraps_at_page_boundary() {
    let (mut cpu, mut bus) = make();
    cpu.sp = 0x00;
    cpu.a = 0x55;
    w(&mut bus, 0x0200, &[0x48]); // PHA — sp wraps to 0xFF
    cpu.step(&mut bus);
    assert_eq!(cpu.sp, 0xFF);
    assert_eq!(bus.mem[0x0100], 0x55);
}

// ---------------------------------------------------------------------------
// Unofficial opcodes
// ---------------------------------------------------------------------------

#[test]
fn lax_loads_a_and_x() {
    let (mut cpu, mut bus) = make();
    bus.mem[0x0030] = 0x77;
    w(&mut bus, 0x0200, &[0xA7, 0x30]); // LAX $30
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x77);
    assert_eq!(cpu.x, 0x77);
    assert_eq!(cycles, 3);
}

#[test]
fn lax_imm() {
    let (mut cpu, mut bus) = make();
    w(&mut bus, 0x0200, &[0xAB, 0x42]); // LAX #$42
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x42);
    assert_eq!(cpu.x, 0x42);
}

#[test]
fn sax_stores_a_and_x() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0xF0;
    cpu.x = 0x3C;
    w(&mut bus, 0x0200, &[0x87, 0x50]); // SAX $50 — stores A & X
    let cycles = cpu.step(&mut bus);
    assert_eq!(bus.mem[0x0050], 0x30);
    assert_eq!(cycles, 3);
    // SAX should not alter A or X
    assert_eq!(cpu.a, 0xF0);
    assert_eq!(cpu.x, 0x3C);
}

#[test]
fn dcp_decrements_then_compares() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x10;
    bus.mem[0x0040] = 0x11;
    w(&mut bus, 0x0200, &[0xC7, 0x40]); // DCP $40
    let cycles = cpu.step(&mut bus);
    assert_eq!(bus.mem[0x0040], 0x10); // decremented
    assert!(cpu.flag(FLAG_Z)); // A == M after dec
    assert!(cpu.flag(FLAG_C));
    assert_eq!(cycles, 5);
}

#[test]
fn isc_increments_then_sbcs() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x20;
    cpu.p |= FLAG_C;
    bus.mem[0x0040] = 0x0F;
    w(&mut bus, 0x0200, &[0xE7, 0x40]); // ISC $40
    let cycles = cpu.step(&mut bus);
    assert_eq!(bus.mem[0x0040], 0x10);
    assert_eq!(cpu.a, 0x10); // 0x20 - 0x10 = 0x10
    assert_eq!(cycles, 5);
}

#[test]
fn slo_shifts_then_oras() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x01;
    bus.mem[0x0050] = 0x40;
    w(&mut bus, 0x0200, &[0x07, 0x50]); // SLO $50
    cpu.step(&mut bus);
    assert_eq!(bus.mem[0x0050], 0x80); // ASL
    assert_eq!(cpu.a, 0x81); // ORA
    assert!(cpu.flag(FLAG_N));
}

#[test]
fn sre_shifts_then_eors() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0xFF;
    bus.mem[0x0050] = 0xFF;
    w(&mut bus, 0x0200, &[0x47, 0x50]); // SRE $50
    cpu.step(&mut bus);
    assert_eq!(bus.mem[0x0050], 0x7F); // LSR of 0xFF
    assert_eq!(cpu.a, 0x80); // 0xFF ^ 0x7F
    assert!(cpu.flag(FLAG_N));
    assert!(cpu.flag(FLAG_C)); // LSR shifted out bit 0
}

#[test]
fn rla_rotates_then_ands() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0xFF;
    cpu.p &= !FLAG_C;
    bus.mem[0x0050] = 0x80;
    w(&mut bus, 0x0200, &[0x27, 0x50]); // RLA $50
    cpu.step(&mut bus);
    assert_eq!(bus.mem[0x0050], 0x00); // ROL 0x80 with C=0 → 0x00, C=1
    assert_eq!(cpu.a, 0x00); // AND with 0x00
    assert!(cpu.flag(FLAG_Z));
    assert!(cpu.flag(FLAG_C));
}

#[test]
fn rra_rotates_then_adcs() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0x00;
    cpu.p |= FLAG_C;
    bus.mem[0x0050] = 0x00;
    w(&mut bus, 0x0200, &[0x67, 0x50]); // RRA $50
    cpu.step(&mut bus);
    // ROR 0x00 with C=1 → 0x80, new C=0; ADC 0x00 + 0x80 = 0x80
    assert_eq!(bus.mem[0x0050], 0x80);
    assert_eq!(cpu.a, 0x80);
    assert!(cpu.flag(FLAG_N));
}

#[test]
fn anc_copies_n_to_c() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0xFF;
    w(&mut bus, 0x0200, &[0x0B, 0x80]); // ANC #$80
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x80);
    assert!(cpu.flag(FLAG_C));
    assert!(cpu.flag(FLAG_N));
}

#[test]
fn alr_ands_then_lsrs() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0xFF;
    w(&mut bus, 0x0200, &[0x4B, 0x55]); // ALR #$55
    cpu.step(&mut bus);
    // AND: 0xFF & 0x55 = 0x55; LSR: 0x55 >> 1 = 0x2A, C = 1
    assert_eq!(cpu.a, 0x2A);
    assert!(cpu.flag(FLAG_C));
    assert!(!cpu.flag(FLAG_N));
}

#[test]
fn axs_subtracts_imm_from_a_and_x() {
    let (mut cpu, mut bus) = make();
    cpu.a = 0xFF;
    cpu.x = 0x0F;
    w(&mut bus, 0x0200, &[0xCB, 0x05]); // AXS #$05 — X = (A&X) - imm
    cpu.step(&mut bus);
    assert_eq!(cpu.x, 0x0A); // 0x0F - 0x05
    assert!(cpu.flag(FLAG_C));
}

#[test]
fn unofficial_nop_1byte() {
    let (mut cpu, mut bus) = make();
    w(&mut bus, 0x0200, &[0x1A]); // 1-byte NOP
    let pc = cpu.pc;
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.pc, pc + 1);
    assert_eq!(cycles, 2);
}

#[test]
fn unofficial_nop_2byte() {
    let (mut cpu, mut bus) = make();
    w(&mut bus, 0x0200, &[0x80, 0x42]); // DOP #$42
    let pc = cpu.pc;
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.pc, pc + 2);
    assert_eq!(cycles, 2);
}

#[test]
fn unofficial_nop_3byte() {
    let (mut cpu, mut bus) = make();
    w(&mut bus, 0x0200, &[0x0C, 0x00, 0x04]); // TOP $0400
    let pc = cpu.pc;
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.pc, pc + 3);
    assert_eq!(cycles, 4);
}

#[test]
fn sbc_unofficial_eb() {
    // $EB is an unofficial duplicate of SBC immediate
    let (mut cpu, mut bus) = make();
    cpu.a = 0x50;
    cpu.p |= FLAG_C;
    w(&mut bus, 0x0200, &[0xEB, 0x10]);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x40);
}

// ---------------------------------------------------------------------------
// Cycle counts (spot checks)
// ---------------------------------------------------------------------------

#[test]
fn cycle_counts_load_store() {
    let cases: &[(&[u8], u8)] = &[
        (&[0xA9, 0x00], 2),       // LDA imm
        (&[0xA5, 0x00], 3),       // LDA zp
        (&[0xB5, 0x00], 4),       // LDA zp,X
        (&[0xAD, 0x00, 0x00], 4), // LDA abs
        (&[0x85, 0x00], 3),       // STA zp
        (&[0x8D, 0x00, 0x00], 4), // STA abs
    ];
    for &(program, expected) in cases {
        let (mut cpu, mut bus) = make();
        w(&mut bus, 0x0200, program);
        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, expected, "program {:02X?}", program);
    }
}

#[test]
fn cycle_counts_rmw() {
    let cases: &[(&[u8], u8)] = &[
        (&[0x06, 0x00], 5),       // ASL zp
        (&[0x16, 0x00], 6),       // ASL zp,X
        (&[0x0E, 0x00, 0x00], 6), // ASL abs
        (&[0x1E, 0x00, 0x00], 7), // ASL abs,X
        (&[0xE6, 0x00], 5),       // INC zp
        (&[0xC6, 0x00], 5),       // DEC zp
    ];
    for &(program, expected) in cases {
        let (mut cpu, mut bus) = make();
        w(&mut bus, 0x0200, program);
        let cycles = cpu.step(&mut bus);
        assert_eq!(cycles, expected, "program {:02X?}", program);
    }
}

// ---------------------------------------------------------------------------
// warm_reset
// ---------------------------------------------------------------------------

#[test]
fn warm_reset_leaves_axy_untouched_sets_i_decrements_sp_and_loads_vector() {
    let (mut cpu, mut bus) = make();
    // Reset vector $FFFC/$FFFD -> $1234.
    bus.mem[0xFFFC] = 0x34;
    bus.mem[0xFFFD] = 0x12;
    // Arbitrary RAM byte, to confirm nothing is pushed to the stack.
    bus.mem[0x0300] = 0xAB;

    cpu.a = 0x11;
    cpu.x = 0x22;
    cpu.y = 0x33;
    cpu.p = FLAG_U; // FLAG_I deliberately clear beforehand.
    cpu.sp = 0xF0;

    cpu.warm_reset(&mut bus);

    assert_eq!(cpu.a, 0x11);
    assert_eq!(cpu.x, 0x22);
    assert_eq!(cpu.y, 0x33);
    assert!(cpu.flag(FLAG_I));
    assert_eq!(cpu.sp, 0xED); // 0xF0 - 3
    assert_eq!(cpu.pc, 0x1234);
    assert_eq!(bus.mem[0x0300], 0xAB); // untouched — nothing was pushed
}

// ---------------------------------------------------------------------------
// KIL/JAM (unofficial halt opcodes)
// ---------------------------------------------------------------------------

/// All 12 KIL/JAM opcode values that permanently halt real 6502/2A03
/// hardware: 0x02 0x12 0x22 0x32 0x42 0x52 0x62 0x72 0x92 0xB2 0xD2 0xF2.
const KIL_OPCODES: [u8; 12] = [
    0x02, 0x12, 0x22, 0x32, 0x42, 0x52, 0x62, 0x72, 0x92, 0xB2, 0xD2, 0xF2,
];

/// Every KIL/JAM opcode consumes the opcode byte on dispatch (T1, ordinary
/// fetch semantics shared by all opcodes: `pc` advances past the opcode
/// itself, landing at opcode_address + 1 before this arm even runs) and
/// then, on T2, performs its own dummy operand-style fetch which advances
/// `pc` a *second* time, to opcode_address + 2. To genuinely freeze at the
/// opcode's own address (matching real 6502/2A03 hardware, where a JAMmed
/// CPU never advances PC past the JAM opcode at all), the arm must undo
/// BOTH advances — wind `pc` back by 2, not 1. This test verifies `pc`
/// returns exactly to its starting address (not start+1) after the full
/// 2-tick instruction completes, for all 12 opcode values.
#[test]
fn kil_opcodes_freeze_pc_within_instruction() {
    for &op in &KIL_OPCODES {
        let (mut cpu, mut bus) = make();
        bus.mem[0x0200] = op;
        let start_pc = cpu.pc; // 0x0200

        cpu.tick(&mut bus); // T1: dispatch fetches the opcode byte.
        let after_dispatch = cpu.pc;
        assert_eq!(
            after_dispatch,
            start_pc.wrapping_add(1),
            "opcode {op:#04x}: dispatch should advance pc past the opcode byte"
        );

        cpu.tick(&mut bus); // T2: KIL body's dummy fetch, then wound back by 2.
        assert_eq!(
            cpu.pc, start_pc,
            "opcode {op:#04x}: pc must return to the opcode's own address, not drift forward"
        );

        assert!(
            cpu.instruction_boundary(),
            "opcode {op:#04x}: instruction should be fully retired after 2 ticks"
        );
    }
}

/// Confirms the freeze is a genuine fixed point, not merely "stays inside a
/// run of identical jam bytes": running many more ticks keeps re-decoding
/// the SAME opcode at the SAME address forever. `pc` alternates between
/// `start_pc` (queue empty, between instructions) and `start_pc + 1`
/// (mid-instruction, right after dispatch's own fetch and before the KIL
/// body winds it back) — it never climbs past `start_pc + 1`, and always
/// returns to exactly `start_pc` at each instruction boundary, for as many
/// instructions as we care to run.
#[test]
fn kil_opcodes_stay_trapped_across_consecutive_instructions() {
    let (mut cpu, mut bus) = make();
    bus.mem[0x0200] = 0x02;
    let start_pc = cpu.pc; // 0x0200

    // 10 full KIL "instructions" worth of ticks (20 raw ticks: dispatch,
    // execute, dispatch, execute, ...). If pc drifted even by 1 per
    // instruction (the bug this test guards against), it would have moved
    // 10 bytes forward by now instead of returning to start_pc every time.
    for i in 0..20u32 {
        cpu.tick(&mut bus);
        let expected = if i % 2 == 0 {
            start_pc.wrapping_add(1) // just past dispatch's own fetch
        } else {
            start_pc // wound back by the KIL body at the end of each instruction
        };
        assert_eq!(cpu.pc, expected, "tick {i}: pc should be {expected:#06x}");
    }
    assert_eq!(cpu.pc, start_pc);
    assert!(cpu.instruction_boundary());
}

/// The scenario the pre-fix bug (`wrapping_sub(1)` instead of `(2)`) allowed
/// to slip through: a single KIL byte immediately followed by a DIFFERENT,
/// non-KIL opcode. With the old off-by-one, `pc` settled one byte forward
/// of the KIL opcode after each "instruction", so it would eventually land
/// on — and dispatch — that following byte as a real opcode, letting the
/// CPU fall through and resume normal execution. With the fix, `pc` must
/// never leave the KIL opcode's own address, so the following byte must
/// never be reached or executed.
#[test]
fn kil_never_falls_through_to_a_following_non_kil_opcode() {
    let (mut cpu, mut bus) = make();
    bus.mem[0x0200] = 0x02; // KIL
    bus.mem[0x0201] = 0xA9; // LDA #$42 — would prove an escape if ever reached.
    bus.mem[0x0202] = 0x42;
    let start_pc = cpu.pc; // 0x0200

    for i in 0..20u32 {
        cpu.tick(&mut bus);
        assert!(
            cpu.pc == start_pc || cpu.pc == start_pc.wrapping_add(1),
            "tick {i}: pc left the KIL opcode's address and reached {:#06x} — fell through to the following byte",
            cpu.pc
        );
    }
    assert_eq!(
        cpu.pc, start_pc,
        "pc must settle back exactly on the KIL opcode, not drift onto the LDA that follows it"
    );
    // If LDA #$42 had ever executed, cpu.a would be 0x42 — confirm it never ran.
    assert_ne!(
        cpu.a, 0x42,
        "LDA #$42 must never have executed — the CPU should still be jammed on the KIL opcode"
    );
}

// ---------------------------------------------------------------------------
// DMA stall cycle reporting (CpuBus::take_dma_stall_cycles)
// ---------------------------------------------------------------------------

struct StallBus {
    mem: [u8; 65536],
    stall_on_next_read: u32,
}

impl StallBus {
    fn new() -> Self {
        Self {
            mem: [0u8; 65536],
            stall_on_next_read: 0,
        }
    }
}

impl crate::cpu::Bus for StallBus {
    fn read(&mut self, addr: u16) -> u8 {
        self.mem[addr as usize]
    }
    fn write(&mut self, addr: u16, data: u8) {
        self.mem[addr as usize] = data;
    }
    fn take_dma_stall_cycles(&mut self) -> u32 {
        let n = self.stall_on_next_read;
        self.stall_on_next_read = 0;
        n
    }
}

#[test]
fn dma_stall_cycles_are_folded_into_cpu_cycles() {
    let mut cpu = Cpu::new();
    cpu.pc = 0x0200;
    let mut bus = StallBus::new();
    bus.mem[0x0200] = 0xA9; // LDA #$42 — normally 2 cycles
    bus.mem[0x0201] = 0x42;
    bus.stall_on_next_read = 4; // simulate a 4-cycle DMC DMA stall mid-instruction
    let cycles = cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x42);
    assert_eq!(cycles, 6); // 2 base + 4 stolen by the stall
}

#[test]
fn no_stall_leaves_cycle_count_unchanged() {
    let mut cpu = Cpu::new();
    cpu.pc = 0x0200;
    let mut bus = StallBus::new();
    bus.mem[0x0200] = 0xA9;
    bus.mem[0x0201] = 0x42;
    let cycles = cpu.step(&mut bus);
    assert_eq!(cycles, 2);
}

// ---------------------------------------------------------------------------
// SHA / SHS(TAS) / SHY / SHX — unstable high-byte stores
// (AccuracyCoin "behavior 1", the common NES CPU: value = reg & (H+1); on a
// page cross the write address's high byte BECOMES that value; a DMC DMA
// halting the dummy-read cycle right before the write drops the & (H+1).)
// ---------------------------------------------------------------------------

#[test]
fn shy_no_cross_stores_y_and_h_plus_1_at_effective_addr() {
    let (mut cpu, mut bus) = make();
    // SHY $15BB,X with X=0: no cross. Value = Y & ($15+1) = $FF & $16 = $16.
    w(&mut bus, 0x0200, &[0x9C, 0xBB, 0x15]);
    cpu.x = 0x00;
    cpu.y = 0xFF;
    let cycles = cpu.step(&mut bus);
    assert_eq!(bus.mem[0x15BB], 0x16);
    assert_eq!(cycles, 5);
}

#[test]
fn shy_page_cross_high_byte_becomes_value() {
    let (mut cpu, mut bus) = make();
    // SHY $1E90,X with X=$80 → crosses into $1F10. Value = Y & $1F = $05.
    // Corrupted address = (value << 8) | lo = $0510 — NOT $1F10, NOT $1E10.
    w(&mut bus, 0x0200, &[0x9C, 0x90, 0x1E]);
    cpu.x = 0x80;
    cpu.y = 0x05;
    cpu.step(&mut bus);
    assert_eq!(bus.mem[0x0510], 0x05, "write goes to (value<<8)|lo");
    assert_eq!(bus.mem[0x1F10], 0x00, "carried effective address untouched");
    assert_eq!(bus.mem[0x1E10], 0x00, "pre-carry address untouched");
}

#[test]
fn shx_page_cross_high_byte_becomes_value() {
    let (mut cpu, mut bus) = make();
    // SHX $1E90,Y with Y=$80 → crosses. Value = X & $1F = $05 → addr $0510.
    w(&mut bus, 0x0200, &[0x9E, 0x90, 0x1E]);
    cpu.y = 0x80;
    cpu.x = 0x05;
    cpu.step(&mut bus);
    assert_eq!(bus.mem[0x0510], 0x05);
    assert_eq!(bus.mem[0x1F10], 0x00);
    assert_eq!(bus.mem[0x1E10], 0x00);
}

#[test]
fn sha_abs_y_page_cross_high_byte_becomes_value() {
    let (mut cpu, mut bus) = make();
    // SHA $1E90,Y with Y=$80, A=$0D, X=$FF. Value = A & X & $1F = $0D.
    // Corrupted address = $0D10.
    w(&mut bus, 0x0200, &[0x9F, 0x90, 0x1E]);
    cpu.a = 0x0D;
    cpu.x = 0xFF;
    cpu.y = 0x80;
    cpu.step(&mut bus);
    assert_eq!(bus.mem[0x0D10], 0x0D);
    assert_eq!(bus.mem[0x1F10], 0x00);
}

#[test]
fn sha_ind_y_page_cross_high_byte_becomes_value() {
    let (mut cpu, mut bus) = make();
    // SHA ($60),Y — pointer at $60/$61 → base $1EF0; Y=$10 crosses to $1F00.
    // A=$55, X=$AA: value = $55 & $AA & $1F = $00 → write lands at $0000.
    w(&mut bus, 0x0200, &[0x93, 0x60]);
    bus.mem[0x0060] = 0xF0;
    bus.mem[0x0061] = 0x1E;
    bus.mem[0x0000] = 0xFF;
    cpu.a = 0x55;
    cpu.x = 0xAA;
    cpu.y = 0x10;
    let cycles = cpu.step(&mut bus);
    assert_eq!(
        bus.mem[0x0000], 0x00,
        "write goes to $0000 (value $00 as hi)"
    );
    assert_eq!(bus.mem[0x1F00], 0x00);
    assert_eq!(cycles, 6);
}

#[test]
fn tas_sets_sp_and_page_cross_high_byte_becomes_value() {
    let (mut cpu, mut bus) = make();
    // SHS $1E90,Y with Y=$80, A=$0D, X=$FF: SP = A & X = $0D;
    // value = SP & $1F = $0D → addr $0D10.
    w(&mut bus, 0x0200, &[0x9B, 0x90, 0x1E]);
    cpu.a = 0x0D;
    cpu.x = 0xFF;
    cpu.y = 0x80;
    cpu.step(&mut bus);
    assert_eq!(cpu.sp, 0x0D);
    assert_eq!(bus.mem[0x0D10], 0x0D);
    assert_eq!(bus.mem[0x1F10], 0x00);
}

#[test]
fn sh_stores_always_perform_precarry_dummy_read_before_write() {
    let (mut cpu, mut bus) = make();
    // SHY $15BB,X with X=0 — no cross, but hardware still performs the cycle-4
    // dummy read of the (pre-carry == effective) address before the write.
    w(&mut bus, 0x0200, &[0x9C, 0xBB, 0x15]);
    cpu.x = 0x00;
    cpu.y = 0xFF;
    bus.trace.clear();
    cpu.step(&mut bus);
    let expected: Vec<(u16, bool)> = vec![
        (0x0200, false), // opcode
        (0x0201, false), // operand lo
        (0x0202, false), // operand hi
        (0x15BB, false), // dummy read
        (0x15BB, true),  // write
    ];
    assert_eq!(bus.trace, expected);
}

/// Bus that pretends a DMC DMA halts the CPU when a chosen address is read
/// (bumping `dmc_dma_count` the way `Bus::maybe_dmc_dma` does), and/or that
/// a DMC request rises during a chosen read and stays pending (RDY low,
/// `dmc_dma_pending` true) afterwards.
struct DmaOnReadBus {
    mem: Box<[u8; 65536]>,
    dma_on_read_of: u16,
    pending_after_read_of: u16,
    dma_count: u64,
    pending: bool,
}

impl DmaOnReadBus {
    fn new(dma_on_read_of: u16) -> Self {
        Self {
            mem: Box::new([0u8; 65536]),
            dma_on_read_of,
            pending_after_read_of: 0xFFFF,
            dma_count: 0,
            pending: false,
        }
    }
}

impl crate::cpu::Bus for DmaOnReadBus {
    fn read(&mut self, addr: u16) -> u8 {
        if addr == self.dma_on_read_of {
            self.dma_count += 1;
        }
        if addr == self.pending_after_read_of {
            self.pending = true;
        }
        self.mem[addr as usize]
    }
    fn write(&mut self, addr: u16, data: u8) {
        self.mem[addr as usize] = data;
    }
    fn dmc_dma_count(&self) -> u64 {
        self.dma_count
    }
    fn dmc_dma_pending(&self) -> bool {
        self.pending
    }
}

#[test]
fn shy_with_dma_on_dummy_read_becomes_sty() {
    // AccuracyCoin: "SHY just becomes STY if a DMA occurs on the right cpu
    // cycle" — a DMC DMA halting the dummy-read cycle immediately before the
    // write drops the & (H+1); the raw register is stored.
    let mut cpu = Cpu::new();
    cpu.pc = 0x0200;
    let mut bus = DmaOnReadBus::new(0x0500); // halt lands on the dummy read
    bus.mem[0x0200] = 0x9C; // SHY $0500,X
    bus.mem[0x0201] = 0x00;
    bus.mem[0x0202] = 0x05;
    cpu.x = 0x00;
    cpu.y = 0xA5;
    cpu.step(&mut bus);
    assert_eq!(
        bus.mem[0x0500], 0xA5,
        "H isn't part of the equation anymore"
    );
}

#[test]
fn shy_without_dma_masks_with_h_plus_1() {
    let mut cpu = Cpu::new();
    cpu.pc = 0x0200;
    let mut bus = DmaOnReadBus::new(0xFFFF); // DMA never fires
    bus.mem[0x0200] = 0x9C; // SHY $0500,X
    bus.mem[0x0201] = 0x00;
    bus.mem[0x0202] = 0x05;
    cpu.x = 0x00;
    cpu.y = 0xA5;
    cpu.step(&mut bus);
    assert_eq!(bus.mem[0x0500], 0xA5 & 0x06, "value = Y & (H+1)");
}

#[test]
fn shy_with_dma_request_rising_during_dummy_read_also_becomes_sty() {
    // The request can rise DURING the dummy read itself: the CPU won't halt
    // until its next read (after the write — writes never halt), but RDY is
    // already low at the write, which is what corrupts the value on
    // hardware. Detected via dmc_dma_pending after the dummy read.
    let mut cpu = Cpu::new();
    cpu.pc = 0x0200;
    let mut bus = DmaOnReadBus::new(0xFFFF); // no halt is ever serviced
    bus.pending_after_read_of = 0x0500; // request rises during the dummy read
    bus.mem[0x0200] = 0x9C; // SHY $0500,X
    bus.mem[0x0201] = 0x00;
    bus.mem[0x0202] = 0x05;
    cpu.x = 0x00;
    cpu.y = 0xA5;
    cpu.step(&mut bus);
    assert_eq!(bus.mem[0x0500], 0xA5);
}

#[test]
fn shy_with_dma_on_operand_hi_fetch_keeps_mask() {
    // A halt serviced ON the operand-high fetch means the request rose even
    // earlier — the DMA completes and RDY is high again by the dummy read,
    // so the value is still masked with (H+1).
    let mut cpu = Cpu::new();
    cpu.pc = 0x0200;
    let mut bus = DmaOnReadBus::new(0x0202); // halt lands on operand-hi fetch
    bus.mem[0x0200] = 0x9C; // SHY $0500,X
    bus.mem[0x0201] = 0x00;
    bus.mem[0x0202] = 0x05;
    cpu.x = 0x00;
    cpu.y = 0xA5;
    cpu.step(&mut bus);
    assert_eq!(bus.mem[0x0500], 0xA5 & 0x06);
}

// ---------------------------------------------------------------------------
// Unofficial NOPs perform their target read (AccuracyCoin "All NOPs": NOP
// $3AEA — a $2002 mirror — must clear the PPU VBlank flag, so the read is a
// real, side-effecting bus access; also keeps 1 bus access per CPU cycle for
// the realtime DMC clock)
// ---------------------------------------------------------------------------

#[test]
fn nop_absolute_0c_reads_target_address() {
    let (mut cpu, mut bus) = make();
    w(&mut bus, 0x0200, &[0x0C, 0xEA, 0x3A]); // NOP $3AEA
    bus.trace.clear();
    let cycles = cpu.step(&mut bus);
    let expected: Vec<(u16, bool)> = vec![
        (0x0200, false), // opcode
        (0x0201, false), // operand lo
        (0x0202, false), // operand hi
        (0x3AEA, false), // target read (observable — e.g. a $2002 mirror)
    ];
    assert_eq!(bus.trace, expected);
    assert_eq!(cycles, 4);
}

#[test]
fn nop_zero_page_reads_target_address() {
    let (mut cpu, mut bus) = make();
    w(&mut bus, 0x0200, &[0x04, 0xCA]); // NOP $CA
    bus.trace.clear();
    let cycles = cpu.step(&mut bus);
    let expected: Vec<(u16, bool)> = vec![(0x0200, false), (0x0201, false), (0x00CA, false)];
    assert_eq!(bus.trace, expected);
    assert_eq!(cycles, 3);
}

#[test]
fn nop_zero_page_x_reads_effective_address() {
    let (mut cpu, mut bus) = make();
    w(&mut bus, 0x0200, &[0x14, 0xC0]); // NOP $C0,X
    cpu.x = 0x0A;
    bus.trace.clear();
    let cycles = cpu.step(&mut bus);
    assert_eq!(bus.trace.last(), Some(&(0x00CA, false)));
    assert_eq!(cycles, 4);
}
