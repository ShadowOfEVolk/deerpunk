//! MethodDB-generated API (`namespace` + `bind`) on host subsystems.

use deerpunk_engine_common::cvar_def::ARCHIVE;

deerpunk_engine_common::cvar_def!(
    "cl_cmdrate",
    "30",
    "Max UserCmd sends per second from client (0 = every frame). Does not affect server replication tick.",
    ARCHIVE
);

use deerpunk_engine_macro as methoddb;

use std::ffi::c_void;

use deerpunk_engine_common::network::protocol::SVC;
use deerpunk_engine_common::network::sound_event::{SoundPlayRpc, SoundStopRpc};
use deerpunk_engine_common::network::{UserCmd, USERCMD_SIZE};

use deerpunk_engine_common::network::HostCommand;
use crate::move_state;
use crate::user_messages::UmHookFn;
use crate::physics_trace::PhysicsTraceResult;
use crate::engine_api::physics::CapsuleCastHitOut;
use deerpunk_engine_common::components::mesh3d::Mesh3D;
use deerpunk_engine_common::components::SkinnedMesh3D;
use deerpunk_engine_common::physics::ColliderPart;
use deerpunk_engine_api::components::ComponentRegisterDesc;
use deerpunk_engine_api::resources::ResourceLoaderDesc;
use crate::gamelib_abi::ScheduleSystemFn;

#[methoddb::namespace(name = "v1:net", on_host, scope = ["server", "client"])]
impl crate::Host {
    #[methoddb::bind(name = "client_cmd")]
    pub fn api_net_client_cmd(&self, line: &str) -> i32 {
        let line = line.trim();
        if line.is_empty() {
            return -1;
        }
        let mut parts = line.split_whitespace();
        let Some(cmd) = parts.next() else {
            return -1;
        };
        match cmd {
            "disconnect" => {
                self.loopback.post_host_command(HostCommand::StopServer);
                0
            }
            _ => {
                log::warn!("[net] unknown client_cmd: {line}");
                -1
            }
        }
    }

    #[methoddb::bind(name = "in_game")]
    pub fn api_net_in_game(&self) -> i32 {
        if self.client_in_play() {
            1
        } else {
            0
        }
    }

    #[methoddb::bind(name = "msg_register")]
    pub fn api_net_msg_register(&mut self, name: &str, size: i16) -> i32 {
        match self.um.register(name, size) {
            Ok(id) => id as i32,
            Err(e) => {
                log::warn!("[um] register: {e}");
                -1
            }
        }
    }

    #[methoddb::bind(name = "msg_send")]
    pub fn api_net_msg_send(&mut self, dest: i32, name: &str, data: &[u8]) -> i32 {
        let slot = if dest <= 0 {
            None
        } else {
            Some((dest - 1) as u32)
        };
        match self.um_send(name, data, slot) {
            Ok(()) => 0,
            Err(e) => {
                log::warn!("[um] send: {e}");
                -1
            }
        }
    }

    #[methoddb::bind(name = "msg_hook")]
    pub fn api_net_msg_hook(&mut self, name: &str, callback: UmHookFn) -> i32 {
        match self.um.hook(name, callback) {
            Ok(()) => 0,
            Err(e) => {
                log::warn!("[um] hook: {e}");
                -1
            }
        }
    }

    #[methoddb::bind(name = "send_usercmd")]
    pub fn api_net_send_usercmd(&mut self, data: &[u8]) -> i32 {
        if data.len() < USERCMD_SIZE {
            return -1;
        }
        let Some(cmd) = UserCmd::decode(data) else {
            return -1;
        };
        let rate = self.cvar.get_f32_or("cl_cmdrate", 30.0);
        if rate > 0.0 {
            let min_dt = 1.0 / rate;
            let client = self.client_engine.as_mut().unwrap();
            let now = client.time();
            if now - client.last_usercmd_send_secs < min_dt {
                return 1;
            }
            client.last_usercmd_send_secs = now;
        }
        let sent_at = self.client_engine.as_ref().unwrap().time();
        let cmd = self
            .client_engine
            .as_mut()
            .unwrap()
            .usercmd_channel
            .note_sent(cmd, sent_at);
        let packet = cmd.encode_clc_packet().to_vec();
        let via = if self.client_net_active() {
            "c2s udp"
        } else {
            "c2s local"
        };
        crate::net_packet_log::log_clc_out(&self.cvar, via, &packet);
        if self.client_net_active() {
            if !self.client_in_play() {
                return 0;
            }
            self.client_engine.as_mut().unwrap().network.send_c2s(packet);
        } else {
            let server_on = self
                .server_thread
                .as_ref()
                .map(|s| s.is_enabled())
                .unwrap_or_else(|| self.server_engine.as_ref().map(|e| e.enabled).unwrap_or(false));
            if !self.client_in_play() || !server_on {
                return 0;
            }
            self.loopback.send_client_packet(packet);
        }
        0
    }

    #[methoddb::bind(name = "player_net_id")]
    pub fn api_net_player_net_id(&self, slot: u32) -> u32 {
        #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
        {
            return self.player_net_id(slot).unwrap_or(0) as u32;
        }
        #[cfg(not(any(feature = "dedicated-server", feature = "client", feature = "full")))]
        {
            let _ = slot;
            0
        }
    }
}

#[methoddb::namespace(name = "v1:players", on_host, scope = ["server", "client"])]
impl crate::Host {
    #[methoddb::bind(name = "set_entity")]
    pub fn api_players_set_entity(&mut self, slot: u32, entity: u64) -> i32 {
        if self.server_engine.is_none() {
            log::error!("[players] set_entity on client host (server thread only)");
            return -1;
        }
        self.server_mut().players.set_entity(slot, entity);
        0
    }

    #[methoddb::bind(name = "local_entity")]
    pub fn api_players_local_entity(&self) -> u64 {
        #[cfg(any(feature = "client", feature = "full"))]
        if self.client_engine.is_some() {
            return self.local_client_entity().unwrap_or(0);
        }
        let Some(slot) = self.server().players.local_slot() else {
            return 0;
        };
        self.server()
            .players
            .get(slot)
            .and_then(|p| p.entity)
            .unwrap_or(0)
    }

    #[methoddb::bind(name = "get_usercmd")]
    pub fn api_players_get_usercmd(&self, slot: u32, out: *mut u8, out_len: usize) -> i32 {
        if out.is_null() || out_len < USERCMD_SIZE {
            return -1;
        }
        if self.server_engine.is_none() {
            return -1;
        }
        let Some(cmd) = self.server().players.usercmd(slot) else {
            return -1;
        };
        let bytes = cmd.encode();
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, USERCMD_SIZE);
        }
        0
    }

    #[methoddb::bind(name = "set_simorg")]
    pub fn api_players_set_simorg(&mut self, x: f32, y: f32, z: f32) -> i32 {
        #[cfg(any(feature = "client", feature = "full"))]
        if let Some(client) = self.client_engine.as_mut() {
            client.set_simorg(glam::Vec3::new(x, y, z));
            return 0;
        }
        -1
    }

    #[methoddb::bind(name = "clear_simorg")]
    pub fn api_players_clear_simorg(&mut self) -> i32 {
        #[cfg(any(feature = "client", feature = "full"))]
        if let Some(client) = self.client_engine.as_mut() {
            client.clear_simorg();
            return 0;
        }
        -1
    }

    #[methoddb::bind(name = "usercmd_ack")]
    pub fn api_players_usercmd_ack(&self) -> u32 {
        #[cfg(any(feature = "client", feature = "full"))]
        if let Some(client) = self.client_engine.as_ref() {
            return client.usercmd_channel.incoming_acknowledged;
        }
        0
    }

    #[methoddb::bind(name = "usercmd_outgoing")]
    pub fn api_players_usercmd_outgoing(&self) -> u32 {
        #[cfg(any(feature = "client", feature = "full"))]
        if let Some(client) = self.client_engine.as_ref() {
            return client.usercmd_channel.outgoing_sequence;
        }
        0
    }

    /// Copy a previously sent cmd by absolute `sequence` into `out` (64 bytes).
    #[methoddb::bind(name = "usercmd_sent_time")]
    pub fn api_players_usercmd_sent_time(&self, sequence: u32) -> f32 {
        #[cfg(any(feature = "client", feature = "full"))]
        if let Some(client) = self.client_engine.as_ref() {
            return client.usercmd_channel.sent_time(sequence).unwrap_or(-1.0);
        }
        -1.0
    }

    /// Authoritative server feet (`server_feet` when skipping repl transform, else pawn transform).
    #[methoddb::bind(name = "server_feet")]
    pub fn api_players_server_feet(&self, out: *mut f32) -> i32 {
        if out.is_null() {
            return -1;
        }
        #[cfg(any(feature = "client", feature = "full"))]
        if let Some(client) = self.client_engine.as_ref() {
            use deerpunk_engine_common::components::TransformComponent;
            if client.local_sim.server_valid {
                unsafe {
                    *out.add(0) = client.local_sim.server_feet.x;
                    *out.add(1) = client.local_sim.server_feet.y;
                    *out.add(2) = client.local_sim.server_feet.z;
                }
                return 0;
            }
            if let Some(ent) = self.local_client_entity() {
                let tid = client.world.component_init::<TransformComponent>();
                if let Some(t) = client.world.get::<TransformComponent>(ent, tid) {
                    unsafe {
                        *out.add(0) = t.position.x;
                        *out.add(1) = t.position.y;
                        *out.add(2) = t.position.z;
                    }
                    return 0;
                }
            }
        }
        -1
    }

    #[methoddb::bind(name = "prediction_correction")]
    pub fn api_players_prediction_correction(
        &mut self,
        pred_x: f32,
        pred_y: f32,
        pred_z: f32,
        auth_x: f32,
        auth_y: f32,
        auth_z: f32,
    ) -> i32 {
        let smooth = self.cvar.get_f32_or("cl_smoothtime", 0.1);
        #[cfg(any(feature = "client", feature = "full"))]
        if let Some(client) = self.client_engine.as_mut() {
            client.push_prediction_correction(
                glam::Vec3::new(pred_x, pred_y, pred_z),
                glam::Vec3::new(auth_x, auth_y, auth_z),
                smooth,
            );
            return 0;
        }
        -1
    }

    #[methoddb::bind(name = "usercmd_lookup")]
    pub fn api_players_usercmd_lookup(&self, sequence: u32, out: *mut u8, out_len: usize) -> i32 {
        if out.is_null() || out_len < USERCMD_SIZE {
            return -1;
        }
        #[cfg(any(feature = "client", feature = "full"))]
        if let Some(client) = self.client_engine.as_ref() {
            let Some(cmd) = client.usercmd_channel.get(sequence) else {
                return -1;
            };
            let bytes = cmd.encode();
            unsafe {
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, USERCMD_SIZE);
            }
            return 0;
        }
        -1
    }
}

#[methoddb::namespace(name = "v1:move", on_host, scope = ["client"])]
impl crate::Host {
    #[methoddb::bind(name = "snapshot")]
    pub fn api_move_snapshot(
        &self,
        axes_ptr: *mut f32,
        axis_count: usize,
        buttons_ptr: *mut u64,
    ) -> i32 {
        if axes_ptr.is_null() || buttons_ptr.is_null() || axis_count == 0 {
            return -1;
        }
        let (axes, buttons) = move_state::snapshot();
        let n = axis_count.min(axes.len());
        unsafe {
            std::ptr::copy_nonoverlapping(axes.as_ptr(), axes_ptr, n);
            *buttons_ptr = buttons;
        }
        0
    }
}

#[cfg(any(feature = "client", feature = "full"))]
#[methoddb::namespace(name = "v1:client", on_host, scope = ["client"])]
impl crate::Host {
    #[methoddb::bind(name = "set_view_net_id")]
    pub fn api_client_set_view_net_id(&mut self, net_id: u16) -> i32 {
        self.client_set_view_net_id(net_id);
        0
    }

    #[methoddb::bind(name = "set_lightstyle")]
    pub fn api_client_set_lightstyle(&mut self, style: u8, pattern: &str) -> i32 {
        self.client_set_lightstyle(style, pattern);
        0
    }

    /// `out[0..3]` eye, `out[3..6]` normalized look (Z-up).
    #[methoddb::bind(name = "scene_time")]
    pub fn api_client_scene_time(&self) -> f32 {
        self.client_engine
            .as_ref()
            .map(|c| c.time())
            .unwrap_or(0.0)
    }

    #[methoddb::bind(name = "view_ray")]
    pub fn api_client_view_ray(&self, out: *mut f32) -> i32 {
        if out.is_null() {
            return -1;
        }
        let Some(client) = self.client_engine.as_ref() else {
            return -1;
        };
        let Some(ent) = self.local_client_entity() else {
            return -1;
        };
        use deerpunk_engine_client::view_camera::{camera_world_at_entity, find_view_camera_child};
        use glam::Vec3;

        let pawn = ent;
        let Some(cam_ent) = find_view_camera_child(&client.world, pawn) else {
            return -1;
        };
        let Some((eye, view_rot)) = camera_world_at_entity(&client.world, cam_ent) else {
            return -1;
        };
        let dir = (view_rot * Vec3::Y).normalize_or_zero();
        if dir.length_squared() < 1e-8 {
            return -1;
        }
        unsafe {
            *out.add(0) = eye.x;
            *out.add(1) = eye.y;
            *out.add(2) = eye.z;
            *out.add(3) = dir.x;
            *out.add(4) = dir.y;
            *out.add(5) = dir.z;
        }
        0
    }

    #[methoddb::bind(name = "debug_aabb")]
    pub fn api_client_debug_aabb(
        &mut self,
        min: *const f32,
        max: *const f32,
        r: f32,
        g: f32,
        b: f32,
        a: f32,
    ) -> i32 {
        if min.is_null() || max.is_null() {
            return -1;
        }
        let Some(client) = self.client_engine.as_mut() else {
            return -1;
        };
        let color = deerpunk_engine_renderer::DebugColor { r, g, b, a };
        let mn = unsafe { [*min, *min.add(1), *min.add(2)] };
        let mx = unsafe { [*max, *max.add(1), *max.add(2)] };
        client.game_debug_mut().aabb(mn, mx, color);
        0
    }
}

#[cfg(not(any(feature = "client", feature = "full")))]
#[methoddb::namespace(name = "v1:client", on_host, scope = ["client"])]
impl crate::Host {
    #[methoddb::bind(name = "set_view_net_id")]
    pub fn api_client_set_view_net_id(&mut self, _net_id: u16) -> i32 {
        -1
    }

    #[methoddb::bind(name = "set_lightstyle")]
    pub fn api_client_set_lightstyle(&mut self, _style: u8, _pattern: &str) -> i32 {
        -1
    }

    #[methoddb::bind(name = "scene_time")]
    pub fn api_client_scene_time(&self) -> f32 {
        0.0
    }

    #[methoddb::bind(name = "view_ray")]
    pub fn api_client_view_ray(&self, _out: *mut f32) -> i32 {
        -1
    }

    #[methoddb::bind(name = "debug_aabb")]
    pub fn api_client_debug_aabb(
        &mut self,
        _min: *const f32,
        _max: *const f32,
        _r: f32,
        _g: f32,
        _b: f32,
        _a: f32,
    ) -> i32 {
        -1
    }
}

#[cfg(any(feature = "client", feature = "full"))]
#[methoddb::namespace(name = "v1:ui", on_host, scope = ["server", "client"])]
impl crate::Host {
    #[methoddb::bind(name = "load")]
    pub fn api_ui_load(&mut self, id: &str) -> i32 {
        ui_client_op(self, |c| c.ui_load(id))
    }

    #[methoddb::bind(name = "unload")]
    pub fn api_ui_unload(&mut self, id: &str) -> i32 {
        ui_client_op(self, |c| c.ui_unload(id))
    }

    #[methoddb::bind(name = "show")]
    pub fn api_ui_show(&mut self, id: &str) -> i32 {
        ui_client_op(self, |c| c.ui_show(id))
    }

    #[methoddb::bind(name = "hide")]
    pub fn api_ui_hide(&mut self, id: &str) -> i32 {
        ui_client_op(self, |c| c.ui_hide(id))
    }

    #[methoddb::bind(name = "lua_eval")]
    pub fn api_ui_lua_eval(&mut self, code: &str) -> i32 {
        ui_client_op(self, |c| c.ui_lua_eval(code))
    }

    #[methoddb::bind(name = "data_model_set_bool")]
    pub fn api_ui_data_model_set_bool(&mut self, document: &str, field: &str, value: i32) -> i32 {
        ui_client_op(self, |c| c.ui_data_model_set_bool(document, field, value != 0))
    }

    #[methoddb::bind(name = "data_model_set_number")]
    pub fn api_ui_data_model_set_number(&mut self, document: &str, field: &str, value: f32) -> i32 {
        ui_client_op(self, |c| c.ui_data_model_set_number(document, field, value as f64))
    }

    #[methoddb::bind(name = "data_model_set_string")]
    pub fn api_ui_data_model_set_string(&mut self, document: &str, field: &str, value: &str) -> i32 {
        ui_client_op(self, |c| c.ui_data_model_set_string(document, field, value))
    }
}

#[cfg(not(any(feature = "client", feature = "full")))]
#[methoddb::namespace(name = "v1:ui", on_host, scope = ["server", "client"])]
impl crate::Host {
    #[methoddb::bind(name = "load")]
    pub fn api_ui_load(&mut self, _id: &str) -> i32 {
        -1
    }

    #[methoddb::bind(name = "unload")]
    pub fn api_ui_unload(&mut self, _id: &str) -> i32 {
        -1
    }

    #[methoddb::bind(name = "show")]
    pub fn api_ui_show(&mut self, _id: &str) -> i32 {
        -1
    }

    #[methoddb::bind(name = "hide")]
    pub fn api_ui_hide(&mut self, _id: &str) -> i32 {
        -1
    }

    #[methoddb::bind(name = "lua_eval")]
    pub fn api_ui_lua_eval(&mut self, _code: &str) -> i32 {
        -1
    }

    #[methoddb::bind(name = "data_model_set_bool")]
    pub fn api_ui_data_model_set_bool(&mut self, _document: &str, _field: &str, _value: i32) -> i32 {
        -1
    }

    #[methoddb::bind(name = "data_model_set_number")]
    pub fn api_ui_data_model_set_number(
        &mut self,
        _document: &str,
        _field: &str,
        _value: f32,
    ) -> i32 {
        -1
    }

    #[methoddb::bind(name = "data_model_set_string")]
    pub fn api_ui_data_model_set_string(
        &mut self,
        _document: &str,
        _field: &str,
        _value: &str,
    ) -> i32 {
        -1
    }
}

#[cfg(any(feature = "client", feature = "full"))]
fn ui_client_op<F>(host: &mut crate::Host, f: F) -> i32
where
    F: FnOnce(&mut deerpunk_engine_client::client_engine::ClientEngine) -> Result<(), String>,
{
    let Some(client) = host.client_engine.as_mut() else {
        log::error!("[ui] no client engine");
        return -1;
    };
    match f(client) {
        Ok(()) => 0,
        Err(e) => {
            log::error!("[ui] {e}");
            -1
        }
    }
}

#[methoddb::namespace(name = "v1:world", on_host, scope = ["server", "client"])]
impl crate::Host {
    #[methoddb::bind(name = "spawn")]
    pub fn api_world_spawn(&mut self) -> u64 {
        self.server_mut().world.spawn()
    }

    #[methoddb::bind(name = "has")]
    pub fn api_world_has(&self, entity: u64, comp: u64) -> bool {
        self.server().world.has(entity, comp)
    }

    #[methoddb::bind(name = "remove")]
    pub fn api_world_remove(&mut self, entity: u64, comp: u64) {
        self.server_mut().world.remove(entity, comp);
    }

    #[methoddb::bind(name = "get")]
    pub fn api_world_get(&mut self, entity: u64, comp: u64) -> *mut c_void {
        self.world_get_component_ptr(entity, comp)
    }

    #[methoddb::bind(name = "set")]
    pub fn api_world_set(
        &mut self,
        entity: u64,
        comp: u64,
        data: *const c_void,
        size: usize,
    ) {
        self.world_set_component_raw(entity, comp, data, size);
    }

    #[methoddb::bind(name = "clear")]
    pub fn api_world_clear(&mut self) {
        self.clear_server_world();
    }

    #[methoddb::bind(name = "free_value")]
    pub fn api_world_free_value(&mut self, _ptr: *mut c_void) {}

    /// Spawn `{gamedir}/prefabs/{name}.prefab` into the server world.
    #[methoddb::bind(name = "spawn_prefab")]
    pub fn api_world_spawn_prefab(
        &mut self,
        name: &str,
        opts: &deerpunk_engine_api::spawn_prefab::SpawnPrefabOpts,
    ) -> u64 {
        if name.is_empty() {
            return 0;
        }
        match self.spawn_prefab(name, opts) {
            Ok(id) => id,
            Err(e) => {
                log::error!("[world] spawn_prefab {name}: {e}");
                0
            }
        }
    }

    #[methoddb::bind(name = "spawn_bundle")]
    pub fn api_world_spawn_bundle(
        &mut self,
        _comp_ids: *const u64,
        _comp_data: *const *mut c_void,
        _count: usize,
    ) -> u64 {
        0
    }
}

#[methoddb::namespace(name = "v1:world", on_host, scope = ["server"])]
impl crate::Host {
    #[methoddb::bind(name = "destroy")]
    pub fn api_world_destroy(&mut self, entity: u64) {
        self.server_mut().world.destroy(entity);
    }

    #[methoddb::bind(name = "component_register")]
    pub fn api_world_component_register(&mut self, desc: &ComponentRegisterDesc) -> u64 {
        crate::engine_api::components::register_component_desc(self, desc)
    }

}

#[methoddb::namespace(name = "v1:server", on_host, scope = ["server"])]
impl crate::Host {
    #[methoddb::bind(name = "set_enabled")]
    pub fn api_set_server_enabled(&mut self, enabled: bool) {
        self.set_server_enabled(enabled);
    }

    #[methoddb::bind(name = "activated")]
    pub fn api_server_activated(&mut self) {
        if self.server_engine.is_some() {
            self.loopback
                .post_host_command(deerpunk_engine_common::network::HostCommand::InprocessConnect);
        } else {
            self.on_server_activated();
        }
    }

    #[methoddb::bind(name = "is_enabled")]
    pub fn api_server_is_enabled(&self) -> i32 {
        #[cfg(any(feature = "client", feature = "full"))]
        if self.server_engine.is_none() {
            return if self
                .server_thread
                .as_ref()
                .map(|s| s.is_enabled())
                .unwrap_or(false)
            {
                1
            } else {
                0
            };
        }
        if self.server().enabled {
            1
        } else {
            0
        }
    }
}

#[methoddb::namespace(name = "v1:schedule", on_host, scope = ["server", "client"])]
impl crate::Host {
    #[methoddb::bind(name = "add_system")]
    pub fn api_schedule_add_system(
        &mut self,
        _system_fn: ScheduleSystemFn,
        _user_data: *mut c_void,
    ) {}

    #[methoddb::bind(name = "clear")]
    pub fn api_schedule_clear(&mut self) {}

    #[methoddb::bind(name = "run")]
    pub fn api_schedule_run(&mut self, _delta_time: f32) {}
}

#[methoddb::namespace(name = "v1:engine", on_host, scope = ["server", "client"])]
impl crate::Host {
    #[methoddb::bind(name = "frame")]
    pub fn api_engine_frame(&mut self, _delta_time: f32) -> i32 {
        0
    }

    /// Gamelib `server_shutdown` hook — host exit calls [`Host::shutdown`] separately.
    #[methoddb::bind(name = "shutdown")]
    pub fn api_engine_shutdown(&mut self) {}
}

#[methoddb::namespace(name = "v1:log", on_host, scope = ["server", "client"])]
impl crate::Host {
    /// `level`: 1=error, 2=warn, 3=info, 4=debug, 5=trace.
    #[methoddb::bind(name = "write")]
    pub fn api_log_write(&self, level: i32, msg: &str) -> i32 {
        match level {
            1 => log::error!(target: "gamelib", "{msg}"),
            2 => log::warn!(target: "gamelib", "{msg}"),
            3 => log::info!(target: "gamelib", "{msg}"),
            4 => log::debug!(target: "gamelib", "{msg}"),
            5 => log::trace!(target: "gamelib", "{msg}"),
            _ => log::info!(target: "gamelib", "{msg}"),
        }
        0
    }
}

#[methoddb::namespace(name = "v1:resources", on_host, scope = ["server", "client"])]
impl crate::Host {
    #[methoddb::bind(name = "register")]
    pub fn api_resources_register(&mut self, desc: &ResourceLoaderDesc) -> i32 {
        crate::engine_api::resources::register_loader_desc(self, desc)
    }
}

#[cfg(any(feature = "client", feature = "full"))]
#[methoddb::namespace(name = "v1:renderer", on_host, scope = ["server", "client"])]
impl crate::Host {
    #[methoddb::bind(name = "load_texture")]
    pub fn api_renderer_load_texture(&mut self, path: &str) -> u32 {
        match image::open(path) {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let (w, h) = rgba.dimensions();
                let pixels = rgba.into_raw();
                self.client_engine
                    .as_mut()
                    .map(|ce| ce.renderer_mut().upload_texture(&pixels, w, h).0)
                    .unwrap_or(0)
            }
            Err(e) => {
                log::error!("[renderer] failed to load texture '{}': {}", path, e);
                0
            }
        }
    }
}

#[cfg(not(any(feature = "client", feature = "full")))]
#[methoddb::namespace(name = "v1:renderer", on_host, scope = ["server", "client"])]
impl crate::Host {
    #[methoddb::bind(name = "load_texture")]
    pub fn api_renderer_load_texture(&mut self, _path: &str) -> u32 {
        0
    }
}

#[methoddb::namespace(name = "v1:physics", on_host, scope = ["server", "client"])]
impl crate::Host {
    /// Rapier segment trace — returns hit entity + fraction/normal/point.
    #[methoddb::bind(name = "trace_line")]
    pub fn api_physics_trace_line(
        &mut self,
        ox: f32,
        oy: f32,
        oz: f32,
        ex: f32,
        ey: f32,
        ez: f32,
        exclude_entity: u64,
        out: &mut PhysicsTraceResult,
    ) -> i32 {
        self.physics_trace_line(ox, oy, oz, ex, ey, ez, exclude_entity, out)
    }

    #[methoddb::bind(name = "create_shape")]
    pub fn api_physics_create_shape(&mut self, abi: &[u8]) -> u32 {
        if abi.is_empty() {
            return 0;
        }
        self.server_mut().physics.create_shape(abi)
    }

    #[methoddb::bind(name = "destroy_shape")]
    pub fn api_physics_destroy_shape(&mut self, handle: u32) -> i32 {
        self.server_mut().physics.destroy_shape(handle);
        0
    }

    #[methoddb::bind(name = "create_parts")]
    pub fn api_physics_create_parts(
        &mut self,
        parts_ptr: *const ColliderPart,
        count: usize,
    ) -> u32 {
        if parts_ptr.is_null() || count == 0 {
            return 0;
        }
        let parts = unsafe { std::slice::from_raw_parts(parts_ptr, count) };
        self.server_mut().physics.create_parts(parts)
    }

    #[methoddb::bind(name = "destroy_parts")]
    pub fn api_physics_destroy_parts(&mut self, handle: u32) -> i32 {
        self.server_mut().physics.destroy_parts(handle);
        0
    }

    #[methoddb::bind(name = "cast_capsule")]
    pub fn api_physics_cast_capsule(
        &mut self,
        center: *const f32,
        half_height: f32,
        radius: f32,
        dir: *const f32,
        max_dist: f32,
        exclude_entity: u64,
        out: &mut CapsuleCastHitOut,
    ) -> i32 {
        if center.is_null() || dir.is_null() {
            return -1;
        }
        let center = unsafe { [*center, *center.add(1), *center.add(2)] };
        let dir = unsafe { [*dir, *dir.add(1), *dir.add(2)] };
        *out = CapsuleCastHitOut::default();
        match self.server_mut().physics.cast_capsule(
            center,
            half_height,
            radius,
            dir,
            max_dist,
            exclude_entity,
        ) {
            Some(hit) => {
                out.hit = 1;
                out.start_solid = u32::from(hit.start_solid);
                out.toi = hit.toi;
                out.normal = hit.normal;
                out.point = hit.point;
                out.entity = hit.entity;
                1
            }
            None => 0,
        }
    }
}

#[cfg(any(feature = "client", feature = "full"))]
#[methoddb::namespace(name = "v1:input", on_host, scope = ["server", "client"])]
impl crate::Host {
    #[methoddb::bind(name = "is_key_down")]
    pub fn api_input_is_key_down(&self, key: u32) -> bool {
        self.client_engine
            .as_ref()
            .map(|ce| ce.input.is_down(key as u16))
            .unwrap_or(false)
    }

    #[methoddb::bind(name = "mouse_dx")]
    pub fn api_input_mouse_dx(&self) -> f32 {
        self.client_engine
            .as_ref()
            .map(|ce| ce.input.mouse_dx())
            .unwrap_or(0.0)
    }

    #[methoddb::bind(name = "mouse_dy")]
    pub fn api_input_mouse_dy(&self) -> f32 {
        self.client_engine
            .as_ref()
            .map(|ce| ce.input.mouse_dy())
            .unwrap_or(0.0)
    }

    #[methoddb::bind(name = "take_mouse_wheel")]
    pub fn api_input_take_mouse_wheel(&mut self) -> f32 {
        self.client_engine
            .as_mut()
            .map(|ce| ce.input.take_mouse_wheel())
            .unwrap_or(0.0)
    }

    #[methoddb::bind(name = "set_relative_mouse")]
    pub fn api_input_set_relative_mouse(&mut self, on: bool) -> i32 {
        if let Some(ce) = self.client_engine.as_mut() {
            ce.set_relative_mouse(on);
        }
        0
    }
}

#[cfg(not(any(feature = "client", feature = "full")))]
#[methoddb::namespace(name = "v1:input", on_host, scope = ["server", "client"])]
impl crate::Host {
    #[methoddb::bind(name = "is_key_down")]
    pub fn api_input_is_key_down(&self, _key: u32) -> bool {
        false
    }

    #[methoddb::bind(name = "mouse_dx")]
    pub fn api_input_mouse_dx(&self) -> f32 {
        0.0
    }

    #[methoddb::bind(name = "mouse_dy")]
    pub fn api_input_mouse_dy(&self) -> f32 {
        0.0
    }

    #[methoddb::bind(name = "take_mouse_wheel")]
    pub fn api_input_take_mouse_wheel(&self) -> f32 {
        0.0
    }

    #[methoddb::bind(name = "set_relative_mouse")]
    pub fn api_input_set_relative_mouse(&mut self, _on: bool) -> i32 {
        0
    }
}

#[methoddb::namespace(name = "v1:host", on_host, scope = ["server"])]
impl crate::Host {
    /// Queue a main-thread console proxy that forwards to this server thread's [`CmdSystem`].
    #[methoddb::bind(name = "register_main_console")]
    pub fn api_host_register_main_console(&mut self, name: &str) -> i32 {
        if name.trim().is_empty() {
            return -1;
        }
        crate::main_cmd_proxy::enqueue_main_console_proxy(name);
        0
    }
}

#[methoddb::namespace(name = "v1:host", on_host, scope = ["client"])]
impl crate::Host {
    /// Main thread: start / switch listen-server world (payload → server thread).
    #[methoddb::bind(name = "start_listen_map")]
    pub fn api_host_start_listen_map(&self, world: &str, seed: i32) -> i32 {
        self.request_listen_map(world, seed as i64)
    }
}

#[methoddb::namespace(name = "v1:host", on_host, scope = ["server", "client"])]
impl crate::Host {
    #[methoddb::bind(name = "flush_pending_albedos")]
    pub fn api_host_flush_pending_albedos(&mut self) -> i32 {
        #[cfg(any(feature = "client", feature = "full"))]
        {
            self.flush_pending_albedos() as i32
        }
        #[cfg(not(any(feature = "client", feature = "full")))]
        {
            -1
        }
    }

    #[methoddb::bind(name = "refresh_stream_meshes")]
    pub fn api_host_refresh_stream_meshes(&mut self) -> i32 {
        #[cfg(any(feature = "client", feature = "full"))]
        {
            if let Some(ce) = self.client_engine.as_mut() {
                let gamedir = self.fs.gamedir().to_path_buf();
                ce.refresh_stream_meshes(&mut self.mesh_registry, &gamedir, &self.resources);
            }
            0
        }
        #[cfg(not(any(feature = "client", feature = "full")))]
        {
            -1
        }
    }

    #[methoddb::bind(name = "client_track_static")]
    pub fn api_host_client_track_static(&mut self, entity: u64) -> i32 {
        #[cfg(any(feature = "client", feature = "full"))]
        {
            if let Some(client) = self.client_engine.as_mut() {
                client.mesh_adopt_entity(&mut self.mesh_registry, entity);
                client.rep.track_static_entity(entity);
            }
            0
        }
        #[cfg(not(any(feature = "client", feature = "full")))]
        {
            let _ = entity;
            -1
        }
    }

    /// Load `Mesh3D` on the client ECS from a mesh URI (BSP `#mesh:world`, `#brush:N`, …).
    #[methoddb::bind(name = "client_apply_mesh_uri")]
    pub fn api_host_client_apply_mesh_uri(
        &mut self,
        entity: u64,
        uri_ptr: *mut u8,
        uri_len: usize,
    ) -> i32 {
        #[cfg(any(feature = "client", feature = "full"))]
        {
            let uri = methoddb_read_cstr(uri_ptr, uri_len);
            if uri.is_empty() {
                return 0;
            }
            let Some(ce) = self.client_engine.as_mut() else {
                return 0;
            };
            let gamedir = self.fs.gamedir().to_path_buf();
            match deerpunk_engine_common::scene::apply_mesh3d_uri(
                &ce.world,
                &mut self.mesh_registry,
                &gamedir,
                &self.resources,
                entity,
                &uri,
            ) {
                Ok(()) => {
                    ce.mesh_adopt_entity(&mut self.mesh_registry, entity);
                    let mesh_comp = ce.world.component_init::<Mesh3D>();
                    if let Some(mesh3d) = ce.world.get::<Mesh3D>(entity, mesh_comp) {
                        let detail = self
                            .mesh_registry
                            .get(mesh3d.mesh)
                            .map(|m| {
                                format!(
                                    "handle={} verts={} indices={}",
                                    mesh3d.mesh.0,
                                    m.positions.len(),
                                    m.indices.len()
                                )
                            })
                            .unwrap_or_else(|| {
                                format!(
                                    "handle={} (missing in registry — Mesh3D set via FFI may have released slot)",
                                    mesh3d.mesh.0
                                )
                            });
                        log::info!(
                            "[host] client_apply_mesh_uri '{uri}' ent={entity:#x} {detail}"
                        );
                    }
                    1
                }
                Err(e) => {
                    log::error!("[host] client_apply_mesh_uri '{uri}': {e}");
                    0
                }
            }
        }
        #[cfg(not(any(feature = "client", feature = "full")))]
        {
            let _ = (entity, uri_ptr, uri_len);
            0
        }
    }
}

fn methoddb_read_cstr(ptr: *mut u8, len: usize) -> String {
    if ptr.is_null() || len == 0 {
        return String::new();
    }
    unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len)).to_string()
    }
}

fn audio_queue_play(host: &mut crate::Host, play: SoundPlayRpc) {
    let mut packet = Vec::with_capacity(play.event.len() + 48);
    packet.push(SVC::SoundPlay as u8);
    packet.extend_from_slice(&play.encode());
    host.queue_sound_packet(packet);
}

fn audio_queue_stop(host: &mut crate::Host, sound_id: u32) {
    let pkt = SoundStopRpc::new(sound_id).to_packet();
    host.queue_sound_packet(pkt);
}

#[methoddb::namespace(name = "v1:audio", on_host, scope = ["server", "client"])]
impl crate::Host {
    #[methoddb::bind(name = "play_2d")]
    pub fn api_audio_play_2d(&mut self, event: &str, volume_u8: u32) -> u32 {
        let id = self.alloc_sound_id();
        audio_queue_play(
            self,
            SoundPlayRpc::play_2d(id, event, volume_u8 as f32 / 255.0, Vec::new()),
        );
        id
    }

    #[methoddb::bind(name = "play_3d")]
    pub fn api_audio_play_3d(
        &mut self,
        event: &str,
        x: f32,
        y: f32,
        z: f32,
        volume_u8: u32,
        param_name: &str,
        param_value: f32,
    ) -> u32 {
        let params = if param_name.is_empty() {
            Vec::new()
        } else {
            vec![(param_name.to_string(), param_value)]
        };
        let id = self.alloc_sound_id();
        audio_queue_play(
            self,
            SoundPlayRpc::play_3d(id, event, [x, y, z], volume_u8 as f32 / 255.0, params),
        );
        id
    }

    #[methoddb::bind(name = "start_attached")]
    pub fn api_audio_start_attached(
        &mut self,
        entity: u64,
        event: &str,
        x: f32,
        y: f32,
        z: f32,
        volume_u8: u32,
    ) -> u32 {
        let id = self.alloc_sound_id();
        let net_id = {
            #[cfg(any(feature = "dedicated-server", feature = "client", feature = "full"))]
            {
                self.server().rep.net_id_of(entity).unwrap_or(0)
            }
            #[cfg(not(any(feature = "dedicated-server", feature = "client", feature = "full")))]
            {
                0u16
            }
        };
        audio_queue_play(
            self,
            SoundPlayRpc::play_loop(id, event, [x, y, z], volume_u8 as f32 / 255.0, net_id),
        );
        id
    }

    #[methoddb::bind(name = "stop_sound")]
    pub fn api_audio_stop_sound(&mut self, sound_id: u32) -> i32 {
        if sound_id == 0 {
            return -1;
        }
        audio_queue_stop(self, sound_id);
        0
    }
}

#[cfg(any(feature = "client", feature = "full"))]
#[methoddb::namespace(name = "v1:audio", on_host, scope = ["client"])]
impl crate::Host {
    #[methoddb::bind(name = "set_world_mesh")]
    pub fn api_audio_set_world_mesh(&mut self, entity: u64) -> i32 {
        let ok = self
            .client_engine
            .as_mut()
            .map(|ce| ce.install_audio_world_mesh(&self.mesh_registry, entity))
            .unwrap_or(false);
        if ok {
            0
        } else {
            -1
        }
    }
}

#[methoddb::namespace(name = "v1:world", on_host, scope = ["server", "client"])]
impl crate::Host {
    #[methoddb::bind(name = "query")]
    pub fn api_world_query(
        &mut self,
        require: &[u64],
        exclude: &[u64],
        out: *mut u64,
        max: usize,
    ) -> usize {
        let _ = exclude;
        let qid = self.server_mut().world.query_prepare_ids(require);
        let mut ids = Vec::new();
        if let Some(batches) = self.server_mut().world.query_batches(qid) {
            for batch in batches {
                let entities =
                    unsafe { std::slice::from_raw_parts(batch.entities, batch.count) };
                ids.extend_from_slice(entities);
            }
        }
        self.server_mut().world.query_release(qid);
        let mut written = 0usize;
        for &e in &ids {
            if written >= max || out.is_null() {
                break;
            }
            unsafe {
                *out.add(written) = e;
            }
            written += 1;
        }
        written
    }

    #[methoddb::bind(name = "query_count")]
    pub fn api_world_query_count(&mut self, require: &[u64], exclude: &[u64]) -> usize {
        let _ = exclude;
        let qid = self.server_mut().world.query_prepare_ids(require);
        let mut count = 0usize;
        if let Some(batches) = self.server_mut().world.query_batches(qid) {
            for batch in batches {
                count += batch.count;
            }
        }
        self.server_mut().world.query_release(qid);
        count
    }

    #[methoddb::bind(name = "query_matches")]
    pub fn api_world_query_matches(
        &self,
        entity: u64,
        require: &[u64],
        exclude: &[u64],
    ) -> bool {
        for &c in require {
            if !self.server().world.has(entity, c) {
                return false;
            }
        }
        for &c in exclude {
            if self.server().world.has(entity, c) {
                return false;
            }
        }
        true
    }

    #[methoddb::bind(name = "query_prepare")]
    pub fn api_world_query_prepare(&mut self, expr: &str) -> u32 {
        self.server_mut().world.query_prepare(expr) as u32
    }

    #[methoddb::bind(name = "query_execute")]
    pub fn api_world_query_execute(
        &mut self,
        query_id: u32,
        callback: crate::gamelib_abi::WorldQueryCallback,
        ctx: *mut c_void,
    ) {
        let qid = query_id as usize;
        if let Some(batches) = self.server_mut().world.query_batches(qid) {
            for batch in batches {
                let entities =
                    unsafe { std::slice::from_raw_parts(batch.entities, batch.count) };
                let field_count = batch.columns.len();
                for (i, &entity) in entities.iter().enumerate() {
                    let fields: Vec<*const c_void> = (0..field_count)
                        .map(|f| unsafe { batch.columns[f].add(i * batch.sizes[f]) })
                        .collect();
                    callback(entity, fields.as_ptr(), field_count, ctx);
                }
            }
        }
    }
}

#[cfg(any(feature = "client", feature = "full"))]
#[methoddb::namespace(name = "v1:client_world", on_host, scope = ["server", "client"])]
impl crate::Host {
    #[methoddb::bind(name = "get")]
    pub fn api_client_world_get(&self, entity: u64, comp: u64) -> *mut c_void {
        deerpunk_engine_common::ecs_validate::assert_component_id(comp, "client_world::get");
        let Some(world) = self.client_engine.as_ref().map(|c| &c.world) else {
            return std::ptr::null_mut();
        };
        let ptr = world.get_raw(entity, comp);
        if ptr.is_null() {
            std::ptr::null_mut()
        } else {
            ptr as *mut c_void
        }
    }

    #[methoddb::bind(name = "set")]
    pub fn api_client_world_set(
        &mut self,
        entity: u64,
        comp: u64,
        data: *const c_void,
        size: usize,
    ) {
        let Some(ce) = self.client_engine.as_mut() else {
            log::error!("[client_world] set entity {entity:#x} comp {comp:#x}: client engine not initialized");
            return;
        };
        deerpunk_engine_common::ecs_validate::assert_component_id(comp, "client_world::set");
        deerpunk_engine_common::ecs_validate::assert_entity_alive(&ce.world, entity, "client_world::set");
        if data.is_null() {
            panic!("[client_world] set entity {entity:#x} comp {comp:#x}: null data pointer");
        }
        if size == 0 {
            panic!(
                "[client_world] set entity {entity:#x} comp {comp:#x}: payload size is 0 \
                 (zero-sized component type?)"
            );
        }
        if size == std::mem::size_of::<SkinnedMesh3D>() {
            crate::mesh_components::set_skinned_raw(
                &ce.world,
                &mut self.mesh_registry,
                entity,
                comp,
                data,
                size,
            );
            ce.mesh_adopt_entity(&mut self.mesh_registry, entity);
            return;
        }
        if size == std::mem::size_of::<Mesh3D>() {
            crate::mesh_components::set_mesh3d_raw(
                &ce.world,
                &mut self.mesh_registry,
                entity,
                comp,
                data,
                size,
            );
            ce.mesh_adopt_entity(&mut self.mesh_registry, entity);
            return;
        }
        ce.world.set_raw(entity, comp, data, size);
    }

    #[methoddb::bind(name = "has")]
    pub fn api_client_world_has(&self, entity: u64, comp: u64) -> bool {
        deerpunk_engine_common::ecs_validate::assert_component_id(comp, "client_world::has");
        self.client_engine
            .as_ref()
            .map(|c| c.world.has(entity, comp))
            .unwrap_or(false)
    }

    #[methoddb::bind(name = "is_alive")]
    pub fn api_client_world_is_alive(&self, entity: u64) -> bool {
        self.client_engine
            .as_ref()
            .map(|c| c.world.is_alive(entity))
            .unwrap_or(false)
    }

    #[methoddb::bind(name = "spawn")]
    pub fn api_client_world_spawn(&self) -> u64 {
        self.client_engine
            .as_ref()
            .map(|c| c.world.spawn())
            .unwrap_or(0)
    }

    #[methoddb::bind(name = "destroy")]
    pub fn api_client_world_destroy(&mut self, entity: u64) {
        if let Some(ce) = self.client_engine.as_mut() {
            ce.mesh_release_entity(&mut self.mesh_registry, entity);
            ce.drop_heap_mesh_components(&mut self.mesh_registry, entity);
            ce.world.destroy(entity);
        }
    }

    #[methoddb::bind(name = "remove")]
    pub fn api_client_world_remove(&mut self, entity: u64, comp: u64) {
        let Some(ce) = self.client_engine.as_mut() else {
            log::error!("[client_world] remove entity {entity:#x} comp {comp:#x}: client engine not initialized");
            return;
        };
        deerpunk_engine_common::ecs_validate::assert_component_id(comp, "client_world::remove");
        deerpunk_engine_common::ecs_validate::assert_entity_alive(&ce.world, entity, "client_world::remove");
        ce.world.remove(entity, comp);
    }

    #[methoddb::bind(name = "component_register")]
    pub fn api_client_world_component_register(
        &self,
        desc: &ComponentRegisterDesc,
    ) -> u64 {
        crate::engine_api::components::register_client_component_desc(self, desc)
    }

    #[methoddb::bind(name = "query")]
    pub fn api_client_world_query(
        &mut self,
        require: &[u64],
        exclude: &[u64],
        out: *mut u64,
        max: usize,
    ) -> usize {
        let _ = exclude;
        let Some(ce) = self.client_engine.as_mut() else {
            return 0;
        };
        let qid = ce.world.query_prepare_ids(require);
        let mut ids = Vec::new();
        if let Some(batches) = ce.world.query_batches(qid) {
            for batch in batches {
                let entities =
                    unsafe { std::slice::from_raw_parts(batch.entities, batch.count) };
                ids.extend_from_slice(entities);
            }
        }
        ce.world.query_release(qid);
        let mut written = 0usize;
        for &e in &ids {
            if written >= max || out.is_null() {
                break;
            }
            unsafe {
                *out.add(written) = e;
            }
            written += 1;
        }
        written
    }
}

#[methoddb::namespace(name = "v1:mesh", on_host, scope = ["client", "server"])]
impl crate::Host {
    #[methoddb::bind(name = "create")]
    pub fn api_mesh_create(
        &mut self,
        positions: *const f32,
        vertex_count: u32,
        normals: *const f32,
        uvs: *const f32,
        indices: *const c_void,
        index_count: u32,
        joint_indices: *const c_void,
        joint_weights: *const f32,
        lightmap_uvs: *const f32,
        lightmap_styles: *const f32,
        atlas_rects: *const f32,
    ) -> usize {
        let desc = crate::mesh_upload::MeshCreateDesc {
            positions,
            vertex_count,
            normals,
            uvs,
            indices,
            index_count,
            joint_indices,
            joint_weights,
            lightmap_uvs,
            lightmap_styles,
            atlas_rects,
            allocator_id: 0,
        };
        match crate::mesh_upload::mesh_from_desc(&desc) {
            Some(mesh) => self.mesh_registry.insert(mesh).0,
            None => 0,
        }
    }

    /// Like `v1:mesh:create` but tags the mesh with a registered custom
    /// allocator (`allocator_id`) so the renderer routes uploads/frees
    /// through it instead of the built-in slab. Used by streaming voxel
    /// chunks to reclaim GPU regions on rebuild.
    #[methoddb::bind(name = "create_with")]
    pub fn api_mesh_create_with(
        &mut self,
        positions: *const f32,
        vertex_count: u32,
        normals: *const f32,
        uvs: *const f32,
        indices: *const c_void,
        index_count: u32,
        joint_indices: *const c_void,
        joint_weights: *const f32,
        lightmap_uvs: *const f32,
        lightmap_styles: *const f32,
        atlas_rects: *const f32,
        allocator_id: u32,
    ) -> usize {
        let desc = crate::mesh_upload::MeshCreateDesc {
            positions,
            vertex_count,
            normals,
            uvs,
            indices,
            index_count,
            joint_indices,
            joint_weights,
            lightmap_uvs,
            lightmap_styles,
            atlas_rects,
            allocator_id,
        };
        match crate::mesh_upload::mesh_from_desc(&desc) {
            Some(mesh) => self.mesh_registry.insert(mesh).0,
            None => 0,
        }
    }

    /// Register a custom (game-supplied) mesh allocator. The engine wraps
    /// the FFI vtable + opaque ctx in a `ForeignMeshAllocator`, pushes it
    /// onto the client renderer's allocator table, and returns a stable
    /// id. The id is reused across `create_with` calls. Used by streaming
    /// voxel chunks to reclaim GPU regions on rebuild.
    ///
    /// `vtable_bytes` is the raw byte representation of
    /// `MeshAllocatorVTable` (three `extern "C" fn` pointers, `repr(C)`).
    /// `alloc_ctx` is an opaque pointer the engine passes back to every
    /// vtable call; the game keeps ownership and must keep it alive until
    /// the allocator is unregistered (engine shutdown / level end).
    ///
    /// Returns `u32::MAX` on failure (no client renderer, bad name, bad
    /// vtable size). Re-registering an existing name returns the
    /// previously assigned id without pushing a new entry.
    #[methoddb::bind(name = "register_allocator")]
    pub fn api_mesh_register_allocator(
        &mut self,
        name: &str,
        vtable_bytes: &[u8],
        alloc_ctx: *mut u8,
    ) -> u32 {
        use deerpunk_engine_common::resource::mesh_allocator::{
            ForeignMeshAllocator, MeshAllocatorRegistration, MeshAllocatorVTable,
        };

        if name.is_empty() {
            return u32::MAX;
        }
        // Idempotent: re-registering the same name returns the old id.
        if let Some(&id) = self.mesh_allocator_names.get(name) {
            return id;
        }
        if vtable_bytes.len() != std::mem::size_of::<MeshAllocatorVTable>() {
            return u32::MAX;
        }
        let vtable = unsafe {
            std::ptr::read_unaligned(vtable_bytes.as_ptr() as *const MeshAllocatorVTable)
        };
        let reg = MeshAllocatorRegistration { vtable, ctx: alloc_ctx };

        let Some(ce) = self.client_engine.as_mut() else {
            // Server-only host: nothing to register with.
            return u32::MAX;
        };
        let id = ce
            .renderer_mut()
            .register_mesh_allocator(Box::new(ForeignMeshAllocator::new(reg)));
        self.mesh_allocator_names.insert(name.to_owned(), id);
        id
    }

    /// Look up a registered allocator's id by name. Returns `u32::MAX` if
    /// no allocator is registered under `name`.
    #[methoddb::bind(name = "allocator_id")]
    pub fn api_mesh_allocator_id(&self, name: &str) -> u32 {
        self.mesh_allocator_names.get(name).copied().unwrap_or(u32::MAX)
    }

    /// `v1:mesh:release` is **removed**: mesh lifetime is owned by the
    /// `MeshRegistry` and reclaimed by its per-frame sweep. Calling this
    /// entry point is a programming error — there is no valid use case.
    #[methoddb::bind(name = "release")]
    pub fn api_mesh_release(&mut self, _handle: usize) {
        panic!(
            "v1:mesh:release is removed — mesh lifetime is owned by MeshRegistry and \
            reclaimed by the per-frame sweep. Drop the Mesh3D component or \
            destroy the entity instead."
        );
    }

    #[methoddb::bind(name = "content_id")]
    pub fn api_mesh_content_id(&self, handle: usize) -> u64 {
        use deerpunk_engine_common::resource::mesh_handle::MeshHandle;
        self.mesh_registry.content_id(MeshHandle(handle))
    }
}

#[methoddb::namespace(name = "v1:bone_buffer", on_host, scope = ["client"])]
impl crate::Host {
    #[methoddb::bind(name = "alloc")]
    pub fn api_bone_buffer_alloc(&mut self, bone_count: u32) -> u32 {
        let Some(ce) = self.client_engine.as_mut() else {
            return 0;
        };
        ce.bone_buffer_alloc(bone_count as usize).0
    }

    #[methoddb::bind(name = "free")]
    pub fn api_bone_buffer_free(&mut self, handle: u32) {
        if let Some(ce) = self.client_engine.as_mut() {
            ce.bone_buffer_free(deerpunk_engine_client::bone_buffer_pool::BoneBufferHandle(handle));
        }
    }

    #[methoddb::bind(name = "write")]
    pub fn api_bone_buffer_write(
        &mut self,
        handle: u32,
        matrices_ptr: *const f32,
        bone_count: u32,
    ) -> i32 {
        let Some(ce) = self.client_engine.as_mut() else {
            return 0;
        };
        if matrices_ptr.is_null() || bone_count == 0 {
            return 0;
        }
        let n = bone_count as usize;
        let floats = n * 16;
        if floats == 0 {
            return 0;
        }
        let slice = unsafe { std::slice::from_raw_parts(matrices_ptr, floats) };
        let mut mats = Vec::with_capacity(n);
        for i in 0..n {
            let off = i * 16;
            let cols: [f32; 16] = slice[off..off + 16].try_into().unwrap_or([0.0; 16]);
            let m = glam::Mat4::from_cols_array(&cols);
            mats.push(m);
        }
        if ce.bone_buffer_write(
            deerpunk_engine_client::bone_buffer_pool::BoneBufferHandle(handle),
            &mats,
        ) {
            1
        } else {
            0
        }
    }
}
