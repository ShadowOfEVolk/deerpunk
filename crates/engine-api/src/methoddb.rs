//! [`EngineApiMethod::flags`] for MethodDB / inspector.

/// Callable from the **server** gamelib DLL (`darkdemo_amd64`, dedicated host, listen-server server side).
pub const SCOPE_SERVER: u32 = 1 << 0;
/// Callable from the **client** gamelib DLL (`client_amd64`, listen-server client side).
pub const SCOPE_CLIENT: u32 = 1 << 1;

pub const SCOPE_BOTH: u32 = SCOPE_SERVER | SCOPE_CLIENT;
