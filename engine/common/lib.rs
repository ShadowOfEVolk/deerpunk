// Common components shared between client and server

pub mod cmd;
pub mod components;
pub mod cvar;
pub mod fs;
pub mod network;

pub mod coord;
pub mod events;
pub mod frame;
pub mod frame_buffer;
pub mod physics;
pub mod resource;
pub mod scheduler;
pub mod transform;

pub use cmd::CmdSystem;
pub use components::{
    NetReplicatedComponent, SimpleCircleRenderer, SimpleEntity, Terrain, TransformComponent,
};
pub use cvar::CvarSystem;
pub use fs::{FileStream, FileSystem};
pub use network::replication::{NetworkComponent, NetworkField};

pub use transform::Transform;

// Built-in cvars available to all binaries that depend on engine-common.
inventory::submit! {
    cvar::CvarDef { name: "r_texturehandle", default: "", desc: "Shared memory texture handle for editor frame output" }
}
inventory::submit! {
    cvar::CvarDef { name: "gamedir", default: "examples/deathmatch/gamedir", desc: "Game directory path" }
}

/// Macro to create a singleton system
/// Usage: singleton_system!(function_name, SystemType);
/// Generates: static SYSTEM_NAME and pub fn function_name()
#[macro_export]
macro_rules! singleton_system {
    ($fn_name:ident, $system_type:ty) => {
        paste::paste! {
            static [<$fn_name:upper>]: once_cell::sync::Lazy<std::sync::Mutex<$system_type>> =
                once_cell::sync::Lazy::new(|| std::sync::Mutex::new(<$system_type>::new()));

            pub fn $fn_name() -> std::sync::MutexGuard<'static, $system_type> {
                [<$fn_name:upper>].lock().unwrap()
            }
        }
    };
}
