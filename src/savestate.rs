//! Save states: a full snapshot of the running machine (CPU, PPU, APU, bus,
//! and the cartridge's *mutable* state) serialized to a compact binary blob
//! via `bincode`.
//!
//! What is NOT saved: the immutable ROM (PRG-ROM / CHR-ROM) and the PPU
//! framebuffer. A save state is therefore tied to the ROM currently loaded —
//! restoring re-uses the live cartridge (keeping its ROM and reconstructed
//! mapper object) and only overwrites its mutable parts (PRG-RAM, CHR-RAM,
//! and the mapper's banking/IRQ registers). The framebuffer is regenerated on
//! the next rendered frame.
//!
//! The bulk of the state is `#[derive(Serialize, Deserialize)]` on the machine
//! structs themselves; the two awkward pieces are handled locally here:
//!   * fixed byte arrays larger than 32 (serde's derive only covers `[T; 0..=32]`)
//!     — see [`byte_arr`] / [`boxed_byte_arr`];
//!   * the `Box<dyn Mapper>` trait object — serialized through the object-safe
//!     `Mapper::state_save`/`state_load` methods into an opaque `Vec<u8>`.

use crate::bus::Bus;
use crate::cpu::Cpu;
use crate::system::SystemClock;

/// Magic bytes identifying a save-state blob ("NESS").
const MAGIC: u32 = 0x4E45_5353;
/// Bump whenever the serialized layout changes incompatibly.
const VERSION: u32 = 1;

/// The mutable, ROM-independent parts of a `Cartridge`. The ROM itself is
/// reconstructed from the already-loaded cartridge on restore.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct CartridgeState {
    pub(crate) prg_ram: Vec<u8>,
    pub(crate) chr_ram: Vec<u8>,
    /// Opaque mapper register state (`Mapper::state_save`).
    pub(crate) mapper: Vec<u8>,
}

/// Serialize the current machine state to a byte blob.
///
/// Errors if no ROM is loaded (nothing to snapshot).
pub fn save(cpu: &Cpu, bus: &Bus, clock: &SystemClock) -> Result<Vec<u8>, String> {
    let cart = bus
        .cartridge_ref()
        .ok_or_else(|| "no ROM loaded".to_string())?
        .capture_state();
    // A tuple of references — `bus` serializes with its `cartridge` field
    // skipped, so no clone of the (non-`Clone`) cartridge is needed.
    let payload = (MAGIC, VERSION, cpu, bus, &cart, clock);
    bincode::serialize(&payload).map_err(|e| e.to_string())
}

/// Restore a machine state previously produced by [`save`], overwriting
/// `cpu`/`bus`/`clock` in place. The currently-loaded cartridge is reused for
/// its ROM and mapper object; only its mutable state is replaced. Errors (and
/// leaves the machine untouched) if the blob is invalid or no ROM is loaded.
pub fn load(
    data: &[u8],
    cpu: &mut Cpu,
    bus: &mut Bus,
    clock: &mut SystemClock,
) -> Result<(), String> {
    let (magic, version, new_cpu, mut new_bus, cart, new_clock): (
        u32,
        u32,
        Cpu,
        Bus,
        CartridgeState,
        SystemClock,
    ) = bincode::deserialize(data).map_err(|e| e.to_string())?;
    if magic != MAGIC {
        return Err("not a valid save state".to_string());
    }
    if version != VERSION {
        return Err(format!("unsupported save-state version {version}"));
    }
    // Move the live cartridge (with its ROM + reconstructed mapper) into the
    // restored bus, then apply the saved mutable cartridge state to it.
    let mut cartridge = bus
        .take_cartridge()
        .ok_or_else(|| "no ROM loaded".to_string())?;
    cartridge.restore_state(&cart);
    new_bus.insert_cartridge(cartridge);
    *cpu = new_cpu;
    *bus = new_bus;
    *clock = new_clock;
    Ok(())
}

/// serde `with` adaptor for `[u8; N]` (N > 32, which serde's derive doesn't
/// cover). Serialized as a fixed-length tuple of bytes.
pub(crate) mod byte_arr {
    use serde::de::{Error, SeqAccess, Visitor};
    use serde::ser::SerializeTuple;
    use serde::{Deserializer, Serializer};
    use std::fmt;

    pub fn serialize<S: Serializer, const N: usize>(
        arr: &[u8; N],
        s: S,
    ) -> Result<S::Ok, S::Error> {
        let mut t = s.serialize_tuple(N)?;
        for b in arr {
            t.serialize_element(b)?;
        }
        t.end()
    }

    struct ArrVisitor<const N: usize>;

    impl<'de, const N: usize> Visitor<'de> for ArrVisitor<N> {
        type Value = [u8; N];

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "an array of {N} bytes")
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<[u8; N], A::Error> {
            let mut out = [0u8; N];
            for (i, slot) in out.iter_mut().enumerate() {
                *slot = seq
                    .next_element()?
                    .ok_or_else(|| Error::invalid_length(i, &self))?;
            }
            Ok(out)
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>, const N: usize>(
        d: D,
    ) -> Result<[u8; N], D::Error> {
        d.deserialize_tuple(N, ArrVisitor::<N>)
    }
}

/// serde `with` adaptor for `Box<[u8; N]>` (see [`byte_arr`]).
pub(crate) mod boxed_byte_arr {
    use serde::{Deserializer, Serializer};

    // serde's derive passes `&self.field` (i.e. `&Box<[u8; N]>`), so match that
    // type exactly rather than relying on deref coercion through the generated
    // code.
    #[allow(clippy::borrowed_box)]
    pub fn serialize<S: Serializer, const N: usize>(
        arr: &Box<[u8; N]>,
        s: S,
    ) -> Result<S::Ok, S::Error> {
        super::byte_arr::serialize(&**arr, s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>, const N: usize>(
        d: D,
    ) -> Result<Box<[u8; N]>, D::Error> {
        Ok(Box::new(super::byte_arr::deserialize(d)?))
    }
}
