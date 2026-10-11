use crate::Resolver;

fn resolve(r: Resolver, name: &str) -> *const () {
    let bytes = name.as_bytes();
    let ptr = r(bytes.as_ptr(), bytes.len());
    if ptr.is_null() {
        panic!("[engine-api] unresolved method: {}", name);
    }
    ptr
}

type ClientCmdFn = extern "C" fn(*mut u8, *const u8, usize) -> i32;
type SetEnabledFn = extern "C" fn(*mut u8, i32) -> i32;
type IsEnabledFn = extern "C" fn(*mut u8) -> i32;
type SetPlayerEntityFn = extern "C" fn(*mut u8, u32, u64) -> i32;
type SendUsercmdFn = extern "C" fn(*mut u8, *const u8, usize) -> i32;
type GetUsercmdFn = extern "C" fn(*mut u8, u32, *mut u8, usize) -> i32;
type MoveSnapshotFn = extern "C" fn(*mut u8, *mut f32, usize, *mut u64) -> i32;
type ApplyLocalViewFn = extern "C" fn(*mut u8, f32, f32) -> i32;
type LocalEntityFn = extern "C" fn(*mut u8) -> u64;

pub struct NetApi {
    ctx: *mut u8,
    client_cmd_fn: ClientCmdFn,
    set_enabled_fn: SetEnabledFn,
    is_enabled_fn: IsEnabledFn,
    set_player_entity_fn: SetPlayerEntityFn,
    send_usercmd_fn: SendUsercmdFn,
    get_usercmd_fn: GetUsercmdFn,
    move_snapshot_fn: MoveSnapshotFn,
    apply_local_view_fn: ApplyLocalViewFn,
    local_entity_fn: LocalEntityFn,
}

unsafe impl Send for NetApi {}
unsafe impl Sync for NetApi {}

impl NetApi {
    pub fn new(ctx: *mut u8, r: Resolver) -> Self {
        Self {
            ctx,
            client_cmd_fn: unsafe { std::mem::transmute(resolve(r, "v1:net:client_cmd")) },
            set_enabled_fn: unsafe { std::mem::transmute(resolve(r, "v1:server:set_enabled")) },
            is_enabled_fn: unsafe { std::mem::transmute(resolve(r, "v1:server:is_enabled")) },
            set_player_entity_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:players:set_entity"))
            },
            send_usercmd_fn: unsafe { std::mem::transmute(resolve(r, "v1:net:send_usercmd")) },
            get_usercmd_fn: unsafe { std::mem::transmute(resolve(r, "v1:players:get_usercmd")) },
            move_snapshot_fn: unsafe { std::mem::transmute(resolve(r, "v1:move:snapshot")) },
            apply_local_view_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:players:apply_local_view"))
            },
            local_entity_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:players:local_entity"))
            },
        }
    }

    /// Queue client→server intent (`map <name>`, `disconnect`).
    pub fn client_cmd(&self, line: &str) -> Result<(), String> {
        let bytes = line.as_bytes();
        let rc = (self.client_cmd_fn)(self.ctx, bytes.as_ptr(), bytes.len());
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("net.client_cmd failed: {line}"))
        }
    }

    pub fn set_server_enabled(&self, on: bool) {
        (self.set_enabled_fn)(self.ctx, if on { 1 } else { 0 });
    }

    pub fn server_enabled(&self) -> bool {
        (self.is_enabled_fn)(self.ctx) != 0
    }

    /// Bind a spawned pawn entity to a player connection slot.
    pub fn set_player_entity(&self, slot: u32, entity: u64) {
        (self.set_player_entity_fn)(self.ctx, slot, entity);
    }

    /// Send a 64-byte UserCmd payload to the server (CLC::UserCmd).
    pub fn send_usercmd(&self, payload: &[u8; 64]) -> Result<(), String> {
        let rc = (self.send_usercmd_fn)(self.ctx, payload.as_ptr(), payload.len());
        if rc == 0 {
            Ok(())
        } else {
            Err("net.send_usercmd failed".into())
        }
    }

    /// Read latest UserCmd for a player slot into a 64-byte buffer.
    pub fn get_usercmd(&self, slot: u32, out: &mut [u8; 64]) -> Result<(), String> {
        let rc = (self.get_usercmd_fn)(self.ctx, slot, out.as_mut_ptr(), out.len());
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("players.get_usercmd slot={slot} failed"))
        }
    }

    /// Sample current `+forward`/… move state (axes + buttons).
    pub fn move_snapshot(&self, axes: &mut [f32; 4], buttons: &mut u64) -> Result<(), String> {
        let rc = (self.move_snapshot_fn)(self.ctx, axes.as_mut_ptr(), axes.len(), buttons);
        if rc == 0 {
            Ok(())
        } else {
            Err("move.snapshot failed".into())
        }
    }

    /// Client mlook: stamp absolute viewangles onto the local mirrored pawn camera.
    pub fn apply_local_view(&self, yaw: f32, pitch: f32) {
        (self.apply_local_view_fn)(self.ctx, yaw, pitch);
    }

    /// Local listen player's pawn entity, or `None` if unbound.
    pub fn local_entity(&self) -> Option<u64> {
        let id = (self.local_entity_fn)(self.ctx);
        if id == 0 {
            None
        } else {
            Some(id)
        }
    }
}
