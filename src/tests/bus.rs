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
