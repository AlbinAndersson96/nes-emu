use crate::bus::Bus;
use crate::cpu::Bus as CpuBus;

#[test]
fn controller_1_shifts_out_bits_lsb_first_then_returns_open_bus_ones() {
    let mut bus = Bus::new();
    // A=bit0, B=bit1, Select=bit2, Start=bit3, Up=bit4, Down=bit5, Left=bit6, Right=bit7
    bus.set_controller_state(0, 0b1011_0100);
    bus.write(0x4016, 1); // strobe high: continuously reload
    bus.write(0x4016, 0); // strobe low: freeze, start serial read

    let expected_bits = [0u8, 0, 1, 0, 1, 1, 0, 1];
    for expected in expected_bits {
        assert_eq!(bus.read(0x4016), expected);
    }
    // Past the 8th read, real hardware returns 1 (open bus / pull-up).
    for _ in 0..4 {
        assert_eq!(bus.read(0x4016), 1);
    }
}

#[test]
fn controller_read_while_strobed_returns_latch_bit0_continuously() {
    let mut bus = Bus::new();
    bus.set_controller_state(0, 0b1011_0100); // bit0 (A) = 0
    bus.write(0x4016, 1); // strobe high: continuously reload, never shifts

    // Repeated reads while still strobed must all return the same bit,
    // never advancing through the latched pattern.
    for _ in 0..5 {
        assert_eq!(bus.read(0x4016) & 1, 0);
    }

    // Hardware continuously reloads from the live button state while
    // strobed, so a change mid-strobe must be reflected on the very next
    // read (no serial shifting has consumed anything yet).
    bus.set_controller_state(0, 0b0000_0001); // bit0 (A) = 1
    assert_eq!(bus.read(0x4016) & 1, 1);
    assert_eq!(bus.read(0x4016) & 1, 1);

    // Releasing strobe freezes the register at the current latch and
    // starts serial shifting from there.
    bus.write(0x4016, 0);
    assert_eq!(bus.read(0x4016) & 1, 1); // A
    assert_eq!(bus.read(0x4016) & 1, 0); // B
}

#[test]
fn controller_2_reads_from_4017_independently_of_controller_1() {
    let mut bus = Bus::new();
    bus.set_controller_state(0, 0xFF);
    bus.set_controller_state(1, 0x00);
    bus.write(0x4016, 1);
    bus.write(0x4016, 0);

    assert_eq!(bus.read(0x4017), 0);
    assert_eq!(bus.read(0x4016), 1);
}

#[test]
fn dmc_dma_halt_reads_hit_the_cpu_target_address_with_real_side_effects() {
    let mut bus = Bus::new();
    bus.write(0x4010, 0x00);
    bus.write(0x4012, 0x00);
    bus.write(0x4013, 0x00);
    bus.write(0x4015, 0x10); // enable DMC — arms needs_dma() once the timer fires

    // Prime controller 1 with a known 8-bit pattern; each $4016 read shifts
    // the register by exactly one bit position.
    bus.set_controller_state(0, 0b1011_0100);
    bus.write(0x4016, 1);
    bus.write(0x4016, 0);

    // Drive real $4016 reads (each also ticking the DMC's real-time clock)
    // until one of them happens to be the read the DMC's timer underflows
    // on. If a DMC DMA halt/fetch sequence lands on THIS read, its 2-3 dummy
    // re-reads target the CPU's own in-flight address — $4016 itself — so
    // the shift register advances by MORE than the single position an
    // ordinary read would produce. A single before/after comparison against
    // the deterministic single-shift formula proves extra real reads of
    // $4016 happened inside that one logical CPU read.
    let mut found_extra_shift = false;
    for _ in 0..600 {
        let before = bus.peek_controller_shift(0);
        let _ = bus.read(0x4016);
        let after = bus.peek_controller_shift(0);
        let expected_single_shift = (before >> 1) | 0x80;
        if after != expected_single_shift {
            found_extra_shift = true;
            break;
        }
    }
    assert!(
        found_extra_shift,
        "a DMC DMA halt/fetch sequence landing on a $4016 read should \
         perform its own real, side-effecting re-reads of $4016 (the CPU's \
         in-flight target address), shifting the register by more than the \
         single position an ordinary read would"
    );
}

#[test]
fn dmc_dma_stall_reports_three_or_four_extra_cycles() {
    use crate::cpu::Bus as CpuBusTrait;
    let mut bus = Bus::new();
    bus.write(0x4010, 0x00);
    bus.write(0x4012, 0x00);
    bus.write(0x4013, 0x00);
    bus.write(0x4015, 0x10);

    let mut total_stall = 0u32;
    for _ in 0..600 {
        let _ = bus.read(0x00FF); // arbitrary RAM address, no register side effects
        total_stall += bus.take_dma_stall_cycles();
        if total_stall > 0 {
            break;
        }
    }
    assert!(
        total_stall == 3 || total_stall == 4,
        "expected a 3- or 4-cycle DMC DMA stall, got {total_stall}"
    );
}

#[test]
fn reading_4015_does_not_update_cpu_open_bus_latch() {
    // $4015 is internal to the 2A03 — the status byte never drives the
    // external data bus (AccuracyCoin "Open Bus" test 7: LDA $40FF,X with
    // X=$16 dummy-reads $4015 on the page cross, and the following read of
    // unmapped $4115 must still return $40, the operand high byte).
    let mut bus = Bus::new();
    let _ = bus.read(0x0040); // RAM read puts $00 on the bus…
    bus.write(0x0040, 0x40);
    let _ = bus.read(0x0040); // …now the latch holds $40
    let _ = bus.read(0x4015); // status read must NOT disturb it
    assert_eq!(
        bus.read(0x5000),
        0x40,
        "unmapped read returns the pre-$4015 open-bus value"
    );
}

#[test]
fn writing_4015_does_update_cpu_open_bus_latch() {
    // Writes always drive the bus, even to $4015 (Open Bus test 8).
    let mut bus = Bus::new();
    bus.write(0x4015, 0x60);
    assert_eq!(bus.read(0x5000), 0x60);
}
