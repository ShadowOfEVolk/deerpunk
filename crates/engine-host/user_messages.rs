//! UserMessage registry (GoldSrc / Xash): names at register, `u8 msg_id` on wire.

use std::collections::HashMap;

use deerpunk_engine_common::network::user_message::UserMessageRpc;

/// `extern "C" fn(name_ptr, name_len, data_ptr, data_len)`
pub type UmHookFn = extern "C" fn(*const u8, usize, *const u8, usize);

#[derive(Clone, Debug)]
struct UmEntry {
    id: u16,
    size: i16,
}

pub struct UserMessageRegistry {
    by_name: HashMap<String, UmEntry>,
    by_id: HashMap<u16, String>,
    next_id: u16,
    hooks: HashMap<String, UmHookFn>,
}

impl Default for UserMessageRegistry {
    fn default() -> Self {
        Self {
            by_name: HashMap::new(),
            by_id: HashMap::new(),
            next_id: 1,
            hooks: HashMap::new(),
        }
    }
}

impl UserMessageRegistry {
    pub fn register(&mut self, name: &str, size: i16) -> Result<u16, String> {
        let name = name.trim();
        if name.is_empty() || name.len() > 255 {
            return Err("bad user message name".into());
        }
        if let Some(e) = self.by_name.get(name) {
            return Ok(e.id);
        }
        if self.next_id > u8::MAX as u16 {
            return Err("user message id exhausted".into());
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.by_name
            .insert(name.to_string(), UmEntry { id, size });
        self.by_id.insert(id, name.to_string());
        log::info!("[um] register '{name}' id={id} size={size}");
        Ok(id)
    }

    pub fn id_of(&self, name: &str) -> Option<u16> {
        self.by_name.get(name.trim()).map(|e| e.id)
    }

    pub fn name_of(&self, id: u16) -> Option<&str> {
        self.by_id.get(&id).map(|s| s.as_str())
    }

    pub fn is_registered(&self, name: &str) -> bool {
        self.by_name.contains_key(name.trim())
    }

    pub fn hook(&mut self, name: &str, cb: UmHookFn) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("empty hook name".into());
        }
        if !self.by_name.contains_key(name) {
            let _ = self.register(name, -1)?;
        }
        self.hooks.insert(name.to_string(), cb);
        log::info!("[um] hook '{name}'");
        Ok(())
    }

    pub fn invoke_hooks(&self, rpc: &UserMessageRpc) {
        let Some(name) = self.name_of(rpc.msg_id) else {
            log::warn!("[um] hook: unknown msg_id={}", rpc.msg_id);
            return;
        };
        let Some(cb) = self.hooks.get(name) else {
            return;
        };
        let nb = name.as_bytes();
        cb(nb.as_ptr(), nb.len(), rpc.data.as_ptr(), rpc.data.len());
    }
}
