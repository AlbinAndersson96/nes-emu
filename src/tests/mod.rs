//! Test tree, split into two groups:
//! - `unit`: white-box tests that reach into crate internals (CPU micro-op
//!   state, PPU registers, bus wiring, mapper banking). These use the
//!   flat-memory [`unit::TestBus`] fixture or drive components directly.
//! - `integration`: whole-ROM harnesses that boot a `.nes` image through the
//!   real `Bus`/`Cpu`/`SystemClock` and assert its self-reported verdict
//!   (blargg `$6000` protocol, on-screen result codes, AccuracyCoin).

mod integration;
mod unit;
