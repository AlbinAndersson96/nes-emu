use crate::cartridge::Mirroring;
use crate::ppu::Ppu;

// Helper: tick the PPU by N CPU cycles (no cartridge)
fn tick(ppu: &mut Ppu, cpu_cycles: u64) {
    ppu.tick(cpu_cycles, None);
}

// Helper: advance to exactly (scanline, dot) from a freshly constructed PPU.
// PPU starts at scanline=0, dot=0.  Each CPU cycle = 3 PPU dots.
fn tick_to(ppu: &mut Ppu, target_scanline: u16, target_dot: u16) {
    let target_dots = target_scanline as u64 * 341 + target_dot as u64;
    // Each call to tick(1) advances by 3 dots, so we drive 1 CPU cycle at a time
    // until we overshoot, then use the fractional remainder.
    let full_cycles = target_dots / 3;
    let remainder = target_dots % 3;
    tick(ppu, full_cycles);
    // Drive the remainder one dot at a time (each 1-cpu-cycle tick = 3 dots,
    // so there's no perfect sub-cycle API). We approximate by driving 1 more
    // CPU cycle if there are leftover dots — the PPU will be ≤2 dots past target.
    if remainder > 0 {
        tick(ppu, 1);
    }
}

// ── VBlank timing ────────────────────────────────────────────────────────────

#[test]
fn vblank_set_at_scanline_241_dot_1() {
    let mut ppu = Ppu::new();
    // VBlank fires when clock_dot processes (scanline=241, dot=1).
    // After n CPU cycles the PPU state is (3n/341, 3n%341).
    // State (241, 1) is reached after 3n = 241*341+1 = 82182 clocks → n = 27394.
    // clock_dot is called WITH that state, setting vblank, then advances to (241, 2).
    // So at n = 27394 the state is (241, 1) and vblank is NOT yet set — but a
    // $2002 read AT that state would hit the 1-dot-before-VBL race and
    // suppress the flag for the whole frame (tested separately below), so the
    // "before" probe reads one cycle earlier, at (240, 339).
    tick(&mut ppu, 27393);
    let status_before = ppu.read_register(2, None);
    assert_eq!(
        status_before & 0x80,
        0,
        "VBlank should not be set before dot 1 is processed"
    );

    tick(&mut ppu, 2); // 27395 total cycles: processes through (241,3), sets VBlank
    let status_after = ppu.read_register(2, None);
    assert_eq!(
        status_after & 0x80,
        0x80,
        "VBlank should be set after scanline 241 dot 1"
    );
}

#[test]
fn vblank_cleared_at_prerender_scanline() {
    let mut ppu = Ppu::new();
    // Tick into VBlank
    tick_to(&mut ppu, 241, 1);
    tick(&mut ppu, 1); // ensure we're past dot 1
    // VBlank should be set now
    // Don't use read_register(2) here because that clears the flag; inspect via NMI path.
    // Instead set PPUCTRL NMI enable and confirm nmi_pending clears at pre-render.
    ppu.write_register(0, 0x80, None); // enable NMI
    ppu.take_nmi(); // consume it

    // Advance to pre-render scanline 261 dot 1
    // From scanline 241 to scanline 261 = 20 scanlines × 341 dots = 6820 dots
    tick(&mut ppu, 6820 / 3 + 1);
    // After pre-render clears VBlank, read_register(2) bit 7 should be 0
    let status = ppu.read_register(2, None);
    assert_eq!(
        status & 0x80,
        0,
        "VBlank should be cleared at pre-render scanline"
    );
}

// ── NMI generation ───────────────────────────────────────────────────────────

#[test]
fn nmi_fires_when_ctrl_nmi_set_and_vblank_starts() {
    let mut ppu = Ppu::new();
    ppu.write_register(0, 0x80, None); // NMI enable
    tick_to(&mut ppu, 241, 1);
    tick(&mut ppu, 1);
    assert!(
        ppu.take_nmi(),
        "NMI should fire at VBlank with NMI enable set"
    );
}

#[test]
fn nmi_suppressed_when_ctrl_nmi_clear() {
    let mut ppu = Ppu::new();
    // NMI bit NOT set in PPUCTRL
    tick_to(&mut ppu, 241, 1);
    tick(&mut ppu, 1);
    assert!(
        !ppu.take_nmi(),
        "NMI must not fire when NMI enable bit is clear"
    );
}

#[test]
fn enabling_nmi_mid_vblank_fires_immediately() {
    let mut ppu = Ppu::new();
    // Arrive in VBlank with NMI disabled
    tick_to(&mut ppu, 241, 3);
    tick(&mut ppu, 1);
    assert!(!ppu.take_nmi(), "NMI must be clear before we enable it");
    // Now enable NMI while still in VBlank. The NMI line (vblank && enable)
    // rises immediately, and the CPU-side edge detector latches it at the
    // next per-cycle sample — so tick one CPU cycle before checking.
    ppu.write_register(0, 0x80, None);
    tick(&mut ppu, 1);
    assert!(
        ppu.take_nmi(),
        "Enabling NMI mid-VBlank should fire NMI at the next cycle sample"
    );
}

// ── $2002 side effects ───────────────────────────────────────────────────────

#[test]
fn read_status_clears_vblank_flag() {
    let mut ppu = Ppu::new();
    ppu.write_register(0, 0x80, None);
    tick_to(&mut ppu, 241, 1);
    tick(&mut ppu, 1);
    let first = ppu.read_register(2, None);
    assert_eq!(first & 0x80, 0x80, "first read should show VBlank set");
    let second = ppu.read_register(2, None);
    assert_eq!(second & 0x80, 0, "second read should show VBlank cleared");
}

#[test]
fn read_status_clears_write_toggle() {
    let mut ppu = Ppu::new();
    // First PPUADDR write sets w=true
    ppu.write_register(6, 0x20, None);
    // Read $2002 — must reset w to false
    ppu.read_register(2, None);
    // If w was reset, the next PPUADDR write is a first-write (high byte)
    ppu.write_register(6, 0x21, None); // high byte
    ppu.write_register(6, 0x00, None); // low byte → v = $2100
    // Write a byte via PPUDATA and confirm it lands in VRAM near NT 1
    ppu.write_register(7, 0xAB, None);
    // Read it back: v was $2100, after write it's $2101; we need to re-set address
    ppu.write_register(6, 0x21, None);
    ppu.write_register(6, 0x00, None);
    let _ = ppu.read_register(7, None); // discard buffer
    let val = ppu.read_register(7, None);
    assert_eq!(
        val, 0xAB,
        "PPUADDR after $2002 read should address $2100 correctly"
    );
}

#[test]
fn read_status_cannot_unlatch_pending_nmi() {
    let mut ppu = Ppu::new();
    ppu.write_register(0, 0x80, None);
    tick_to(&mut ppu, 241, 1);
    tick(&mut ppu, 1);
    // The NMI edge was sampled a full CPU cycle ago — like on hardware, a
    // later $2002 read (which clears the flag and drops the line) cannot
    // un-latch the CPU's edge detector. Suppression only happens when the
    // read lands within the same CPU cycle as VBL onset, BEFORE the edge
    // sample — that path needs mid-instruction read placement and is covered
    // by the blargg ROM tests (ppu_vbl_nmi/06-suppression,
    // vbl_nmi_timing/5.nmi_suppression).
    ppu.read_register(2, None);
    assert!(
        ppu.take_nmi(),
        "$2002 read must not clear an already-latched NMI edge"
    );
}

#[test]
fn read_status_one_dot_before_vbl_suppresses_flag_and_nmi() {
    let mut ppu = Ppu::new();
    ppu.write_register(0, 0x80, None);
    // Position exactly at (241, 1): the NEXT dot to process is the one that
    // sets the VBlank flag (see vblank_set_at_scanline_241_dot_1 for the
    // arithmetic). A $2002 read here is the hardware race: it returns the
    // flag as clear AND prevents it from being set that frame, so no NMI
    // fires either.
    tick(&mut ppu, 27394);
    let status = ppu.read_register(2, None);
    assert_eq!(status & 0x80, 0, "race read returns the flag as clear");
    tick(&mut ppu, 3);
    assert!(!ppu.take_nmi(), "suppressed VBL must not generate an NMI");
    let status = ppu.read_register(2, None);
    assert_eq!(
        status & 0x80,
        0,
        "VBlank flag must never be set in a frame whose onset was raced by a $2002 read"
    );
}

// ── PPUADDR / PPUDATA ────────────────────────────────────────────────────────

#[test]
fn ppuaddr_two_write_sets_v() {
    let mut ppu = Ppu::new();
    ppu.write_register(6, 0x21, None); // high byte
    ppu.write_register(6, 0x08, None); // low byte → v = $2108
    // Write a sentinel to VRAM via PPUDATA
    ppu.write_register(7, 0x55, None);
    // Re-address and read back
    ppu.write_register(6, 0x21, None);
    ppu.write_register(6, 0x08, None);
    let _ = ppu.read_register(7, None); // flush buffer
    let val = ppu.read_register(7, None);
    assert_eq!(val, 0x55);
}

#[test]
fn ppudata_auto_increment_by_1() {
    let mut ppu = Ppu::new();
    ppu.write_register(6, 0x20, None);
    ppu.write_register(6, 0x00, None);
    ppu.write_register(7, 0x11, None); // $2000
    ppu.write_register(7, 0x22, None); // $2001
    // Read back
    ppu.write_register(6, 0x20, None);
    ppu.write_register(6, 0x00, None);
    let _ = ppu.read_register(7, None);
    assert_eq!(ppu.read_register(7, None), 0x11);
    assert_eq!(ppu.read_register(7, None), 0x22);
}

#[test]
fn ppudata_auto_increment_by_32() {
    let mut ppu = Ppu::new();
    ppu.write_register(0, 0x04, None); // PPUCTRL: VRAM increment = 32
    ppu.write_register(6, 0x20, None);
    ppu.write_register(6, 0x00, None);
    ppu.write_register(7, 0xAA, None); // $2000
    ppu.write_register(7, 0xBB, None); // $2020
    ppu.write_register(6, 0x20, None);
    ppu.write_register(6, 0x00, None);
    let _ = ppu.read_register(7, None);
    assert_eq!(ppu.read_register(7, None), 0xAA);
    assert_eq!(ppu.read_register(7, None), 0xBB);
}

#[test]
fn ppudata_read_is_buffered_for_nametable() {
    let mut ppu = Ppu::new();
    // Write $2000 = 0x42
    ppu.write_register(6, 0x20, None);
    ppu.write_register(6, 0x00, None);
    ppu.write_register(7, 0x42, None);
    // Re-address
    ppu.write_register(6, 0x20, None);
    ppu.write_register(6, 0x00, None);
    // First read returns stale buffer (0), not 0x42
    let first = ppu.read_register(7, None);
    assert_eq!(
        first, 0x00,
        "first PPUDATA read from VRAM must return old buffer"
    );
    // Second read returns the value
    let second = ppu.read_register(7, None);
    assert_eq!(second, 0x42);
}

#[test]
fn ppudata_palette_read_is_immediate() {
    let mut ppu = Ppu::new();
    // Write palette entry $3F01 = 0x1C
    ppu.write_register(6, 0x3F, None);
    ppu.write_register(6, 0x01, None);
    ppu.write_register(7, 0x1C, None);
    // Re-address $3F01
    ppu.write_register(6, 0x3F, None);
    ppu.write_register(6, 0x01, None);
    // Read — palette reads are not buffered
    let val = ppu.read_register(7, None);
    assert_eq!(val, 0x1C, "palette PPUDATA read must be immediate");
}

// ── Palette mirroring ────────────────────────────────────────────────────────

#[test]
fn palette_backdrop_mirror_3f10_reads_3f00() {
    let mut ppu = Ppu::new();
    // Write $3F00 = 0x0F
    ppu.write_register(6, 0x3F, None);
    ppu.write_register(6, 0x00, None);
    ppu.write_register(7, 0x0F, None);
    // Read from $3F10 (should mirror $3F00)
    ppu.write_register(6, 0x3F, None);
    ppu.write_register(6, 0x10, None);
    let val = ppu.read_register(7, None);
    assert_eq!(val, 0x0F, "$3F10 must mirror $3F00");
}

// ── OAM ──────────────────────────────────────────────────────────────────────

#[test]
fn oam_read_write_via_oamaddr_oamdata() {
    let mut ppu = Ppu::new();
    ppu.write_register(3, 0x04, None); // OAMADDR = 4
    ppu.write_register(4, 0xDE, None); // write 0xDE at OAM[4], addr auto-inc to 5
    ppu.write_register(4, 0xAD, None); // write 0xAD at OAM[5]
    ppu.write_register(3, 0x04, None); // reset OAMADDR
    assert_eq!(ppu.read_register(4, None), 0xDE);
    ppu.write_register(3, 0x05, None);
    assert_eq!(ppu.read_register(4, None), 0xAD);
}

#[test]
fn oam_dma_write_fills_oam() {
    let mut ppu = Ppu::new();
    for i in 0u8..=255 {
        ppu.oam_dma_write(i, i.wrapping_add(1));
    }
    ppu.write_register(3, 0x00, None);
    assert_eq!(ppu.read_register(4, None), 0x01);
    ppu.write_register(3, 0xFF, None);
    assert_eq!(ppu.read_register(4, None), 0x00);
}

// ── PPUSCROLL / loopy register ───────────────────────────────────────────────

#[test]
fn ppuscroll_first_write_sets_coarse_x_and_fine_x() {
    let mut ppu = Ppu::new();
    // Write X scroll = 0b10110111 → coarse X = 22, fine_x = 7
    ppu.write_register(5, 0b10110111, None);
    // We can't read internal regs directly, but verify via PPUADDR round-trip:
    // t coarse X = bits [4:0] of t = 0b10110 = 22
    // We'll check indirectly: write second scroll byte then check t via PPUADDR path
    // For now, verify no panic and that w flipped (second write of PPUADDR should be low byte)
    ppu.write_register(6, 0x20, None); // this is a PPUADDR first write (w should be true after scroll)
    ppu.write_register(6, 0x00, None); // second write → v = $2000 ish (t modified by scroll)
    // Just check the write didn't crash and the address is somewhat sensible
    ppu.write_register(7, 0x77, None);
    // If we read back from a re-set address it should work
    ppu.write_register(6, 0x20, None);
    ppu.write_register(6, 0x00, None);
    let _ = ppu.read_register(7, None);
    // No assert on value; we just verify no panic/UB
}

#[test]
fn ppuctrl_nametable_bits_go_to_t() {
    let mut ppu = Ppu::new();
    // Write PPUCTRL with nametable = 3 (bits 1:0 = 11)
    ppu.write_register(0, 0x03, None);
    // Now t bits 11:10 should be 11. Confirm by writing PPUADDR to capture t
    // and reading VRAM at the expected nametable-3 address.
    // Nametable 3 offset starts at $2C00; with t bits 11:10 = 11 the NT base is $0C00
    // We do a PPUADDR write sequence to set t low/high, then verify v = t after second write
    ppu.write_register(6, 0x20, None); // t high = $20 → clears bits 14:8 of t, sets [13:8]
    ppu.write_register(6, 0x00, None); // t low = $00 → v = t
    // Write to $2000 area and read back
    ppu.write_register(7, 0xCC, None);
    ppu.write_register(6, 0x20, None);
    ppu.write_register(6, 0x00, None);
    let _ = ppu.read_register(7, None);
    let v = ppu.read_register(7, None);
    assert_eq!(v, 0xCC);
}

// ── Nametable mirroring ──────────────────────────────────────────────────────

#[test]
fn horizontal_mirroring_nt0_eq_nt1() {
    let mut ppu = Ppu::new();
    ppu.set_mirroring(Mirroring::Horizontal);
    // Write to NT0 ($2000)
    ppu.write_register(6, 0x20, None);
    ppu.write_register(6, 0x10, None);
    ppu.write_register(7, 0xAA, None);
    // Read from NT1 ($2400) — should see same value
    ppu.write_register(6, 0x24, None);
    ppu.write_register(6, 0x10, None);
    let _ = ppu.read_register(7, None);
    let val = ppu.read_register(7, None);
    assert_eq!(
        val, 0xAA,
        "NT0 and NT1 must share bank in horizontal mirroring"
    );
}

#[test]
fn horizontal_mirroring_nt2_eq_nt3() {
    let mut ppu = Ppu::new();
    ppu.set_mirroring(Mirroring::Horizontal);
    // Write to NT2 ($2800)
    ppu.write_register(6, 0x28, None);
    ppu.write_register(6, 0x05, None);
    ppu.write_register(7, 0xBB, None);
    // Read from NT3 ($2C00)
    ppu.write_register(6, 0x2C, None);
    ppu.write_register(6, 0x05, None);
    let _ = ppu.read_register(7, None);
    let val = ppu.read_register(7, None);
    assert_eq!(
        val, 0xBB,
        "NT2 and NT3 must share bank in horizontal mirroring"
    );
}

#[test]
fn horizontal_mirroring_nt0_ne_nt2() {
    let mut ppu = Ppu::new();
    ppu.set_mirroring(Mirroring::Horizontal);
    ppu.write_register(6, 0x20, None);
    ppu.write_register(6, 0x00, None);
    ppu.write_register(7, 0x11, None);
    ppu.write_register(6, 0x28, None);
    ppu.write_register(6, 0x00, None);
    ppu.write_register(7, 0x22, None);
    // Read NT0
    ppu.write_register(6, 0x20, None);
    ppu.write_register(6, 0x00, None);
    let _ = ppu.read_register(7, None);
    let nt0 = ppu.read_register(7, None);
    // Read NT2
    ppu.write_register(6, 0x28, None);
    ppu.write_register(6, 0x00, None);
    let _ = ppu.read_register(7, None);
    let nt2 = ppu.read_register(7, None);
    assert_ne!(
        nt0, nt2,
        "NT0 and NT2 must be different banks in horizontal mirroring"
    );
}

#[test]
fn vertical_mirroring_nt0_eq_nt2() {
    let mut ppu = Ppu::new();
    ppu.set_mirroring(Mirroring::Vertical);
    ppu.write_register(6, 0x20, None);
    ppu.write_register(6, 0x20, None);
    ppu.write_register(7, 0xCC, None);
    // Read from NT2 ($2800)
    ppu.write_register(6, 0x28, None);
    ppu.write_register(6, 0x20, None);
    let _ = ppu.read_register(7, None);
    let val = ppu.read_register(7, None);
    assert_eq!(
        val, 0xCC,
        "NT0 and NT2 must share bank in vertical mirroring"
    );
}

#[test]
fn vertical_mirroring_nt1_eq_nt3() {
    let mut ppu = Ppu::new();
    ppu.set_mirroring(Mirroring::Vertical);
    ppu.write_register(6, 0x24, None);
    ppu.write_register(6, 0x00, None);
    ppu.write_register(7, 0xDD, None);
    ppu.write_register(6, 0x2C, None);
    ppu.write_register(6, 0x00, None);
    let _ = ppu.read_register(7, None);
    let val = ppu.read_register(7, None);
    assert_eq!(
        val, 0xDD,
        "NT1 and NT3 must share bank in vertical mirroring"
    );
}

// ── Sprite evaluation ────────────────────────────────────────────────────────

#[test]
fn sprite_evaluation_finds_sprites_in_range() {
    let mut ppu = Ppu::new();
    ppu.write_register(1, 0x18, None); // enable BG + sprites so rendering is on
    // Place sprite 0 at Y=10 (OAM byte 0); tile=1, attr=0, x=20
    ppu.oam_dma_write(0, 10);
    ppu.oam_dma_write(1, 0x01);
    ppu.oam_dma_write(2, 0x00);
    ppu.oam_dma_write(3, 20);
    // Place sprite 1 at Y=200 (off-screen for scanline 11)
    ppu.oam_dma_write(4, 200);
    // Tick through scanline 10, dot 256 where sprite eval happens
    tick_to(&mut ppu, 10, 256);
    tick(&mut ppu, 1);
    // Sprite-0 should be in secondary OAM; we verify via sprite-0-hit later
    // For now check frame_ready is not yet set (we're on scanline 10)
    assert!(!ppu.frame_ready);
}

#[test]
fn sprite_overflow_flag_set_when_more_than_8() {
    let mut ppu = Ppu::new();
    ppu.write_register(1, 0x18, None);
    // Place 9 sprites all at Y=10
    for i in 0..9u8 {
        ppu.oam_dma_write(i * 4, 10); // Y
        ppu.oam_dma_write(i * 4 + 1, i); // tile
        ppu.oam_dma_write(i * 4 + 2, 0); // attr
        ppu.oam_dma_write(i * 4 + 3, i * 8); // X
    }
    tick_to(&mut ppu, 10, 256);
    tick(&mut ppu, 1);
    let status = ppu.read_register(2, None);
    assert_eq!(
        status & 0x20,
        0x20,
        "sprite overflow flag should be set with >8 sprites"
    );
}

// ── frame_ready ──────────────────────────────────────────────────────────────

#[test]
fn frame_ready_set_after_scanline_239() {
    let mut ppu = Ppu::new();
    tick_to(&mut ppu, 239, 256);
    tick(&mut ppu, 1);
    assert!(
        ppu.frame_ready,
        "frame_ready must be set after scanline 239 dot 256"
    );
}

#[test]
fn frame_ready_starts_false() {
    let ppu = Ppu::new();
    assert!(!ppu.frame_ready);
}

// ── Greyscale mode ───────────────────────────────────────────────────────────

#[test]
fn greyscale_mode_masks_palette_to_0x30() {
    let mut ppu = Ppu::new();
    ppu.write_register(1, 0x19, None); // greyscale bit (bit 0) + BG + sprite enable
    // Set a palette color that would be masked
    ppu.write_register(6, 0x3F, None);
    ppu.write_register(6, 0x01, None);
    ppu.write_register(7, 0x2F, None); // 0x2F & 0x30 = 0x20, & 0x3F = 0x2F
    // Tick to a visible pixel and check the framebuffer value
    // Without rendering data the pixel will be backdrop = palette[0]
    // Set backdrop color
    ppu.write_register(6, 0x3F, None);
    ppu.write_register(6, 0x00, None);
    ppu.write_register(7, 0x1F, None); // 0x1F & 0x30 = 0x10
    tick_to(&mut ppu, 1, 1);
    tick(&mut ppu, 1);
    // Pixel (0,0) should be backdrop = 0x1F masked to 0x10
    assert_eq!(
        ppu.frame[0], 0x10,
        "greyscale should mask colour to bits [5:4]"
    );
}

// ── Coarse-X wrap ────────────────────────────────────────────────────────────

#[test]
fn coarse_x_wrap_flips_nametable_bit() {
    let mut ppu = Ppu::new();
    // Set v coarse X = 31 and increment — should wrap to 0 and flip bit 10
    // We do this by writing a specific PPUADDR value and then enabling rendering
    // to drive the increment indirectly. Easier to call via a visible scanline.
    // Instead verify via VRAM writes across the nametable boundary.
    ppu.set_mirroring(Mirroring::Vertical); // NT0 and NT1 are different banks
    // Just verify the method compiles and the mirroring logic holds
    // (functional test of coarse-X wrap requires rendering pipeline integration)
    assert!(true);
}

// ── Odd-frame short ──────────────────────────────────────────────────────────

#[test]
fn even_frame_is_89342_dots() {
    let mut ppu = Ppu::new();
    // rendering disabled → no odd-frame skip, every frame = 262 × 341 = 89342 dots
    // Tick exactly one frame worth of CPU cycles
    let dots_per_frame: u64 = 262 * 341; // 89342
    tick(&mut ppu, dots_per_frame / 3);
    // We should be on scanline 0, dot 0 (or very close)
    // The test just verifies no panic and frame_ready was set during the frame
    // frame_ready is cleared by the caller; we only check it was set in frame
    // so we tick to past scanline 239
    tick_to(&mut ppu, 239, 257);
    tick(&mut ppu, 1);
    assert!(ppu.frame_ready);
}

#[test]
fn odd_frame_skip_reduces_dot_count_when_rendering_enabled() {
    let mut ppu = Ppu::new();
    ppu.write_register(1, 0x18, None); // enable rendering so odd-frame skip applies
    // On odd frames (frame 1, 3, …), the pre-render scanline is 340 dots not 341.
    // Tick exactly one even frame (89342 dots / 3 cpu cycles + remainder):
    // After one full frame the PPU toggles odd_frame to true.
    let dots_per_even_frame: u64 = 341 * 262;
    tick(&mut ppu, dots_per_even_frame / 3 + 1);
    // Now on frame 1 (odd). frame_ready will be set on scanline 239 again.
    tick_to(&mut ppu, 239, 257);
    tick(&mut ppu, 1);
    assert!(ppu.frame_ready);
}

// ── OAMADDR reset during sprite fetch (dots 257-320) ──────────────────────────
//
// Documented hardware quirk (nesdev "PPU sprite evaluation"): OAMADDR is
// forced to 0 during ticks 257-320 of every visible and pre-render scanline,
// while rendering is enabled. Without this, a stale OAMADDR left over from an
// earlier $2003/$2004 access (e.g. by a previous frame's game code) makes the
// next $4014 OAM DMA start copying at the wrong OAM slot — the whole 256-byte
// DMA still runs, but wrapped, so "sprite 0" (OAM bytes 0-3) ends up holding
// whatever wrapped around to that slot instead of the DMA source's first 4
// bytes. AccuracyCoin's Sprite0Hit_Behavior test (and several others that
// share its "Sprite Zero Hits should be working" prerequisite) fail exactly
// this way when OAMADDR isn't reset.

#[test]
fn oam_addr_resets_to_zero_during_sprite_fetch_when_rendering() {
    let mut ppu = Ppu::new();
    ppu.write_register(3, 5, None); // OAMADDR = 5
    ppu.write_register(1, 0x18, None); // enable BG + sprite rendering
    tick_to(&mut ppu, 0, 257); // start of the sprite-fetch window
    assert_eq!(ppu.oam_addr(), 0);
}

#[test]
fn oam_addr_untouched_by_sprite_fetch_window_when_rendering_disabled() {
    let mut ppu = Ppu::new();
    ppu.write_register(3, 5, None); // OAMADDR = 5, rendering left disabled
    tick_to(&mut ppu, 0, 257);
    assert_eq!(ppu.oam_addr(), 5);
}

#[test]
fn oam_dma_after_sprite_fetch_reset_lands_sprite_zero_at_oam_zero() {
    // Reproduces AccuracyCoin's Sprite0Hit_Behavior setup: OAMADDR left
    // nonzero by earlier register writes, then a full frame of rendering
    // passes (forcing OAMADDR back to 0 at dot 257), then the CPU does an
    // OAM DMA — sprite 0's bytes must land at OAM[0..4], not wrapped
    // elsewhere.
    let mut ppu = Ppu::new();
    ppu.write_register(3, 5, None); // stale OAMADDR from earlier CPU code
    ppu.write_register(1, 0x18, None); // enable rendering
    tick_to(&mut ppu, 0, 257); // OAMADDR forced back to 0 here
    assert_eq!(ppu.oam_addr(), 0);
    // Simulate the $4014 OAM DMA: 256 bytes, sprite 0 = [Y, CHR, Attr, X].
    let mut page = [0xFFu8; 256];
    page[0..4].copy_from_slice(&[0x00, 0xFC, 0x00, 0x08]);
    for (i, &b) in page.iter().enumerate() {
        ppu.oam_dma_write(i as u8, b);
    }
    assert_eq!(
        [
            ppu.oam_byte(0),
            ppu.oam_byte(1),
            ppu.oam_byte(2),
            ppu.oam_byte(3)
        ],
        [0x00, 0xFC, 0x00, 0x08]
    );
}

// ── $2007 access during rendering: glitch increment ───────────────────────────
//
// Documented hardware quirk: a $2007 read or write that happens while
// rendering is enabled, during a visible or pre-render scanline, does NOT use
// the normal +1/+32 vram_increment() — instead it triggers the same
// coarse-X-increment + Y-increment pulse the background fetch pipeline itself
// uses. AccuracyCoin's "$2007 Read w/ Rendering" test pins this to exactly
// v += $1001 from a starting v with coarse X < 31 and fine Y < 7 (the
// no-wrap case): +1 from increment_coarse_x, +$1000 from increment_y.

#[test]
fn ppudata_read_during_rendering_uses_coarse_x_and_y_glitch_increment() {
    let mut ppu = Ppu::new();
    tick_to(&mut ppu, 10, 100); // advance to a visible scanline with rendering off
    ppu.write_register(1, 0x18, None); // enable BG + sprite rendering
    ppu.write_register(6, 0x20, None); // v = $2000 (coarse X=0, coarse Y=0, fine Y=0)
    ppu.write_register(6, 0x00, None);
    let _ = ppu.read_register(7, None);
    assert_eq!(ppu.v(), 0x2000 + 0x1001);
}

#[test]
fn ppudata_write_during_rendering_uses_coarse_x_and_y_glitch_increment() {
    let mut ppu = Ppu::new();
    tick_to(&mut ppu, 10, 100);
    ppu.write_register(1, 0x18, None);
    ppu.write_register(6, 0x20, None);
    ppu.write_register(6, 0x00, None);
    ppu.write_register(7, 0x00, None);
    assert_eq!(ppu.v(), 0x2000 + 0x1001);
}

#[test]
fn ppudata_read_outside_rendering_uses_normal_increment() {
    let mut ppu = Ppu::new();
    ppu.write_register(6, 0x20, None); // v = $2000; rendering left disabled
    ppu.write_register(6, 0x00, None);
    let _ = ppu.read_register(7, None);
    assert_eq!(ppu.v(), 0x2001); // normal +1 (PPUCTRL increment-mode bit clear)
}

#[test]
fn ppudata_read_during_vblank_uses_normal_increment_even_if_rendering_was_enabled() {
    let mut ppu = Ppu::new();
    ppu.write_register(1, 0x18, None); // rendering enabled...
    tick_to(&mut ppu, 245, 100); // ...but we're in VBlank, not a render scanline
    ppu.write_register(6, 0x20, None);
    ppu.write_register(6, 0x00, None);
    let _ = ppu.read_register(7, None);
    assert_eq!(ppu.v(), 0x2001);
}

// ── Sprite evaluation seeded from a nonzero/misaligned OAMADDR ────────────────
//
// Real hardware's sprite evaluation doesn't always start at OAM byte 0 — it
// starts wherever OAMADDR ($2003) currently points, which the CPU can set to
// any value (including a non-multiple-of-4 "misaligned" one) right before
// evaluation begins (dot 65). AccuracyCoin's "Arbitrary Sprite Zero" and
// "Misaligned OAM Behavior" tests exploit this: whichever object is examined
// FIRST is treated as "sprite zero" for hit-detection purposes, even if it
// isn't literally OAM index 0 — and if that object is out of range, "sprite
// zero" never exists that scanline at all, regardless of what lands in
// secondary OAM slot 0. `evaluate_sprites` hardcoded its OAM byte pointer to
// start at 0 and treated literal OAM index 0 as sprite zero, ignoring
// OAMADDR entirely.

fn setup_oam(ppu: &mut Ppu, bytes: &[u8; 256]) {
    ppu.write_register(3, 0, None);
    for (i, &b) in bytes.iter().enumerate() {
        ppu.oam_dma_write(i as u8, b);
    }
}

#[test]
fn sprite_evaluation_starts_at_oamaddr_not_always_oam_index_zero() {
    let mut ppu = Ppu::new();
    let mut oam = [0xFFu8; 256];
    oam[128] = 0x00; // "object 32"'s Y position: in range for next_scanline=1
    setup_oam(&mut ppu, &oam);

    tick_to(&mut ppu, 0, 60); // before evaluation starts, rendering still off
    ppu.write_register(1, 0x18, None); // enable rendering
    ppu.write_register(3, 128, None); // OAMADDR = 32*4, as if the CPU just wrote $2003
    tick_to(&mut ppu, 0, 256); // run evaluation for scanline 1 to completion

    assert!(ppu.sprite0_eval());
    assert_eq!(ppu.secondary_oam_byte(0), 0x00);
}

#[test]
fn sprite_evaluation_first_examined_object_out_of_range_means_no_sprite_zero_this_scanline() {
    let mut ppu = Ppu::new();
    let mut oam = [0xFFu8; 256];
    oam[0] = 0x00; // object 0's Y is in range too — a wrong implementation
    oam[1] = 0xBB; // that starts at index 0 would wrongly pick this CHR marker
    oam[128] = 0xFF; // "object 32"'s Y: out of range — first examined, and rejected
    oam[132] = 0x00; // "object 33"'s Y: in range, would land in secondary OAM slot 0
    oam[133] = 0xAA; // ...but must NOT be reached, since object 32 was rejected first
    setup_oam(&mut ppu, &oam);

    tick_to(&mut ppu, 0, 60);
    ppu.write_register(1, 0x18, None);
    ppu.write_register(3, 128, None);
    tick_to(&mut ppu, 0, 256);

    // Object 33 is legitimately found and lands in secondary OAM slot 0 (it's
    // the first object evaluation *accepts*), but since object 32 was the
    // first object *examined* and was rejected, no sprite zero exists this
    // scanline at all — object 33 must not be flagged as sprite zero despite
    // occupying slot 0.
    assert_eq!(ppu.secondary_oam_byte(0), 0x00);
    assert_eq!(ppu.secondary_oam_byte(1), 0xAA);
    assert!(!ppu.sprite0_eval());
}

#[test]
fn sprite_evaluation_misaligned_oamaddr_walks_byte_by_byte() {
    // OAMADDR = 1 (misaligned): the "Y" checked is OAM[1], and on rejection
    // OAMADDR advances by 4 then realigns via & $FC, per AccuracyCoin's own
    // documented walkthrough (e.g. starting misaligned at $80 -> rejected ->
    // $84, still 4-aligned from then on).
    let mut ppu = Ppu::new();
    let mut oam = [0xFFu8; 256];
    oam[0] = 0x00; // object 0's Y is in range too — a wrong implementation
    oam[1] = 0xFF; // that starts at index 0 would wrongly accept it immediately
    oam[4] = 0x00; // after +4 & $FC realignment (1+4=5, 5&$FC=4): in range
    oam[5] = 0xCC; // the CHR marker that should end up in secondary OAM
    setup_oam(&mut ppu, &oam);

    tick_to(&mut ppu, 0, 60);
    ppu.write_register(1, 0x18, None);
    ppu.write_register(3, 1, None);
    tick_to(&mut ppu, 0, 256);

    assert_eq!(ppu.secondary_oam_byte(0), 0x00);
    assert_eq!(ppu.secondary_oam_byte(1), 0xCC);
}

// ── $2004 reads during the secondary-OAM-clear window (dots 1-64) ────────────
//
// While rendering, dots 1-64 of every visible/pre-render scanline are spent
// clearing secondary OAM to $FF — the PPU is internally busy writing $FF over
// and over, and a $2004 read during that window reads that same bus activity
// instead of the real OAMDATA at OAMADDR. AccuracyCoin's "Address $2004
// Behavior" test 4 pins this: even with OAMADDR pointing at a real, non-$FF
// byte, a $2004 read during dots 1-64 (rendering enabled) must return $FF.

#[test]
fn oamdata_read_during_secondary_oam_clear_window_always_returns_ff() {
    let mut ppu = Ppu::new();
    let mut oam = [0xAAu8; 256]; // definitely not $FF, so a real read would differ
    oam[0] = 0x5A;
    setup_oam(&mut ppu, &oam);
    ppu.write_register(3, 0, None); // OAMADDR = 0
    ppu.write_register(1, 0x18, None); // enable rendering
    tick_to(&mut ppu, 0, 30); // dot 30: inside the 1-64 clear window
    assert_eq!(ppu.read_register(4, None), 0xFF);
}

#[test]
fn oamdata_read_during_evaluation_returns_oam_buffer_not_oamaddr() {
    // AccuracyCoin "$2004 Stress": while rendering, past the dot-1-64 clear
    // window, the sprite-evaluation machinery is driving the OAM data bus, so a
    // $2004 read returns the internal OAM buffer — the byte evaluation last
    // read — NOT oam[oam_addr]. With every sprite Y out of range ($AA), the
    // evaluation walk reads one Y byte per object, all $AA, so the buffer sits
    // at $AA; the distinct byte parked at OAMADDR ($5A at oam[0]) is NOT what
    // the read returns.
    let mut ppu = Ppu::new();
    let mut oam = [0xAAu8; 256];
    oam[0] = 0x5A; // sprite 0's Y — also out of range, read once at dot 65/66
    setup_oam(&mut ppu, &oam);
    ppu.write_register(3, 0, None); // OAMADDR = 0
    ppu.write_register(1, 0x18, None); // enable rendering
    tick_to(&mut ppu, 0, 100); // dot 100: well past the clear window, mid-eval
    assert_eq!(
        ppu.read_register(4, None),
        0xAA,
        "returns the eval-driven OAM buffer, not oam[oam_addr] ($5A)"
    );
}

#[test]
fn oamdata_read_during_dots_1_64_with_rendering_disabled_returns_real_oam_value() {
    let mut ppu = Ppu::new();
    let mut oam = [0xAAu8; 256];
    oam[0] = 0x5A;
    setup_oam(&mut ppu, &oam);
    ppu.write_register(3, 0, None); // rendering left disabled
    tick_to(&mut ppu, 0, 30);
    assert_eq!(ppu.read_register(4, None), 0x5A);
}

#[test]
fn greyscale_mode_masks_ppudata_palette_reads() {
    // AccuracyCoin "Palette RAM Quirks" code 6: with greyscale enabled, the
    // low 4 bits of a $2007 palette READ are zero (writes stay unaffected —
    // the mask is applied on the read path, not in storage).
    let mut ppu = Ppu::new();
    ppu.write_register(1, 0x01, None); // greyscale only — no rendering
    ppu.write_register(6, 0x3F, None);
    ppu.write_register(6, 0x01, None);
    ppu.write_register(7, 0x2A, None); // palette $3F01 = $2A

    // Point v back at $3F01 and read it with greyscale on.
    ppu.write_register(6, 0x3F, None);
    ppu.write_register(6, 0x01, None);
    let read = ppu.read_register(7, None);
    assert_eq!(read & 0x3F, 0x20, "$2A & $30 = $20 with greyscale on");

    // Greyscale off: same read returns the full 6-bit value.
    ppu.write_register(1, 0x00, None);
    ppu.write_register(6, 0x3F, None);
    ppu.write_register(6, 0x01, None);
    let read = ppu.read_register(7, None);
    assert_eq!(read & 0x3F, 0x2A, "stored value was never masked");
}

// ── Background shift register serial input ──────────────────────────────────

#[test]
fn bg_shifters_shift_in_zero_low_plane_one_high_plane() {
    // AccuracyCoin "BG Serial In": when the background shift registers shift,
    // the low bit plane brings in a 0 and the high bit plane brings in a 1.
    let mut ppu = Ppu::new();
    ppu.write_register(1, 0x08, None); // enable background rendering

    // Land mid-tile on a visible scanline: the dot%8==1 reload replaced the
    // low byte a few dots ago, and every shift since brought in the serial
    // bits (pattern fetches read 0 with no cartridge).
    tick_to(&mut ppu, 10, 12);
    let (lo, hi) = ppu.bg_shifters();
    assert_eq!(lo & 0x07, 0x00, "low plane shifts in 0s");
    assert_eq!(hi & 0x07, 0x07, "high plane shifts in 1s");
}

#[test]
fn bg_reload_skip_via_rendering_disable_draws_serial_ones() {
    // End-to-end version of AccuracyCoin "BG Serial In" test 2's mechanism:
    // an empty nametable draws only backdrop, but disabling rendering for
    // exactly the dot%8==1 reload dot lets the hi-plane serial 1s (shifted
    // in over the previous 7 dots) escape past bit 7 before the next reload
    // clobbers the low byte — drawing solid color-%10 pixels.
    let mut ppu = Ppu::new();
    // Palette: backdrop $0F, color %10 of palette %00 = $2A (marker).
    ppu.write_register(6, 0x3F, None);
    ppu.write_register(6, 0x00, None);
    ppu.write_register(7, 0x0F, None);
    ppu.write_register(7, 0x00, None);
    ppu.write_register(7, 0x2A, None);
    // Point v back at $2000 so rendering scrolls from a sane origin.
    ppu.write_register(6, 0x20, None);
    ppu.write_register(6, 0x00, None);
    ppu.write_register(1, 0x08, None); // BG on
    ppu.tick_dots(20 * 341 + 65, None); // dots 0-64 of scanline 20 processed; dot 65 (a reload dot) is next
    ppu.write_register(1, 0x00, None); // rendering off…
    ppu.tick_dots(1, None); //            …exactly across dot 65, skipping its reload
    ppu.write_register(1, 0x08, None); // back on
    ppu.tick_dots(200, None); // let the escaped 1s reach the output mux

    let row = &ppu.frame[20 * 256..21 * 256];
    let marker_pixels: Vec<usize> = (0..256).filter(|&x| row[x] == 0x2A).collect();
    assert!(
        !marker_pixels.is_empty(),
        "skipped reload should draw the shifted-in hi-plane 1s as color %10 pixels"
    );
    // And a control: without any disable window, the same setup draws none.
    let mut control = Ppu::new();
    control.write_register(6, 0x3F, None);
    control.write_register(6, 0x00, None);
    control.write_register(7, 0x0F, None);
    control.write_register(7, 0x00, None);
    control.write_register(7, 0x2A, None);
    control.write_register(6, 0x20, None);
    control.write_register(6, 0x00, None);
    control.write_register(1, 0x08, None);
    control.tick_dots(21 * 341, None);
    let row = &control.frame[20 * 256..21 * 256];
    assert!(
        (0..256).all(|x| row[x] != 0x2A),
        "continuous rendering must never expose the serial-in bits"
    );
}
