//! Static cvar definitions collected via [`inventory`] (Xash / GoldSrc style).

/// Saved to `configs/config.cfg` (GoldSrc `FCVAR_ARCHIVE`).
pub const ARCHIVE: u32 = 1 << 0;
/// Saved to `configs/video.cfg` (Xash `FCVAR_RENDERINFO`).
pub const RENDERINFO: u32 = 1 << 1;

pub struct CvarDef {
    pub name: &'static str,
    pub default: &'static str,
    pub desc: &'static str,
    pub flags: u32,
}

inventory::collect!(CvarDef);

/// Register a built-in cvar (link-time via `inventory`).
///
/// ```ignore
/// deerpunk_engine_common::cvar_def!("cl_foo", "0", "help text");
/// deerpunk_engine_common::cvar_def!("r_bar", "1", "help", deerpunk_engine_common::cvar_def::RENDERINFO);
/// ```
#[macro_export]
macro_rules! cvar_def {
    ($name:expr, $default:expr, $desc:expr) => {
        inventory::submit! {
            $crate::cvar_def::CvarDef {
                name: $name,
                default: $default,
                desc: $desc,
                flags: 0,
            }
        }
    };
    ($name:expr, $default:expr, $desc:expr, $flags:expr) => {
        inventory::submit! {
            $crate::cvar_def::CvarDef {
                name: $name,
                default: $default,
                desc: $desc,
                flags: $flags,
            }
        }
    };
}
