//! Identifies the prefab an entity was spawned from.
//!
//! Stamped on every prefab spawn in both worlds (client-only, server-only and
//! networked alike) so the inspector can resolve entity classnames without
//! relying on the networked `NetworkEntity`.

/// 32-bit FNV-1a hash.
pub fn fnv1a32(bytes: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for &b in bytes {
        hash ^= b as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Deterministic id for a prefab classname — both worlds agree without shared state.
pub fn prefab_id_for(classname: &str) -> PrefabId {
    PrefabId {
        id: fnv1a32(classname.as_bytes()),
    }
}

/// Stable id of the prefab this entity was spawned from.
#[derive(Clone, Copy, Debug)]
pub struct PrefabId {
    pub id: u32,
}
