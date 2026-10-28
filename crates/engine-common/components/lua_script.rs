//! Prefab-declared Luau module path for server-side entity scripts.

/// Max UTF-8 bytes for module path (NUL-padded).
pub const LUA_MODULE_MAX: usize = 256;

/// Server-only hint: which Luau module to attach after spawn.
/// Not replicated to clients.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct LuaScript {
    pub module: [u8; LUA_MODULE_MAX],
}

impl LuaScript {
    pub fn new(module: &str) -> Self {
        let mut buf = [0u8; LUA_MODULE_MAX];
        let bytes = module.as_bytes();
        let n = bytes.len().min(LUA_MODULE_MAX - 1);
        buf[..n].copy_from_slice(&bytes[..n]);
        Self { module: buf }
    }

    pub fn module_str(&self) -> &str {
        let end = self.module.iter().position(|&b| b == 0).unwrap_or(0);
        std::str::from_utf8(&self.module[..end]).unwrap_or("")
    }
}
