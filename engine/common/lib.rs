// Common components shared between client and server

pub mod cmd;
pub mod cvar;
pub mod fs;
pub mod components;
pub mod network;

pub use cmd::CmdSystem;
pub use cvar::CvarSystem;
pub use fs::{FileSystem, FileStream};
pub use components::{SimpleEntity, SimpleCircleRenderer, TransformComponent, NetReplicatedComponent};
pub use network::replication::{NetworkComponent, NetworkField};


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