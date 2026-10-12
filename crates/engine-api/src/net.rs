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
type ServerActivatedFn = extern "C" fn(*mut u8) -> i32;
type InGameFn = extern "C" fn(*mut u8) -> i32;
type SetPlayerEntityFn = extern "C" fn(*mut u8, u32, u64) -> i32;
type SendUsercmdFn = extern "C" fn(*mut u8, *const u8, usize) -> i32;
type GetUsercmdFn = extern "C" fn(*mut u8, u32, *mut u8, usize) -> i32;
type MoveSnapshotFn = extern "C" fn(*mut u8, *mut f32, usize, *mut u64) -> i32;
/// yaw, pitch, roll, eye_height, fov_y (radians where angular).
type ApplyLocalViewFn = extern "C" fn(*mut u8, f32, f32, f32, f32, f32) -> i32;
type LocalEntityFn = extern "C" fn(*mut u8) -> u64;
/// register(name, size) → msgid (>0) or -1
type UmRegisterFn = extern "C" fn(*mut u8, *const u8, usize, i16) -> i32;
/// send(dest, name, data) — dest 0 = broadcast; >0 = player slot
type UmSendFn = extern "C" fn(*mut u8, i32, *const u8, usize, *const u8, usize) -> i32;
/// hook(name, callback) — client MsgFunc
type UmHookFn = extern "C" fn(
    *mut u8,
    *const u8,
    usize,
    extern "C" fn(*const u8, usize, *const u8, usize),
) -> i32;
type PlayerNetIdFn = extern "C" fn(*mut u8, u32) -> u32;
type ClientSetViewNetIdFn = extern "C" fn(*mut u8, u16) -> i32;
type ClientSetLightstyleFn =
    extern "C" fn(*mut u8, u8, *const u8, usize) -> i32;
pub struct NetApi {
    ctx: *mut u8,
    client_cmd_fn: ClientCmdFn,
    set_enabled_fn: SetEnabledFn,
    is_enabled_fn: IsEnabledFn,
    server_activated_fn: ServerActivatedFn,
    in_game_fn: InGameFn,
    set_player_entity_fn: SetPlayerEntityFn,
    send_usercmd_fn: SendUsercmdFn,
    get_usercmd_fn: GetUsercmdFn,
    move_snapshot_fn: MoveSnapshotFn,
    apply_local_view_fn: ApplyLocalViewFn,
    local_entity_fn: LocalEntityFn,
    um_register_fn: UmRegisterFn,
    um_send_fn: UmSendFn,
    um_hook_fn: UmHookFn,
    player_net_id_fn: PlayerNetIdFn,
    client_set_view_net_id_fn: ClientSetViewNetIdFn,
    client_set_lightstyle_fn: ClientSetLightstyleFn,
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
            server_activated_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:server:activated"))
            },
            in_game_fn: unsafe { std::mem::transmute(resolve(r, "v1:net:in_game")) },
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
            um_register_fn: unsafe { std::mem::transmute(resolve(r, "v1:um:register")) },
            um_send_fn: unsafe { std::mem::transmute(resolve(r, "v1:um:send")) },
            um_hook_fn: unsafe { std::mem::transmute(resolve(r, "v1:um:hook")) },
            player_net_id_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:net:player_net_id"))
            },
            client_set_view_net_id_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:client:set_view_net_id"))
            },
            client_set_lightstyle_fn: unsafe {
                std::mem::transmute(resolve(r, "v1:client:set_lightstyle"))
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

    /// Notify host: server world ready (after `map`); host runs local client connect + init.
    pub fn server_activated(&self) {
        (self.server_activated_fn)(self.ctx);
    }

    /// Client in **play** phase (listen in-process or UDP remote).
    pub fn in_game(&self) -> bool {
        (self.in_game_fn)(self.ctx) != 0
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

    /// Client view stamp onto the local mirrored pawn (angles + eye + fov).
    ///
    /// Angles and `fov_y` in radians; `eye_height` = feet→eye world units (Z-up).
    pub fn apply_local_view(
        &self,
        yaw: f32,
        pitch: f32,
        roll: f32,
        eye_height: f32,
        fov_y: f32,
    ) {
        (self.apply_local_view_fn)(self.ctx, yaw, pitch, roll, eye_height, fov_y);
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

    /// Register a named UserMessage (server **and** client). Returns msgid.
    ///
    /// `size` — max payload bytes, or `-1` / `0` for variable (GoldSrc-style).
    pub fn um_register(&self, name: &str, size: i16) -> Result<u16, String> {
        let bytes = name.as_bytes();
        let rc = (self.um_register_fn)(self.ctx, bytes.as_ptr(), bytes.len(), size);
        if rc > 0 {
            Ok(rc as u16)
        } else {
            Err(format!("um.register '{name}' failed"))
        }
    }

    /// Send UserMessage. `dest = 0` broadcast; `dest > 0` → player slot.
    pub fn um_send(&self, dest: i32, name: &str, data: &[u8]) -> Result<(), String> {
        let nb = name.as_bytes();
        let rc = (self.um_send_fn)(
            self.ctx,
            dest,
            nb.as_ptr(),
            nb.len(),
            data.as_ptr(),
            data.len(),
        );
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("um.send '{name}' failed"))
        }
    }

    /// Broadcast UserMessage to all clients (and apply locally where needed).
    pub fn um_broadcast(&self, name: &str, data: &[u8]) -> Result<(), String> {
        self.um_send(0, name, data)
    }

    /// Send UserMessage to one connection slot.
    pub fn um_send_slot(&self, slot: u32, name: &str, data: &[u8]) -> Result<(), String> {
        self.um_send((slot as i32) + 1, name, data)
    }

    /// Client: hook a UserMessage by name (Xash `pfnHookUserMsg`).
    pub fn um_hook(
        &self,
        name: &str,
        callback: extern "C" fn(*const u8, usize, *const u8, usize),
    ) -> Result<(), String> {
        let nb = name.as_bytes();
        let rc = (self.um_hook_fn)(self.ctx, nb.as_ptr(), nb.len(), callback);
        if rc == 0 {
            Ok(())
        } else {
            Err(format!("um.hook '{name}' failed"))
        }
    }

    /// Server: replication `net_id` for a slot's pawn (`0` = none).
    pub fn player_net_id(&self, slot: u32) -> Option<u16> {
        let id = (self.player_net_id_fn)(self.ctx, slot);
        if id == 0 {
            None
        } else {
            Some(id as u16)
        }
    }

    /// Client: camera follow target (`NetworkEntity` net_id).
    pub fn client_set_view_net_id(&self, net_id: u16) -> Result<(), String> {
        let rc = (self.client_set_view_net_id_fn)(self.ctx, net_id);
        if rc == 0 {
            Ok(())
        } else {
            Err("client.set_view_net_id failed".into())
        }
    }

    /// Client: lightstyle pattern (for renderer).
    pub fn client_set_lightstyle(&self, style: u8, pattern: &str) -> Result<(), String> {
        let bytes = pattern.as_bytes();
        let rc = (self.client_set_lightstyle_fn)(
            self.ctx,
            style,
            bytes.as_ptr(),
            bytes.len(),
        );
        if rc == 0 {
            Ok(())
        } else {
            Err("client.set_lightstyle failed".into())
        }
    }

}
