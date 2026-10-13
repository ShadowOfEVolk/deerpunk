//! MethodDB-generated API (`namespace` + `bind`) on host subsystems.

use deerpunk_macro as methoddb;

use std::ffi::c_void;

use deerpunk_engine_common::network::protocol::SVC;
use deerpunk_engine_common::network::sound_event::{SoundPlayRpc, SoundStopRpc};
use deerpunk_engine_common::network::{UserCmd, USERCMD_SIZE};

use crate::channel::ClientCmd;
use crate::move_state;
use crate::user_messages::UmHookFn;
use crate::physics_trace::PhysicsTraceResult;
use crate::engine_api::physics::CapsuleCastHitOut;
use deerpunk_engine_common::physics::ColliderPart;
use deerpunk_engine_api::components::ComponentRegisterDesc;
use deerpunk_engine_api::resources::ResourceLoaderDesc;
use crate::ScheduleSystemFn;

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
            "map" => {
                let name = parts.next().unwrap_or("").to_string();
                if name.is_empty() {
                    return -1;
                }
                self.channel.push_client(ClientCmd::Map(name));
                0
            }
            "disconnect" => {
                self.channel.push_client(ClientCmd::Disconnect);
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
        let packet = cmd.encode_clc_packet().to_vec();
        if self.net.is_client() {
            if !self.client_in_play() {
                return 0;
            }
            self.net.send_c2s(packet);
        } else {
            self.channel.push_client_bytes(packet);
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
        self.players.set_entity(slot, entity);
        0
    }

    #[methoddb::bind(name = "local_entity")]
    pub fn api_players_local_entity(&self) -> u64 {
        let Some(slot) = self.players.local_slot() else {
            return 0;
        };
        self.players
            .get(slot)
            .and_then(|p| p.entity)
            .unwrap_or(0)
    }

    #[methoddb::bind(name = "get_usercmd")]
    pub fn api_players_get_usercmd(&self, slot: u32, out: *mut u8, out_len: usize) -> i32 {
        if out.is_null() || out_len < USERCMD_SIZE {
            return -1;
        }
        let Some(cmd) = self.players.usercmd(slot) else {
            return -1;
        };
        let bytes = cmd.encode();
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, USERCMD_SIZE);
        }
        0
    }

    #[methoddb::bind(name = "apply_local_view")]
    pub fn api_players_apply_local_view(
        &mut self,
        yaw: f32,
        pitch: f32,
        roll: f32,
        eye_height: f32,
        fov_y: f32,
    ) -> i32 {
        self.apply_local_view(yaw, pitch, roll, eye_height, fov_y);
        0
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
        let st = move_state::global();
        let axes = st.axes();
        let n = axis_count.min(axes.len());
        unsafe {
            std::ptr::copy_nonoverlapping(axes.as_ptr(), axes_ptr, n);
            *buttons_ptr = st.buttons;
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
        self.server_engine.world.spawn()
    }

    #[methoddb::bind(name = "has")]
    pub fn api_world_has(&self, entity: u64, comp: u64) -> bool {
        self.server_engine.world.has(entity, comp)
    }

    #[methoddb::bind(name = "remove")]
    pub fn api_world_remove(&mut self, entity: u64, comp: u64) {
        self.server_engine.world.remove(entity, comp);
    }

    #[methoddb::bind(name = "get")]
    pub fn api_world_get(&self, entity: u64, comp: u64) -> *mut c_void {
        self.world_get_component_ptr(entity, comp)
    }

    #[methoddb::bind(name = "set")]
    pub fn api_world_set(
        &self,
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
        self.server_engine.world.destroy(entity);
    }

    #[methoddb::bind(name = "component_register")]
    pub fn api_world_component_register(&self, desc: &ComponentRegisterDesc) -> u64 {
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
        self.on_server_activated();
    }

    #[methoddb::bind(name = "is_enabled")]
    pub fn api_server_is_enabled(&self) -> i32 {
        if self.server_engine.enabled {
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
        self.server_engine.physics.create_shape(abi)
    }

    #[methoddb::bind(name = "destroy_shape")]
    pub fn api_physics_destroy_shape(&mut self, handle: u32) -> i32 {
        self.server_engine.physics.destroy_shape(handle);
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
        self.server_engine.physics.create_parts(parts)
    }

    #[methoddb::bind(name = "destroy_parts")]
    pub fn api_physics_destroy_parts(&mut self, handle: u32) -> i32 {
        self.server_engine.physics.destroy_parts(handle);
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
        match self.server_engine.physics.cast_capsule(
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

    #[methoddb::bind(name = "set_relative_mouse")]
    pub fn api_input_set_relative_mouse(&mut self, _on: bool) -> i32 {
        0
    }
}

#[methoddb::namespace(name = "v1:host", on_host, scope = ["server", "client"])]
impl crate::Host {
    #[methoddb::bind(name = "flush_pending_albedos")]
    pub fn api_host_flush_pending_albedos(&mut self) -> i32 {
        #[cfg(any(feature = "client", feature = "full"))]
        {
            self.flush_pending_albedos();
            0
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
            if let Some(ce) = self.client_engine.as_ref() {
                let gamedir = self.fs.gamedir().to_path_buf();
                self.client_rep.refresh_mesh_uri_overrides(
                    &ce.world,
                    &gamedir,
                    &mut self.resources,
                );
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
            self.client_rep.track_static_entity(entity);
            0
        }
        #[cfg(not(any(feature = "client", feature = "full")))]
        {
            let _ = entity;
            -1
        }
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
                self.server_rep.net_id_of(entity).unwrap_or(0)
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
            .map(|ce| ce.install_audio_world_mesh(entity))
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
        let qid = self.server_engine.world.query_prepare_ids(require);
        let mut ids = Vec::new();
        if let Some(batches) = self.server_engine.world.query_batches(qid) {
            for batch in batches {
                let entities =
                    unsafe { std::slice::from_raw_parts(batch.entities, batch.count) };
                ids.extend_from_slice(entities);
            }
        }
        self.server_engine.world.query_release(qid);
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
        let qid = self.server_engine.world.query_prepare_ids(require);
        let mut count = 0usize;
        if let Some(batches) = self.server_engine.world.query_batches(qid) {
            for batch in batches {
                count += batch.count;
            }
        }
        self.server_engine.world.query_release(qid);
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
            if !self.server_engine.world.has(entity, c) {
                return false;
            }
        }
        for &c in exclude {
            if self.server_engine.world.has(entity, c) {
                return false;
            }
        }
        true
    }

    #[methoddb::bind(name = "query_prepare")]
    pub fn api_world_query_prepare(&mut self, expr: &str) -> u32 {
        self.server_engine.world.query_prepare(expr) as u32
    }

    #[methoddb::bind(name = "query_execute")]
    pub fn api_world_query_execute(
        &self,
        query_id: u32,
        callback: crate::WorldQueryCallback,
        ctx: *mut c_void,
    ) {
        let qid = query_id as usize;
        if let Some(batches) = self.server_engine.world.query_batches(qid) {
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
        if let Some(ce) = self.client_engine.as_mut() {
            ce.world.set_raw(entity, comp, data, size);
        }
    }

    #[methoddb::bind(name = "has")]
    pub fn api_client_world_has(&self, entity: u64, comp: u64) -> bool {
        self.client_engine
            .as_ref()
            .map(|c| c.world.has(entity, comp))
            .unwrap_or(false)
    }

    #[methoddb::bind(name = "spawn")]
    pub fn api_client_world_spawn(&self) -> u64 {
        self.client_engine
            .as_ref()
            .map(|c| c.world.spawn())
            .unwrap_or(0)
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
