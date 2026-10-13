/// Method resolver passed from host to gamelib (`v1:…` names → function pointers).
pub type ResolverFn = extern "C" fn(*const u8, usize) -> *const ();

/// Shared layout for `server_init` / `client_init`.
pub const GAMELIB_API_VERSION: u32 = 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GamelibInitData {
    pub version: u32,
    pub ctx: *mut u8,
    pub resolver: ResolverFn,
}

pub type ServerInitData = GamelibInitData;
pub type ClientInitData = GamelibInitData;
