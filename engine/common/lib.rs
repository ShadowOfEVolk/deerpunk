// Common components shared between client and server

pub mod cmd;
pub mod cvar;
pub mod fs;

pub use cmd::CmdSystem;
pub use cvar::CvarSystem;
pub use fs::{FileSystem, FileStream};


/// Macro to create a singleton system
/// Usage: singleton_system!(function_name, SystemType);
/// Generates: static SYSTEM_NAME and pub fn function_name()
#[macro_export]
macro_rules! singleton_system {
    ($fn_name:ident, $system_type:ty) => {
        use once_cell::sync::Lazy;
        use std::sync::Mutex;

        paste::paste! {
            static [<$fn_name:upper>]: Lazy<Mutex<$system_type>> = Lazy::new(|| Mutex::new(<$system_type>::new()));

            pub fn $fn_name() -> std::sync::MutexGuard<'static, $system_type> {
                [<$fn_name:upper>].lock().unwrap()
            }
        }
    };
}