//! Entity display name (inspector / studio). Mirrors the native flecs entity name.

/// Max UTF-8 bytes for the display name (NUL-padded).
pub const ENTITY_NAME_MAX: usize = 256;

/// Display name override for an entity. Inspector row name resolution:
/// [`EntityName`] → native flecs name (`ecs_get_name`) → `0x{id:x}`.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct EntityName {
    pub name: [u8; ENTITY_NAME_MAX],
}

impl EntityName {
    pub fn new(name: &str) -> Self {
        let mut buf = [0u8; ENTITY_NAME_MAX];
        let bytes = name.as_bytes();
        let n = bytes.len().min(ENTITY_NAME_MAX - 1);
        buf[..n].copy_from_slice(&bytes[..n]);
        Self { name: buf }
    }

    pub fn as_str(&self) -> &str {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(0);
        std::str::from_utf8(&self.name[..end]).unwrap_or("")
    }
}
