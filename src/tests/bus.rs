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
fn dmc_dma_halt_read_updates_open_bus_and_clears_vblank() {
    let mut bus = Bus::new();
    // Force the PPU into a state where $2002 currently reads back with
    // VBlank set, so the halt-cycle dummy read's side effect (clearing it)
    // is directly observable.
    bus.ppu.force_vblank_for_test();
    assert_eq!(bus.read(0x2002) & 0x80, 0x80);
    bus.ppu.force_vblank_for_test();

    // Prime the DMC to need a fetch on its very next real-time clock: rate 0
    // (fastest), enabled with a 1-byte sample.
    bus.write(0x4010, 0x00);
    bus.write(0x4012, 0x00); // sample address $C000
    bus.write(0x4013, 0x00); // length 1
    bus.write(0x4015, 0x10); // enable DMC

    // Drive enough real reads (each ticking the DMC's real-time clock) for
    // the timer to underflow and a DMA to trigger. Reading $2002 repeatedly
    // both advances the clock and is the register under test for the halt
    // cycles' side effects.
    let mut saw_cleared_vblank_mid_stream = false;
    for _ in 0..600 {
        let v = bus.read(0x2002);
        if v & 0x80 == 0 {
            saw_cleared_vblank_mid_stream = true;
            break;
        }
    }
    assert!(
        saw_cleared_vblank_mid_stream,
        "a DMC DMA halt-cycle dummy read of $2002 should clear VBlank just \
         like any other $2002 read"
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
