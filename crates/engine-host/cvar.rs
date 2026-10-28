pub use deerpunk_engine_common::cvar::{CvarSystem, ARCHIVE, RENDERINFO};
pub use deerpunk_engine_common::cvar_def::{self, CvarDef};

use deerpunk_engine_macro as methoddb;

#[methoddb::namespace(
    name = "v1:cvar",
    on_host,
    scope = ["server", "client"],
)]
impl crate::Host {
    #[methoddb::bind(name = "set")]
    pub fn api_cvar_set(&self, name: &str, value: &str) -> i32 {
        if name.is_empty() {
            return -1;
        }
        self.cvar.set_or_create(name, value);
        0
    }

    #[methoddb::bind(name = "get_f32")]
    pub fn api_cvar_get_f32(&self, name: &str, out: &mut f32) -> i32 {
        if name.is_empty() {
            return -1;
        }
        match self.cvar.get_f32(name) {
            Some(v) => {
                *out = v;
                0
            }
            None => -1,
        }
    }
}
