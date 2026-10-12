//! `v1:world:spawn_prefab` options (FFI-safe, no CBOR).

/// One map keyvalue pair for [`SpawnPrefabOpts`].
#[repr(C)]
pub struct SpawnPrefabKvPair {
    pub key_ptr: *const u8,
    pub key_len: usize,
    pub val_ptr: *const u8,
    pub val_len: usize,
}

/// Optional map-entity fields applied after prefab components (origin, angles, MapKv).
#[repr(C)]
pub struct SpawnPrefabOpts {
    pub origin: [f32; 3],
    pub has_origin: u8,
    /// Pitch, yaw, roll in degrees (GoldSrc `angles`).
    pub angles: [f32; 3],
    pub has_angles: u8,
    pub kv: *const SpawnPrefabKvPair,
    pub kv_count: u32,
}

impl SpawnPrefabOpts {
    pub const fn empty() -> Self {
        Self {
            origin: [0.0, 0.0, 0.0],
            has_origin: 0,
            angles: [0.0, 0.0, 0.0],
            has_angles: 0,
            kv: std::ptr::null(),
            kv_count: 0,
        }
    }
}
