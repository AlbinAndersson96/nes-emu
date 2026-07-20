use super::{Bus, Cpu, FLAG_B, FLAG_C, FLAG_I, FLAG_N, FLAG_U, FLAG_V, FLAG_Z};

/// Executes one instruction. Returns the number of cycles consumed.
pub fn execute(cpu: &mut Cpu, bus: &mut dyn Bus, opcode: u8) -> u8 {
    // Every 6502 instruction reads the byte after its opcode on T2.
    // Multi-byte instructions consume it as their operand (each handler
    // fetches it), but one-byte instructions read and DISCARD it — the read
    // still happens on the bus, and its side effects are observable when
    // executing out of I/O space: blargg's cpu_exec_space_ppuio runs an RTS
    // at $2001 and requires its dummy fetch of $2002 to clear the PPU
    // address latch. PC is not advanced.
    match opcode {
        0x0A | 0x2A | 0x4A | 0x6A               // ASL/ROL/LSR/ROR A
        | 0x08 | 0x28 | 0x48 | 0x68             // PHP/PLP/PHA/PLA
        | 0x18 | 0x38 | 0x58 | 0x78             // CLC/SEC/CLI/SEI
        | 0xB8 | 0xD8 | 0xF8                    // CLV/CLD/SED
        | 0xAA | 0x8A | 0xA8 | 0x98 | 0xBA | 0x9A // TAX/TXA/TAY/TYA/TSX/TXS
        | 0xCA | 0x88 | 0xE8 | 0xC8             // DEX/DEY/INX/INY
        | 0xEA | 0x1A | 0x3A | 0x5A | 0x7A | 0xDA | 0xFA // NOP + 1-byte unofficial NOPs
        | 0x40 | 0x60 => {                      // RTI/RTS
            let _ = bus.read(cpu.pc);
        }
        _ => {}
    }

    match opcode {
        // --- ADC ---
        0x69 => {
            let v = cpu.fetch(bus);
            adc(cpu, v);
            2
        }
        0x65 => {
            let (a, _) = cpu.addr_zero_page(bus);
            adc_m(cpu, bus, a);
            3
        }
        0x75 => {
            let (a, _) = cpu.addr_zero_page_x(bus);
            adc_m(cpu, bus, a);
            4
        }
        0x6D => {
            let (a, _) = cpu.addr_absolute(bus);
            adc_m(cpu, bus, a);
            4
        }
        0x7D => {
            let (a, p) = cpu.addr_absolute_x(bus);
            adc_m(cpu, bus, a);
            4 + p as u8
        }
        0x79 => {
            let (a, p) = cpu.addr_absolute_y(bus);
            adc_m(cpu, bus, a);
            4 + p as u8
        }
        0x61 => {
            let (a, _) = cpu.addr_indirect_x(bus);
            adc_m(cpu, bus, a);
            6
        }
        0x71 => {
            let (a, p) = cpu.addr_indirect_y(bus);
            adc_m(cpu, bus, a);
            5 + p as u8
        }

        // --- AND ---
        0x29 => {
            let v = cpu.fetch(bus);
            and(cpu, v);
            2
        }
        0x25 => {
            let (a, _) = cpu.addr_zero_page(bus);
            and_m(cpu, bus, a);
            3
        }
        0x35 => {
            let (a, _) = cpu.addr_zero_page_x(bus);
            and_m(cpu, bus, a);
            4
        }
        0x2D => {
            let (a, _) = cpu.addr_absolute(bus);
            and_m(cpu, bus, a);
            4
        }
        0x3D => {
            let (a, p) = cpu.addr_absolute_x(bus);
            and_m(cpu, bus, a);
            4 + p as u8
        }
        0x39 => {
            let (a, p) = cpu.addr_absolute_y(bus);
            and_m(cpu, bus, a);
            4 + p as u8
        }
        0x21 => {
            let (a, _) = cpu.addr_indirect_x(bus);
            and_m(cpu, bus, a);
            6
        }
        0x31 => {
            let (a, p) = cpu.addr_indirect_y(bus);
            and_m(cpu, bus, a);
            5 + p as u8
        }

        // --- ASL ---
        0x0A => {
            cpu.a = asl(cpu, cpu.a);
            2
        }
        0x06 => {
            let (a, _) = cpu.addr_zero_page(bus);
            rmw(cpu, bus, a, asl);
            5
        }
        0x16 => {
            let (a, _) = cpu.addr_zero_page_x(bus);
            rmw(cpu, bus, a, asl);
            6
        }
        0x0E => {
            let (a, _) = cpu.addr_absolute(bus);
            rmw(cpu, bus, a, asl);
            6
        }
        0x1E => {
            let a = cpu.addr_absolute_x_rmw(bus);
            rmw(cpu, bus, a, asl);
            7
        }

        // --- Branches ---
        0x90 => branch(cpu, bus, !cpu.flag(FLAG_C)), // BCC
        0xB0 => branch(cpu, bus, cpu.flag(FLAG_C)),  // BCS
        0xF0 => branch(cpu, bus, cpu.flag(FLAG_Z)),  // BEQ
        0x30 => branch(cpu, bus, cpu.flag(FLAG_N)),  // BMI
        0xD0 => branch(cpu, bus, !cpu.flag(FLAG_Z)), // BNE
        0x10 => branch(cpu, bus, !cpu.flag(FLAG_N)), // BPL
        0x50 => branch(cpu, bus, !cpu.flag(FLAG_V)), // BVC
        0x70 => branch(cpu, bus, cpu.flag(FLAG_V)),  // BVS

        // --- BIT ---
        0x24 => {
            let (a, _) = cpu.addr_zero_page(bus);
            bit(cpu, bus, a);
            3
        }
        0x2C => {
            let (a, _) = cpu.addr_absolute(bus);
            bit(cpu, bus, a);
            4
        }

        // --- BRK ---
        0x00 => {
            // T2: fetch padding byte (discarded but PC advances to PC+2).
            let _ = cpu.fetch(bus);
            let p = cpu.p | FLAG_B | FLAG_U;
            // T3–T7 come from micro-ops; VectorFetch may redirect to NMI vector.
            cpu.queue_interrupt_sequence(0xFFFE, p);
            2 // T1+T2
        }

        // --- KIL/JAM (unofficial) --- permanently halts the CPU (jams) on
        // real 6502/2A03 hardware. All 12 opcodes: 0x02 0x12 0x22 0x32 0x42
        // 0x52 0x62 0x72 0x92 0xB2 0xD2 0xF2. PC never advances past this
        // opcode once hit on real hardware (only a physical reset
        // recovers) — modeled here by re-fetching the same byte forever
        // without moving PC: the dispatcher's own opcode fetch already
        // advanced PC by 1 before this arm runs, and the dummy fetch below
        // advances it again, so PC must be wound back by 2 (not 1) to land
        // exactly back on this opcode's own address.
        0x02 | 0x12 | 0x22 | 0x32 | 0x42 | 0x52 | 0x62 | 0x72 | 0x92 | 0xB2 | 0xD2 | 0xF2 => {
            let _ = cpu.fetch(bus); // dummy fetch, not consumed
            cpu.pc = cpu.pc.wrapping_sub(2); // return PC to this opcode's own address: re-decodes forever
            2
        }

        // --- Flag clears / sets ---
        0x18 => {
            cpu.set_flag(FLAG_C, false);
            2
        } // CLC
        0xD8 => {
            cpu.set_flag(super::FLAG_D, false);
            2
        } // CLD
        0x58 => {
            // CLI
            let was_set = cpu.flag(FLAG_I);
            cpu.set_flag(FLAG_I, false);
            if was_set {
                cpu.irq_inhibit_next = true;
            }
            2
        }
        0xB8 => {
            cpu.set_flag(FLAG_V, false);
            2
        } // CLV
        0x38 => {
            cpu.set_flag(FLAG_C, true);
            2
        } // SEC
        0xF8 => {
            cpu.set_flag(super::FLAG_D, true);
            2
        } // SED
        0x78 => {
            cpu.set_flag(FLAG_I, true);
            2
        } // SEI

        // --- CMP ---
        0xC9 => {
            let v = cpu.fetch(bus);
            cmp(cpu, cpu.a, v);
            2
        }
        0xC5 => {
            let (a, _) = cpu.addr_zero_page(bus);
            cmp_m(cpu, bus, cpu.a, a);
            3
        }
        0xD5 => {
            let (a, _) = cpu.addr_zero_page_x(bus);
            cmp_m(cpu, bus, cpu.a, a);
            4
        }
        0xCD => {
            let (a, _) = cpu.addr_absolute(bus);
            cmp_m(cpu, bus, cpu.a, a);
            4
        }
        0xDD => {
            let (a, p) = cpu.addr_absolute_x(bus);
            cmp_m(cpu, bus, cpu.a, a);
            4 + p as u8
        }
        0xD9 => {
            let (a, p) = cpu.addr_absolute_y(bus);
            cmp_m(cpu, bus, cpu.a, a);
            4 + p as u8
        }
        0xC1 => {
            let (a, _) = cpu.addr_indirect_x(bus);
            cmp_m(cpu, bus, cpu.a, a);
            6
        }
        0xD1 => {
            let (a, p) = cpu.addr_indirect_y(bus);
            cmp_m(cpu, bus, cpu.a, a);
            5 + p as u8
        }

        // --- CPX ---
        0xE0 => {
            let v = cpu.fetch(bus);
            cmp(cpu, cpu.x, v);
            2
        }
        0xE4 => {
            let (a, _) = cpu.addr_zero_page(bus);
            cmp_m(cpu, bus, cpu.x, a);
            3
        }
        0xEC => {
            let (a, _) = cpu.addr_absolute(bus);
            cmp_m(cpu, bus, cpu.x, a);
            4
        }

        // --- CPY ---
        0xC0 => {
            let v = cpu.fetch(bus);
            cmp(cpu, cpu.y, v);
            2
        }
        0xC4 => {
            let (a, _) = cpu.addr_zero_page(bus);
            cmp_m(cpu, bus, cpu.y, a);
            3
        }
        0xCC => {
            let (a, _) = cpu.addr_absolute(bus);
            cmp_m(cpu, bus, cpu.y, a);
            4
        }

        // --- DEC ---
        0xC6 => {
            let (a, _) = cpu.addr_zero_page(bus);
            rmw(cpu, bus, a, dec);
            5
        }
        0xD6 => {
            let (a, _) = cpu.addr_zero_page_x(bus);
            rmw(cpu, bus, a, dec);
            6
        }
        0xCE => {
            let (a, _) = cpu.addr_absolute(bus);
            rmw(cpu, bus, a, dec);
            6
        }
        0xDE => {
            let a = cpu.addr_absolute_x_rmw(bus);
            rmw(cpu, bus, a, dec);
            7
        }

        // --- DEX / DEY ---
        0xCA => {
            cpu.x = cpu.x.wrapping_sub(1);
            cpu.set_nz(cpu.x);
            2
        }
        0x88 => {
            cpu.y = cpu.y.wrapping_sub(1);
            cpu.set_nz(cpu.y);
            2
        }

        // --- EOR ---
        0x49 => {
            let v = cpu.fetch(bus);
            eor(cpu, v);
            2
        }
        0x45 => {
            let (a, _) = cpu.addr_zero_page(bus);
            eor_m(cpu, bus, a);
            3
        }
        0x55 => {
            let (a, _) = cpu.addr_zero_page_x(bus);
            eor_m(cpu, bus, a);
            4
        }
        0x4D => {
            let (a, _) = cpu.addr_absolute(bus);
            eor_m(cpu, bus, a);
            4
        }
        0x5D => {
            let (a, p) = cpu.addr_absolute_x(bus);
            eor_m(cpu, bus, a);
            4 + p as u8
        }
        0x59 => {
            let (a, p) = cpu.addr_absolute_y(bus);
            eor_m(cpu, bus, a);
            4 + p as u8
        }
        0x41 => {
            let (a, _) = cpu.addr_indirect_x(bus);
            eor_m(cpu, bus, a);
            6
        }
        0x51 => {
            let (a, p) = cpu.addr_indirect_y(bus);
            eor_m(cpu, bus, a);
            5 + p as u8
        }

        // --- INC ---
        0xE6 => {
            let (a, _) = cpu.addr_zero_page(bus);
            rmw(cpu, bus, a, inc);
            5
        }
        0xF6 => {
            let (a, _) = cpu.addr_zero_page_x(bus);
            rmw(cpu, bus, a, inc);
            6
        }
        0xEE => {
            let (a, _) = cpu.addr_absolute(bus);
            rmw(cpu, bus, a, inc);
            6
        }
        0xFE => {
            let a = cpu.addr_absolute_x_rmw(bus);
            rmw(cpu, bus, a, inc);
            7
        }

        // --- INX / INY ---
        0xE8 => {
            cpu.x = cpu.x.wrapping_add(1);
            cpu.set_nz(cpu.x);
            2
        }
        0xC8 => {
            cpu.y = cpu.y.wrapping_add(1);
            cpu.set_nz(cpu.y);
            2
        }

        // --- JMP ---
        0x4C => {
            cpu.pc = cpu.fetch_u16(bus);
            3
        }
        0x6C => {
            let ptr = cpu.fetch_u16(bus);
            cpu.pc = cpu.read_u16_bugged(bus, ptr);
            5
        }

        // --- JSR ---
        0x20 => {
            jsr(cpu, bus);
            6
        }

        // --- LDA ---
        0xA9 => {
            let v = cpu.fetch(bus);
            lda(cpu, v);
            2
        }
        0xA5 => {
            let (a, _) = cpu.addr_zero_page(bus);
            lda_m(cpu, bus, a);
            3
        }
        0xB5 => {
            let (a, _) = cpu.addr_zero_page_x(bus);
            lda_m(cpu, bus, a);
            4
        }
        0xAD => {
            let (a, _) = cpu.addr_absolute(bus);
            lda_m(cpu, bus, a);
            4
        }
        0xBD => {
            let (a, p) = cpu.addr_absolute_x(bus);
            lda_m(cpu, bus, a);
            4 + p as u8
        }
        0xB9 => {
            let (a, p) = cpu.addr_absolute_y(bus);
            lda_m(cpu, bus, a);
            4 + p as u8
        }
        0xA1 => {
            let (a, _) = cpu.addr_indirect_x(bus);
            lda_m(cpu, bus, a);
            6
        }
        0xB1 => {
            let (a, p) = cpu.addr_indirect_y(bus);
            lda_m(cpu, bus, a);
            5 + p as u8
        }

        // --- LDX ---
        0xA2 => {
            let v = cpu.fetch(bus);
            ldx(cpu, v);
            2
        }
        0xA6 => {
            let (a, _) = cpu.addr_zero_page(bus);
            ldx_m(cpu, bus, a);
            3
        }
        0xB6 => {
            let (a, _) = cpu.addr_zero_page_y(bus);
            ldx_m(cpu, bus, a);
            4
        }
        0xAE => {
            let (a, _) = cpu.addr_absolute(bus);
            ldx_m(cpu, bus, a);
            4
        }
        0xBE => {
            let (a, p) = cpu.addr_absolute_y(bus);
            ldx_m(cpu, bus, a);
            4 + p as u8
        }

        // --- LDY ---
        0xA0 => {
            let v = cpu.fetch(bus);
            ldy(cpu, v);
            2
        }
        0xA4 => {
            let (a, _) = cpu.addr_zero_page(bus);
            ldy_m(cpu, bus, a);
            3
        }
        0xB4 => {
            let (a, _) = cpu.addr_zero_page_x(bus);
            ldy_m(cpu, bus, a);
            4
        }
        0xAC => {
            let (a, _) = cpu.addr_absolute(bus);
            ldy_m(cpu, bus, a);
            4
        }
        0xBC => {
            let (a, p) = cpu.addr_absolute_x(bus);
            ldy_m(cpu, bus, a);
            4 + p as u8
        }

        // --- LSR ---
        0x4A => {
            cpu.a = lsr(cpu, cpu.a);
            2
        }
        0x46 => {
            let (a, _) = cpu.addr_zero_page(bus);
            rmw(cpu, bus, a, lsr);
            5
        }
        0x56 => {
            let (a, _) = cpu.addr_zero_page_x(bus);
            rmw(cpu, bus, a, lsr);
            6
        }
        0x4E => {
            let (a, _) = cpu.addr_absolute(bus);
            rmw(cpu, bus, a, lsr);
            6
        }
        0x5E => {
            let a = cpu.addr_absolute_x_rmw(bus);
            rmw(cpu, bus, a, lsr);
            7
        }

        // --- NOP ---
        0xEA => 2,

        // --- ORA ---
        0x09 => {
            let v = cpu.fetch(bus);
            ora(cpu, v);
            2
        }
        0x05 => {
            let (a, _) = cpu.addr_zero_page(bus);
            ora_m(cpu, bus, a);
            3
        }
        0x15 => {
            let (a, _) = cpu.addr_zero_page_x(bus);
            ora_m(cpu, bus, a);
            4
        }
        0x0D => {
            let (a, _) = cpu.addr_absolute(bus);
            ora_m(cpu, bus, a);
            4
        }
        0x1D => {
            let (a, p) = cpu.addr_absolute_x(bus);
            ora_m(cpu, bus, a);
            4 + p as u8
        }
        0x19 => {
            let (a, p) = cpu.addr_absolute_y(bus);
            ora_m(cpu, bus, a);
            4 + p as u8
        }
        0x01 => {
            let (a, _) = cpu.addr_indirect_x(bus);
            ora_m(cpu, bus, a);
            6
        }
        0x11 => {
            let (a, p) = cpu.addr_indirect_y(bus);
            ora_m(cpu, bus, a);
            5 + p as u8
        }

        // --- Stack ---
        0x48 => {
            let a = cpu.a;
            cpu.push(bus, a);
            3
        } // PHA
        0x08 => {
            let p = cpu.p | FLAG_B | FLAG_U;
            cpu.push(bus, p);
            3
        } // PHP
        0x68 => {
            // T3: internal operation (increment S) — dummy read before the
            // real pop (see rts()/rti() for the same hardware pattern).
            let _ = bus.read(0x0100 | cpu.sp as u16);
            let v = cpu.pop(bus);
            cpu.a = v;
            cpu.set_nz(v);
            4
        } // PLA
        0x28 => {
            // PLP
            let old_i = cpu.flag(FLAG_I);
            let _ = bus.read(0x0100 | cpu.sp as u16);
            let v = cpu.pop(bus);
            cpu.p = (v & !FLAG_B) | FLAG_U;
            if old_i && !cpu.flag(FLAG_I) {
                cpu.irq_inhibit_next = true;
            }
            4
        }

        // --- ROL ---
        0x2A => {
            cpu.a = rol(cpu, cpu.a);
            2
        }
        0x26 => {
            let (a, _) = cpu.addr_zero_page(bus);
            rmw(cpu, bus, a, rol);
            5
        }
        0x36 => {
            let (a, _) = cpu.addr_zero_page_x(bus);
            rmw(cpu, bus, a, rol);
            6
        }
        0x2E => {
            let (a, _) = cpu.addr_absolute(bus);
            rmw(cpu, bus, a, rol);
            6
        }
        0x3E => {
            let a = cpu.addr_absolute_x_rmw(bus);
            rmw(cpu, bus, a, rol);
            7
        }

        // --- ROR ---
        0x6A => {
            cpu.a = ror(cpu, cpu.a);
            2
        }
        0x66 => {
            let (a, _) = cpu.addr_zero_page(bus);
            rmw(cpu, bus, a, ror);
            5
        }
        0x76 => {
            let (a, _) = cpu.addr_zero_page_x(bus);
            rmw(cpu, bus, a, ror);
            6
        }
        0x6E => {
            let (a, _) = cpu.addr_absolute(bus);
            rmw(cpu, bus, a, ror);
            6
        }
        0x7E => {
            let a = cpu.addr_absolute_x_rmw(bus);
            rmw(cpu, bus, a, ror);
            7
        }

        // --- RTI / RTS ---
        0x40 => {
            rti(cpu, bus);
            6
        }
        0x60 => {
            rts(cpu, bus);
            6
        }

        // --- SBC ---
        0xE9 => {
            let v = cpu.fetch(bus);
            sbc(cpu, v);
            2
        }
        0xE5 => {
            let (a, _) = cpu.addr_zero_page(bus);
            sbc_m(cpu, bus, a);
            3
        }
        0xF5 => {
            let (a, _) = cpu.addr_zero_page_x(bus);
            sbc_m(cpu, bus, a);
            4
        }
        0xED => {
            let (a, _) = cpu.addr_absolute(bus);
            sbc_m(cpu, bus, a);
            4
        }
        0xFD => {
            let (a, p) = cpu.addr_absolute_x(bus);
            sbc_m(cpu, bus, a);
            4 + p as u8
        }
        0xF9 => {
            let (a, p) = cpu.addr_absolute_y(bus);
            sbc_m(cpu, bus, a);
            4 + p as u8
        }
        0xE1 => {
            let (a, _) = cpu.addr_indirect_x(bus);
            sbc_m(cpu, bus, a);
            6
        }
        0xF1 => {
            let (a, p) = cpu.addr_indirect_y(bus);
            sbc_m(cpu, bus, a);
            5 + p as u8
        }

        // --- STA ---
        0x85 => {
            let (a, _) = cpu.addr_zero_page(bus);
            let v = cpu.a;
            cpu.write(bus, a, v);
            3
        }
        0x95 => {
            let (a, _) = cpu.addr_zero_page_x(bus);
            let v = cpu.a;
            cpu.write(bus, a, v);
            4
        }
        0x8D => {
            let (a, _) = cpu.addr_absolute(bus);
            let v = cpu.a;
            cpu.write(bus, a, v);
            4
        }
        0x9D => {
            let a = cpu.addr_absolute_x_store(bus);
            let v = cpu.a;
            cpu.write(bus, a, v);
            5
        }
        0x99 => {
            let a = cpu.addr_absolute_y_store(bus);
            let v = cpu.a;
            cpu.write(bus, a, v);
            5
        }
        0x81 => {
            let (a, _) = cpu.addr_indirect_x(bus);
            let v = cpu.a;
            cpu.write(bus, a, v);
            6
        }
        0x91 => {
            let a = cpu.addr_indirect_y_store(bus);
            let v = cpu.a;
            cpu.write(bus, a, v);
            6
        }

        // --- STX ---
        0x86 => {
            let (a, _) = cpu.addr_zero_page(bus);
            let v = cpu.x;
            cpu.write(bus, a, v);
            3
        }
        0x96 => {
            let (a, _) = cpu.addr_zero_page_y(bus);
            let v = cpu.x;
            cpu.write(bus, a, v);
            4
        }
        0x8E => {
            let (a, _) = cpu.addr_absolute(bus);
            let v = cpu.x;
            cpu.write(bus, a, v);
            4
        }

        // --- STY ---
        0x84 => {
            let (a, _) = cpu.addr_zero_page(bus);
            let v = cpu.y;
            cpu.write(bus, a, v);
            3
        }
        0x94 => {
            let (a, _) = cpu.addr_zero_page_x(bus);
            let v = cpu.y;
            cpu.write(bus, a, v);
            4
        }
        0x8C => {
            let (a, _) = cpu.addr_absolute(bus);
            let v = cpu.y;
            cpu.write(bus, a, v);
            4
        }

        // --- Transfers ---
        0xAA => {
            cpu.x = cpu.a;
            cpu.set_nz(cpu.x);
            2
        } // TAX
        0xA8 => {
            cpu.y = cpu.a;
            cpu.set_nz(cpu.y);
            2
        } // TAY
        0xBA => {
            cpu.x = cpu.sp;
            cpu.set_nz(cpu.x);
            2
        } // TSX
        0x8A => {
            cpu.a = cpu.x;
            cpu.set_nz(cpu.a);
            2
        } // TXA
        0x9A => {
            cpu.sp = cpu.x;
            2
        } // TXS — no flags
        0x98 => {
            cpu.a = cpu.y;
            cpu.set_nz(cpu.a);
            2
        } // TYA

        // --- Unofficial opcodes ---

        // 1-byte NOPs
        0x1A | 0x3A | 0x5A | 0x7A | 0xDA | 0xFA => 2,

        // DOP — 2-byte NOPs (immediate)
        0x80 | 0x82 | 0x89 | 0xC2 | 0xE2 => {
            cpu.fetch(bus);
            2
        }

        // DOP — 2-byte NOPs (zero page)
        0x04 | 0x44 | 0x64 => {
            cpu.addr_zero_page(bus);
            3
        }

        // DOP — 2-byte NOPs (zero page,X)
        0x14 | 0x34 | 0x54 | 0x74 | 0xD4 | 0xF4 => {
            cpu.addr_zero_page_x(bus);
            4
        }

        // TOP — 3-byte NOP (absolute)
        0x0C => {
            cpu.addr_absolute(bus);
            4
        }

        // TOP — 3-byte NOPs (absolute,X) — page-cross +1
        0x1C | 0x3C | 0x5C | 0x7C | 0xDC | 0xFC => {
            let (a, p) = cpu.addr_absolute_x(bus);
            let _ = cpu.read(bus, a); // consume the read
            4 + p as u8
        }

        // $EB — unofficial duplicate SBC #n
        0xEB => {
            let v = cpu.fetch(bus);
            sbc(cpu, v);
            2
        }

        // --- SLO (ASL memory then ORA into A) ---
        0x07 => {
            let (a, _) = cpu.addr_zero_page(bus);
            slo(cpu, bus, a);
            5
        }
        0x17 => {
            let (a, _) = cpu.addr_zero_page_x(bus);
            slo(cpu, bus, a);
            6
        }
        0x0F => {
            let (a, _) = cpu.addr_absolute(bus);
            slo(cpu, bus, a);
            6
        }
        0x1F => {
            let a = cpu.addr_absolute_x_rmw(bus);
            slo(cpu, bus, a);
            7
        }
        0x1B => {
            let a = cpu.addr_absolute_y_rmw(bus);
            slo(cpu, bus, a);
            7
        }
        0x03 => {
            let (a, _) = cpu.addr_indirect_x(bus);
            slo(cpu, bus, a);
            8
        }
        0x13 => {
            let (a, _) = cpu.addr_indirect_y(bus);
            slo(cpu, bus, a);
            8
        }

        // --- RLA (ROL memory then AND into A) ---
        0x27 => {
            let (a, _) = cpu.addr_zero_page(bus);
            rla(cpu, bus, a);
            5
        }
        0x37 => {
            let (a, _) = cpu.addr_zero_page_x(bus);
            rla(cpu, bus, a);
            6
        }
        0x2F => {
            let (a, _) = cpu.addr_absolute(bus);
            rla(cpu, bus, a);
            6
        }
        0x3F => {
            let a = cpu.addr_absolute_x_rmw(bus);
            rla(cpu, bus, a);
            7
        }
        0x3B => {
            let a = cpu.addr_absolute_y_rmw(bus);
            rla(cpu, bus, a);
            7
        }
        0x23 => {
            let (a, _) = cpu.addr_indirect_x(bus);
            rla(cpu, bus, a);
            8
        }
        0x33 => {
            let (a, _) = cpu.addr_indirect_y(bus);
            rla(cpu, bus, a);
            8
        }

        // --- SRE (LSR memory then EOR into A) ---
        0x47 => {
            let (a, _) = cpu.addr_zero_page(bus);
            sre(cpu, bus, a);
            5
        }
        0x57 => {
            let (a, _) = cpu.addr_zero_page_x(bus);
            sre(cpu, bus, a);
            6
        }
        0x4F => {
            let (a, _) = cpu.addr_absolute(bus);
            sre(cpu, bus, a);
            6
        }
        0x5F => {
            let a = cpu.addr_absolute_x_rmw(bus);
            sre(cpu, bus, a);
            7
        }
        0x5B => {
            let a = cpu.addr_absolute_y_rmw(bus);
            sre(cpu, bus, a);
            7
        }
        0x43 => {
            let (a, _) = cpu.addr_indirect_x(bus);
            sre(cpu, bus, a);
            8
        }
        0x53 => {
            let (a, _) = cpu.addr_indirect_y(bus);
            sre(cpu, bus, a);
            8
        }

        // --- RRA (ROR memory then ADC into A) ---
        0x67 => {
            let (a, _) = cpu.addr_zero_page(bus);
            rra(cpu, bus, a);
            5
        }
        0x77 => {
            let (a, _) = cpu.addr_zero_page_x(bus);
            rra(cpu, bus, a);
            6
        }
        0x6F => {
            let (a, _) = cpu.addr_absolute(bus);
            rra(cpu, bus, a);
            6
        }
        0x7F => {
            let a = cpu.addr_absolute_x_rmw(bus);
            rra(cpu, bus, a);
            7
        }
        0x7B => {
            let a = cpu.addr_absolute_y_rmw(bus);
            rra(cpu, bus, a);
            7
        }
        0x63 => {
            let (a, _) = cpu.addr_indirect_x(bus);
            rra(cpu, bus, a);
            8
        }
        0x73 => {
            let (a, _) = cpu.addr_indirect_y(bus);
            rra(cpu, bus, a);
            8
        }

        // --- SAX (Store A & X) ---
        0x87 => {
            let (a, _) = cpu.addr_zero_page(bus);
            sax(cpu, bus, a);
            3
        }
        0x97 => {
            let (a, _) = cpu.addr_zero_page_y(bus);
            sax(cpu, bus, a);
            4
        }
        0x8F => {
            let (a, _) = cpu.addr_absolute(bus);
            sax(cpu, bus, a);
            4
        }
        0x83 => {
            let (a, _) = cpu.addr_indirect_x(bus);
            sax(cpu, bus, a);
            6
        }

        // --- LAX (Load A and X from memory) ---
        0xA7 => {
            let (a, _) = cpu.addr_zero_page(bus);
            lax_m(cpu, bus, a);
            3
        }
        0xB7 => {
            let (a, _) = cpu.addr_zero_page_y(bus);
            lax_m(cpu, bus, a);
            4
        }
        0xAF => {
            let (a, _) = cpu.addr_absolute(bus);
            lax_m(cpu, bus, a);
            4
        }
        0xBF => {
            let (a, p) = cpu.addr_absolute_y(bus);
            lax_m(cpu, bus, a);
            4 + p as u8
        }
        0xA3 => {
            let (a, _) = cpu.addr_indirect_x(bus);
            lax_m(cpu, bus, a);
            6
        }
        0xB3 => {
            let (a, p) = cpu.addr_indirect_y(bus);
            lax_m(cpu, bus, a);
            5 + p as u8
        }

        // LAX #imm (ATX) — load both A and X with the immediate value
        0xAB => {
            let v = cpu.fetch(bus);
            lax(cpu, v);
            2
        }

        // --- AXS / SBX (A & X minus immediate, result into X) ---
        0xCB => {
            let v = cpu.fetch(bus);
            axs(cpu, v);
            2
        }

        // --- DCP (DEC memory then CMP with A) ---
        0xC7 => {
            let (a, _) = cpu.addr_zero_page(bus);
            dcp(cpu, bus, a);
            5
        }
        0xD7 => {
            let (a, _) = cpu.addr_zero_page_x(bus);
            dcp(cpu, bus, a);
            6
        }
        0xCF => {
            let (a, _) = cpu.addr_absolute(bus);
            dcp(cpu, bus, a);
            6
        }
        0xDF => {
            let a = cpu.addr_absolute_x_rmw(bus);
            dcp(cpu, bus, a);
            7
        }
        0xDB => {
            let a = cpu.addr_absolute_y_rmw(bus);
            dcp(cpu, bus, a);
            7
        }
        0xC3 => {
            let (a, _) = cpu.addr_indirect_x(bus);
            dcp(cpu, bus, a);
            8
        }
        0xD3 => {
            let (a, _) = cpu.addr_indirect_y(bus);
            dcp(cpu, bus, a);
            8
        }

        // --- ISC / ISB (INC memory then SBC from A) ---
        0xE7 => {
            let (a, _) = cpu.addr_zero_page(bus);
            isc(cpu, bus, a);
            5
        }
        0xF7 => {
            let (a, _) = cpu.addr_zero_page_x(bus);
            isc(cpu, bus, a);
            6
        }
        0xEF => {
            let (a, _) = cpu.addr_absolute(bus);
            isc(cpu, bus, a);
            6
        }
        0xFF => {
            let a = cpu.addr_absolute_x_rmw(bus);
            isc(cpu, bus, a);
            7
        }
        0xFB => {
            let a = cpu.addr_absolute_y_rmw(bus);
            isc(cpu, bus, a);
            7
        }
        0xE3 => {
            let (a, _) = cpu.addr_indirect_x(bus);
            isc(cpu, bus, a);
            8
        }
        0xF3 => {
            let (a, _) = cpu.addr_indirect_y(bus);
            isc(cpu, bus, a);
            8
        }

        // --- ANC (AND #n, copy N to C) ---
        0x0B | 0x2B => {
            let v = cpu.fetch(bus);
            anc(cpu, v);
            2
        }

        // --- ALR (AND #n then LSR accumulator) ---
        0x4B => {
            let v = cpu.fetch(bus);
            alr(cpu, v);
            2
        }

        // --- ARR (AND #n then ROR accumulator, special V/C) ---
        0x6B => {
            let v = cpu.fetch(bus);
            arr(cpu, v);
            2
        }

        // --- ANE / XAA (unstable; A = X & imm) ---
        0x8B => {
            let v = cpu.fetch(bus);
            cpu.a = cpu.x & v;
            cpu.set_nz(cpu.a);
            2
        }

        // --- LAS (AND memory with SP, load all three) ---
        0xBB => {
            let (a, p) = cpu.addr_absolute_y(bus);
            las(cpu, bus, a);
            4 + p as u8
        }

        // --- Unstable high-byte store opcodes (SHA/SHX/SHY/TAS) ---
        // AccuracyCoin "behavior 1", the common NES CPU (verified against its
        // UnOp_SHA/SHS/SHY/SHX tests, which distinguish the manufacturer
        // variants): value = reg & (base_hi + 1); when the index crosses a
        // page, the write address's high byte BECOMES that value (not the
        // pre-carry high byte, not the carried one). A DMC DMA halting the
        // dummy-read cycle right before the write drops the & (base_hi + 1)
        // entirely — see sh_store.

        // SHA (ind),Y — value = A & X & (base_hi + 1)
        0x93 => {
            let ptr = cpu.fetch(bus) as u16;
            let lo = bus.read(ptr) as u16;
            let hi = bus.read(ptr.wrapping_add(1) & 0x00FF) as u16;
            let base = (hi << 8) | lo;
            let mask = cpu.a & cpu.x;
            sh_store(cpu, bus, base, cpu.y, mask);
            6
        }
        // SHY abs,X — value = Y & (base_hi + 1)
        0x9C => {
            let base = cpu.fetch_u16(bus);
            sh_store(cpu, bus, base, cpu.x, cpu.y);
            5
        }
        // SHX abs,Y — value = X & (base_hi + 1)
        0x9E => {
            let base = cpu.fetch_u16(bus);
            sh_store(cpu, bus, base, cpu.y, cpu.x);
            5
        }
        // SHA abs,Y — value = A & X & (base_hi + 1)
        0x9F => {
            let base = cpu.fetch_u16(bus);
            let mask = cpu.a & cpu.x;
            sh_store(cpu, bus, base, cpu.y, mask);
            5
        }
        // TAS (SHS) abs,Y — SP = A & X; value = SP & (base_hi + 1)
        0x9B => {
            let base = cpu.fetch_u16(bus);
            cpu.sp = cpu.a & cpu.x;
            let mask = cpu.sp;
            sh_store(cpu, bus, base, cpu.y, mask);
            5
        }
    }
}

/// Shared tail of the unstable high-byte stores (SHA/SHX/SHY/TAS): the
/// unconditional pre-carry dummy read (these are fixed 5/6-cycle stores — the
/// cycle-4 read happens whether or not a page is crossed, like every indexed
/// store), the value computation, and the corrupted-high-byte write.
///
/// `reg_mask` is the register part of the value (A&X for SHA/TAS, X for SHX,
/// Y for SHY). The stored value is `reg_mask & (base_hi + 1)` — unless RDY
/// was low during the dummy-read cycle immediately before the write, in
/// which case the & is dropped and the raw register is stored (AccuracyCoin:
/// "SHY just becomes STY if a DMA occurs on the right cpu cycle"; its
/// sub-tests time RDY to go low 2 cycles before the write, so it is low
/// through the dummy read). RDY-low during that cycle shows up in one of two
/// ways: a DMC DMA was serviced ON the dummy read itself (`dmc_dma_count`
/// bumped — the request rose one cycle earlier), or the request rose DURING
/// the dummy read and is still pending afterwards (`dmc_dma_pending` — the
/// CPU won't halt until its next read, after the write, but RDY is already
/// low at the write and corrupts the value on hardware). On a page cross the
/// write address's high byte is replaced by the stored value.
fn sh_store(cpu: &mut Cpu, bus: &mut dyn Bus, base: u16, index: u8, reg_mask: u8) {
    let addr = base.wrapping_add(index as u16);
    let lo = addr & 0x00FF;
    let crossed = (base & 0xFF00) != (addr & 0xFF00);
    let precarry = (base & 0xFF00) | lo;
    let dma_before = bus.dmc_dma_count();
    let _ = bus.read(precarry);
    let rdy_low = bus.dmc_dma_count() != dma_before || bus.dmc_dma_pending();
    let value = if rdy_low {
        reg_mask
    } else {
        reg_mask & ((base >> 8) as u8).wrapping_add(1)
    };
    let write_addr = if crossed {
        (u16::from(value) << 8) | lo
    } else {
        addr
    };
    cpu.write(bus, write_addr, value);
}

// ---------------------------------------------------------------------------
// Operation helpers
// ---------------------------------------------------------------------------

fn adc(cpu: &mut Cpu, m: u8) {
    let a = cpu.a as u16;
    let m16 = m as u16;
    let c = cpu.flag(FLAG_C) as u16;
    let result = a + m16 + c;
    cpu.set_flag(FLAG_C, result > 0xFF);
    cpu.set_flag(FLAG_V, (!(a ^ m16) & (a ^ result) & 0x80) != 0);
    cpu.a = result as u8;
    cpu.set_nz(cpu.a);
}

fn adc_m(cpu: &mut Cpu, bus: &mut dyn Bus, addr: u16) {
    let v = cpu.read(bus, addr);
    adc(cpu, v);
}

fn sbc(cpu: &mut Cpu, m: u8) {
    adc(cpu, m ^ 0xFF);
}

fn sbc_m(cpu: &mut Cpu, bus: &mut dyn Bus, addr: u16) {
    let v = cpu.read(bus, addr);
    sbc(cpu, v);
}

fn and(cpu: &mut Cpu, m: u8) {
    cpu.a &= m;
    cpu.set_nz(cpu.a);
}

fn and_m(cpu: &mut Cpu, bus: &mut dyn Bus, addr: u16) {
    let v = cpu.read(bus, addr);
    and(cpu, v);
}

fn ora(cpu: &mut Cpu, m: u8) {
    cpu.a |= m;
    cpu.set_nz(cpu.a);
}

fn ora_m(cpu: &mut Cpu, bus: &mut dyn Bus, addr: u16) {
    let v = cpu.read(bus, addr);
    ora(cpu, v);
}

fn eor(cpu: &mut Cpu, m: u8) {
    cpu.a ^= m;
    cpu.set_nz(cpu.a);
}

fn eor_m(cpu: &mut Cpu, bus: &mut dyn Bus, addr: u16) {
    let v = cpu.read(bus, addr);
    eor(cpu, v);
}

fn asl(cpu: &mut Cpu, v: u8) -> u8 {
    cpu.set_flag(FLAG_C, v & 0x80 != 0);
    let r = v << 1;
    cpu.set_nz(r);
    r
}

fn lsr(cpu: &mut Cpu, v: u8) -> u8 {
    cpu.set_flag(FLAG_C, v & 0x01 != 0);
    let r = v >> 1;
    cpu.set_nz(r);
    r
}

fn rol(cpu: &mut Cpu, v: u8) -> u8 {
    let old_c = cpu.flag(FLAG_C) as u8;
    cpu.set_flag(FLAG_C, v & 0x80 != 0);
    let r = (v << 1) | old_c;
    cpu.set_nz(r);
    r
}

fn ror(cpu: &mut Cpu, v: u8) -> u8 {
    let old_c = cpu.flag(FLAG_C) as u8;
    cpu.set_flag(FLAG_C, v & 0x01 != 0);
    let r = (v >> 1) | (old_c << 7);
    cpu.set_nz(r);
    r
}

fn inc(cpu: &mut Cpu, v: u8) -> u8 {
    let r = v.wrapping_add(1);
    cpu.set_nz(r);
    r
}

fn dec(cpu: &mut Cpu, v: u8) -> u8 {
    let r = v.wrapping_sub(1);
    cpu.set_nz(r);
    r
}

/// Read-modify-write: reads M, applies op, writes result back.
fn rmw(cpu: &mut Cpu, bus: &mut dyn Bus, addr: u16, op: fn(&mut Cpu, u8) -> u8) {
    let v = cpu.read(bus, addr);
    bus.write(addr, v); // write original (RMW behaviour)
    let r = op(cpu, v);
    cpu.write(bus, addr, r);
}

fn bit(cpu: &mut Cpu, bus: &mut dyn Bus, addr: u16) {
    let m = cpu.read(bus, addr);
    cpu.set_flag(FLAG_Z, cpu.a & m == 0);
    cpu.set_flag(FLAG_N, m & FLAG_N != 0);
    cpu.set_flag(FLAG_V, m & FLAG_V != 0);
}

fn cmp(cpu: &mut Cpu, reg: u8, m: u8) {
    let result = reg.wrapping_sub(m);
    cpu.set_flag(FLAG_C, reg >= m);
    cpu.set_nz(result);
}

fn cmp_m(cpu: &mut Cpu, bus: &mut dyn Bus, reg: u8, addr: u16) {
    let v = cpu.read(bus, addr);
    cmp(cpu, reg, v);
}

fn lda(cpu: &mut Cpu, v: u8) {
    cpu.a = v;
    cpu.set_nz(v);
}
fn ldx(cpu: &mut Cpu, v: u8) {
    cpu.x = v;
    cpu.set_nz(v);
}
fn ldy(cpu: &mut Cpu, v: u8) {
    cpu.y = v;
    cpu.set_nz(v);
}

fn lda_m(cpu: &mut Cpu, bus: &mut dyn Bus, addr: u16) {
    let v = cpu.read(bus, addr);
    lda(cpu, v);
}
fn ldx_m(cpu: &mut Cpu, bus: &mut dyn Bus, addr: u16) {
    let v = cpu.read(bus, addr);
    ldx(cpu, v);
}
fn ldy_m(cpu: &mut Cpu, bus: &mut dyn Bus, addr: u16) {
    let v = cpu.read(bus, addr);
    ldy(cpu, v);
}

fn branch(cpu: &mut Cpu, bus: &mut dyn Bus, taken: bool) -> u8 {
    let offset = cpu.fetch(bus) as i8 as i16; // T2: fetch offset
    if !taken {
        return 2; // T1 + T2
    }
    let old_pc = cpu.pc;
    let target = old_pc.wrapping_add(offset as u16);

    if (old_pc & 0xFF00) != (target & 0xFF00) {
        // Page cross: T3 spurious read at page-wrong address, then T4 (BranchPageFix).
        let pcl = (old_pc as u8).wrapping_add(offset as u8) as u16;
        let page_wrong_pc = (old_pc & 0xFF00) | pcl;
        let _ = bus.read(page_wrong_pc); // T3 spurious read
        cpu.pc = page_wrong_pc;
        cpu.branch_target_hi = (target >> 8) as u8;
        cpu.enqueue(super::MicroOp::BranchPageFix);
        3 // T1+T2+T3; T4 comes from BranchPageFix micro-op
    } else {
        // No page cross: set correct PC immediately (T3 implicit).
        cpu.pc = target;
        3 // T1+T2+T3
    }
}

fn jsr(cpu: &mut Cpu, bus: &mut dyn Bus) {
    // T2: fetch ADL. T3: internal operation (predecrement S) — on real
    // hardware this still drives the address bus with a dummy read of the
    // current stack location before the two pushes. T4-T5: push return
    // address (address of the ADH byte). T6: fetch ADH.
    let lo = cpu.fetch(bus) as u16;
    let _ = bus.read(0x0100 | cpu.sp as u16);
    cpu.push_u16(bus, cpu.pc);
    let hi = cpu.fetch(bus) as u16;
    cpu.pc = (hi << 8) | lo;
}

fn rts(cpu: &mut Cpu, bus: &mut dyn Bus) {
    // T3: internal operation (increment S) — dummy read of the stack
    // location before it's incremented (pop_u16 performs the real
    // increment+read for T4/T5). T6: dummy read at the popped PC before the
    // final increment — a real bus cycle, distinct from the next
    // instruction's own opcode fetch at PC+1.
    let _ = bus.read(0x0100 | cpu.sp as u16);
    let target = cpu.pop_u16(bus);
    let _ = bus.read(target);
    cpu.pc = target.wrapping_add(1);
}

fn rti(cpu: &mut Cpu, bus: &mut dyn Bus) {
    // T3: internal operation (increment S) — dummy read before the first
    // real pop (see rts() above for the same pattern).
    let _ = bus.read(0x0100 | cpu.sp as u16);
    let p = cpu.pop(bus);
    cpu.p = (p & !FLAG_B) | FLAG_U;
    cpu.pc = cpu.pop_u16(bus);
    // RTI's FLAG_I restore takes effect immediately (unlike CLI/SEI/PLP which
    // delay by one instruction). If RTI restores FLAG_I=1, block any pending
    // deferred IRQ — the 6502 does not re-fire the IRQ upon return from handler.
    if cpu.flag(FLAG_I) {
        cpu.irq_deferred_blocked = true;
    }
}

// ---------------------------------------------------------------------------
// Unofficial / illegal opcode helpers
// ---------------------------------------------------------------------------

fn slo(cpu: &mut Cpu, bus: &mut dyn Bus, addr: u16) {
    let v = cpu.read(bus, addr);
    bus.write(addr, v);
    let shifted = asl(cpu, v);
    cpu.write(bus, addr, shifted);
    ora(cpu, shifted);
}

fn rla(cpu: &mut Cpu, bus: &mut dyn Bus, addr: u16) {
    let v = cpu.read(bus, addr);
    bus.write(addr, v);
    let rotated = rol(cpu, v);
    cpu.write(bus, addr, rotated);
    and(cpu, rotated);
}

fn sre(cpu: &mut Cpu, bus: &mut dyn Bus, addr: u16) {
    let v = cpu.read(bus, addr);
    bus.write(addr, v);
    let shifted = lsr(cpu, v);
    cpu.write(bus, addr, shifted);
    eor(cpu, shifted);
}

fn rra(cpu: &mut Cpu, bus: &mut dyn Bus, addr: u16) {
    let v = cpu.read(bus, addr);
    bus.write(addr, v);
    let rotated = ror(cpu, v);
    cpu.write(bus, addr, rotated);
    adc(cpu, rotated);
}

fn dcp(cpu: &mut Cpu, bus: &mut dyn Bus, addr: u16) {
    let v = cpu.read(bus, addr);
    bus.write(addr, v);
    let decremented = dec(cpu, v);
    cpu.write(bus, addr, decremented);
    cmp(cpu, cpu.a, decremented);
}

fn isc(cpu: &mut Cpu, bus: &mut dyn Bus, addr: u16) {
    let v = cpu.read(bus, addr);
    bus.write(addr, v);
    let incremented = inc(cpu, v);
    cpu.write(bus, addr, incremented);
    sbc(cpu, incremented);
}

fn sax(cpu: &mut Cpu, bus: &mut dyn Bus, addr: u16) {
    let v = cpu.a & cpu.x;
    cpu.write(bus, addr, v);
}

fn lax(cpu: &mut Cpu, v: u8) {
    cpu.a = v;
    cpu.x = v;
    cpu.set_nz(v);
}

fn lax_m(cpu: &mut Cpu, bus: &mut dyn Bus, addr: u16) {
    let v = cpu.read(bus, addr);
    lax(cpu, v);
}

fn las(cpu: &mut Cpu, bus: &mut dyn Bus, addr: u16) {
    let v = cpu.read(bus, addr) & cpu.sp;
    cpu.a = v;
    cpu.x = v;
    cpu.sp = v;
    cpu.set_nz(v);
}

fn anc(cpu: &mut Cpu, m: u8) {
    cpu.a &= m;
    cpu.set_nz(cpu.a);
    cpu.set_flag(FLAG_C, cpu.a & 0x80 != 0);
}

fn alr(cpu: &mut Cpu, m: u8) {
    cpu.a &= m;
    cpu.set_flag(FLAG_C, cpu.a & 0x01 != 0);
    cpu.a >>= 1;
    cpu.set_nz(cpu.a);
}

fn arr(cpu: &mut Cpu, m: u8) {
    let c = cpu.flag(FLAG_C) as u8;
    cpu.a = ((cpu.a & m) >> 1) | (c << 7);
    cpu.set_nz(cpu.a);
    cpu.set_flag(FLAG_C, cpu.a & 0x40 != 0);
    cpu.set_flag(FLAG_V, ((cpu.a & 0x40) ^ ((cpu.a & 0x20) << 1)) != 0);
}

fn axs(cpu: &mut Cpu, m: u8) {
    let lhs = cpu.a & cpu.x;
    cpu.set_flag(FLAG_C, lhs >= m);
    cpu.x = lhs.wrapping_sub(m);
    cpu.set_nz(cpu.x);
}
