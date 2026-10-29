//! FMOD Studio backend for the deerpunk [`AudioBackend`] trait.
//!
//! Ported (simplified) from game-lib `client-audio`: Studio events over
//! banks in `{gamedir}/sound`, HL-units → meters 3D mapping, entity-attached
//! looping instances, Steam Audio (phonon) spatialization + occlusion +
//! parametric reflections/pathing via the phonon_fmod plugin.

pub mod scene;
pub mod steam_audio;
pub mod steam_audio_bridge;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use deerpunk_engine_client::audio::{AttachedSounds, AudioBackend, AudioListener, FmodEventInfo};
use libfmod::{
    Attributes3d, ChannelGroup, Dsp, EventInstance, Init, LoadBank, Studio, StudioInit, Vector,
};

use steam_audio::SteamAudio;
use steam_audio_bridge::{
    FmodSteamBridge, IPL_SPATIALIZE_APPLY_PATHING, IPL_SPATIALIZE_APPLY_REFLECTIONS,
    IPL_SPATIALIZE_SIMULATION_DEFINED, IPL_SPATIALIZE_SIMULATION_OUTPUTS_HANDLE,
};

/// HL world units → FMOD meters (same basis as game-lib).
const METERS_IN_UNIT: f32 = 1.0 / 40.0;
const OUTPUT_RATE: i32 = 44100;
const DSP_BLOCK: u32 = 512;
const DSP_NUM_BUFFERS: i32 = 4;
/// Built banks directory relative to gamedir.
const BANKS_SUBDIR: &str = "sound";
/// Native FMOD runtime DLLs (loaded from `{gamedir}/cl_dlls`).
const FMOD_DLLS: [&str; 2] = ["fmod.dll", "fmodstudio.dll"];
/// Master Studio bus path (volume control).
const MASTER_BUS: &str = "bus:/";

/// Listener pose in HL units for the simulation side.
pub struct ListenerView {
    pub origin: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub forward: Vec3,
}

impl ListenerView {
    fn from_audio_listener(l: &AudioListener) -> Self {
        let forward = Vec3::from_array(l.forward);
        let up = Vec3::from_array(l.up);
        let right = forward.cross(up).normalize_or_zero();
        Self {
            origin: Vec3::from_array(l.position),
            right,
            up,
            forward,
        }
    }
}

use glam::Vec3;

struct EntitySound {
    path: String,
    instance: EventInstance,
    /// Steam-wired events start paused and are unpaused after the handle bind.
    deferred_start: bool,
}

pub struct FmodBackend {
    /// `Some` after successful [`AudioBackend::init`].
    studio: Option<Studio>,
    entities: RefCell<HashMap<u64, EntitySound>>,
    last_volume: f32,
    /// Steam Audio simulation (None if phonon init failed — events play dry).
    steam: Option<SteamAudio>,
    steam_bridge: Option<FmodSteamBridge>,
    entity_steam_wired: RefCell<HashSet<u64>>,
    entity_sa_handles: RefCell<HashMap<u64, i32>>,
    entity_indirect_sim: RefCell<HashSet<u64>>,
    steam_sim_edicts: RefCell<HashSet<u64>>,
    pending_steam_wires: RefCell<Vec<u64>>,
}

impl Default for FmodBackend {
    fn default() -> Self {
        Self {
            studio: None,
            entities: RefCell::new(HashMap::new()),
            last_volume: 1.0,
            steam: None,
            steam_bridge: None,
            entity_steam_wired: RefCell::new(HashSet::new()),
            entity_sa_handles: RefCell::new(HashMap::new()),
            entity_indirect_sim: RefCell::new(HashSet::new()),
            steam_sim_edicts: RefCell::new(HashSet::new()),
            pending_steam_wires: RefCell::new(Vec::new()),
        }
    }
}

impl FmodBackend {
    /// HL world position → FMOD (HL is Z-up, FMOD is Y-up; same axis swap as game-lib).
    fn hl_pos(origin: [f32; 3]) -> Vector {
        Vector::new(
            origin[0] * METERS_IN_UNIT,
            origin[2] * METERS_IN_UNIT,
            -origin[1] * METERS_IN_UNIT,
        )
    }

    fn hl_dir(dir: [f32; 3]) -> Vector {
        Vector::new(dir[0], dir[2], -dir[1])
    }

    fn listener_attrs(listener: &AudioListener) -> Attributes3d {
        Attributes3d {
            position: Self::hl_pos(listener.position),
            velocity: Vector::new(0.0, 0.0, 0.0),
            forward: Self::hl_dir(listener.forward),
            // HL left-handed: forward×up = −right; FMOD derives right from forward×up.
            up: Self::hl_dir([-listener.up[0], -listener.up[1], -listener.up[2]]),
        }
    }

    fn attrs(origin: [f32; 3]) -> Attributes3d {
        Attributes3d {
            position: Self::hl_pos(origin),
            velocity: Vector::new(0.0, 0.0, 0.0),
            forward: Vector::new(0.0, 1.0, 0.0),
            up: Vector::new(0.0, 0.0, 1.0),
        }
    }

    fn event_path(event: &str) -> String {
        if event.starts_with("event:/") {
            event.to_string()
        } else {
            format!("event:/{event}")
        }
    }

    fn create_instance(studio: &Studio, path: &str) -> Option<EventInstance> {
        let description = match studio.get_event(path) {
            Ok(d) => d,
            Err(err) => {
                log::warn!("[fmod] get_event({path}): {err}");
                return None;
            }
        };
        match description.create_instance() {
            Ok(i) => Some(i),
            Err(err) => {
                log::warn!("[fmod] create_instance({path}): {err}");
                None
            }
        }
    }

    fn is_steam_spatializer(dsp: &Dsp) -> bool {
        matches!(dsp.get_info(), Ok((name, ..)) if name == "Steam Audio Spatializer")
    }

    fn find_spatializer_dsp(group: &ChannelGroup) -> Option<Dsp> {
        if let Ok(num) = group.get_num_ds_ps() {
            for i in 0..num {
                if let Ok(dsp) = group.get_dsp(i) {
                    if Self::is_steam_spatializer(&dsp) {
                        return Some(dsp);
                    }
                }
            }
        }
        if let Ok(num_groups) = group.get_num_groups() {
            for i in 0..num_groups {
                if let Ok(sub) = group.get_group(i) {
                    if let Some(dsp) = Self::find_spatializer_dsp(&sub) {
                        return Some(dsp);
                    }
                }
            }
        }
        None
    }

    fn dsp_uses_sim_defined_int(dsp: &Dsp, param: i32) -> bool {
        dsp.get_parameter_int(param, 256)
            .map(|(value, _)| value == IPL_SPATIALIZE_SIMULATION_DEFINED)
            .unwrap_or(false)
    }

    fn dsp_uses_sim_defined_bool(dsp: &Dsp, param: i32) -> bool {
        dsp.get_parameter_bool(param, 256)
            .map(|(value, _)| value)
            .unwrap_or(false)
    }

    /// True when the spatializer consumes simulation outputs (occlusion etc.).
    fn spatializer_needs_phonon_bind(dsp: &Dsp) -> bool {
        Self::dsp_uses_sim_defined_int(
            dsp,
            steam_audio_bridge::IPL_SPATIALIZE_APPLY_DISTANCEATTENUATION,
        ) || Self::dsp_uses_sim_defined_int(
            dsp,
            steam_audio_bridge::IPL_SPATIALIZE_APPLY_AIRABSORPTION,
        ) || Self::dsp_uses_sim_defined_int(
            dsp,
            steam_audio_bridge::IPL_SPATIALIZE_APPLY_DIRECTIVITY,
        ) || Self::dsp_uses_sim_defined_int(dsp, steam_audio_bridge::IPL_SPATIALIZE_APPLY_OCCLUSION)
            || Self::dsp_uses_sim_defined_int(
                dsp,
                steam_audio_bridge::IPL_SPATIALIZE_APPLY_TRANSMISSION,
            )
            || Self::dsp_uses_sim_defined_bool(dsp, IPL_SPATIALIZE_APPLY_REFLECTIONS)
            || Self::dsp_uses_sim_defined_bool(dsp, IPL_SPATIALIZE_APPLY_PATHING)
    }

    fn spatializer_uses_indirect(dsp: &Dsp) -> bool {
        Self::dsp_uses_sim_defined_bool(dsp, IPL_SPATIALIZE_APPLY_REFLECTIONS)
            || Self::dsp_uses_sim_defined_bool(dsp, IPL_SPATIALIZE_APPLY_PATHING)
    }

    fn apply_spatializer_handle_to_group(group: &ChannelGroup, handle: i32) -> bool {
        let mut applied = false;
        if let Ok(num) = group.get_num_ds_ps() {
            for i in 0..num {
                if let Ok(dsp) = group.get_dsp(i) {
                    if Self::is_steam_spatializer(&dsp)
                        && dsp
                            .set_parameter_int(IPL_SPATIALIZE_SIMULATION_OUTPUTS_HANDLE, handle)
                            .is_ok()
                    {
                        // Reflections/pathing need worker outputs; phonon_fmod
                        // crashes on null IR/sh coeffs — enable after they exist.
                        let _ = dsp.set_parameter_bool(IPL_SPATIALIZE_APPLY_REFLECTIONS, false);
                        let _ = dsp.set_parameter_bool(IPL_SPATIALIZE_APPLY_PATHING, false);
                        applied = true;
                    }
                }
            }
        }
        if let Ok(num_groups) = group.get_num_groups() {
            for i in 0..num_groups {
                if let Ok(sub) = group.get_group(i) {
                    applied |= Self::apply_spatializer_handle_to_group(&sub, handle);
                }
            }
        }
        applied
    }

    fn set_spatializer_indirect_on_group(group: &ChannelGroup, enabled: bool) -> bool {
        let mut applied = false;
        if let Ok(num) = group.get_num_ds_ps() {
            for i in 0..num {
                if let Ok(dsp) = group.get_dsp(i) {
                    if Self::is_steam_spatializer(&dsp) {
                        let refl =
                            dsp.set_parameter_bool(IPL_SPATIALIZE_APPLY_REFLECTIONS, enabled);
                        let path = dsp.set_parameter_bool(IPL_SPATIALIZE_APPLY_PATHING, enabled);
                        applied |= refl.is_ok() || path.is_ok();
                    }
                }
            }
        }
        if let Ok(num_groups) = group.get_num_groups() {
            for i in 0..num_groups {
                if let Ok(sub) = group.get_group(i) {
                    applied |= Self::set_spatializer_indirect_on_group(&sub, enabled);
                }
            }
        }
        applied
    }

    fn stop_entity(key: u64, entities: &mut HashMap<u64, EntitySound>) {
        if let Some(slot) = entities.remove(&key) {
            let _ = slot.instance.stop(libfmod::StopMode::AllowFadeout);
            let _ = slot.instance.release();
        }
    }

    fn set_master_volume(studio: &Studio, volume: f32) {
        if let Ok(bus) = studio.get_bus(MASTER_BUS) {
            if let Err(err) = bus.set_volume(volume) {
                log::warn!("[fmod] set_master_volume({volume}): {err}");
            }
        }
    }

    // ── Steam Audio wiring ─────────────────────────────────────

    fn teardown_steam_entity(&self, key: u64) {
        if let Some(handle) = self.entity_sa_handles.borrow_mut().remove(&key) {
            if let Some(bridge) = self.steam_bridge.as_ref() {
                bridge.remove_source(handle);
            }
        }
        if self.steam_sim_edicts.borrow_mut().remove(&key) {
            if let Some(steam) = self.steam.as_ref() {
                let mut registry = steam.registry.borrow_mut();
                steam
                    .simulation_manager
                    .pop_entity(&mut registry, key as i32);
            }
        }
        self.entity_steam_wired.borrow_mut().remove(&key);
        self.entity_indirect_sim.borrow_mut().remove(&key);
        self.pending_steam_wires.borrow_mut().retain(|k| *k != key);
    }

    /// Registers/updates the sim source for a wired entity.
    fn register_sim_source(&self, key: u64, origin: [f32; 3]) {
        let Some(steam) = self.steam.as_ref() else {
            return;
        };
        let mut registry = steam.registry.borrow_mut();
        let _ = steam.simulation_manager.upsert_entity(
            &mut registry,
            key as i32,
            Vec3::from_array(origin),
            1.0,
        );
        self.steam_sim_edicts.borrow_mut().insert(key);
    }

    /// Pause every attached instance (Spatializer DSP idle for SetScene).
    fn pause_all_entities(&self) {
        for slot in self.entities.borrow().values() {
            let _ = slot.instance.set_paused(true);
        }
    }

    /// Drop phonon source binds without stopping FMOD instances (scene swap).
    fn detach_all_steam_wires(&self) {
        let handles: Vec<i32> = self
            .entity_sa_handles
            .borrow_mut()
            .drain()
            .map(|(_, h)| h)
            .collect();
        if let Some(bridge) = self.steam_bridge.as_ref() {
            for h in handles {
                bridge.remove_source(h);
            }
        }
        if let Some(steam) = self.steam.as_ref() {
            let edicts: Vec<u64> = self.steam_sim_edicts.borrow_mut().drain().collect();
            let mut registry = steam.registry.borrow_mut();
            for key in edicts {
                steam
                    .simulation_manager
                    .pop_entity(&mut registry, key as i32);
            }
        }
        self.entity_steam_wired.borrow_mut().clear();
        self.entity_indirect_sim.borrow_mut().clear();
        let keys: Vec<u64> = self.entities.borrow().keys().copied().collect();
        self.pending_steam_wires.borrow_mut().clear();
        self.pending_steam_wires.borrow_mut().extend(keys);
    }

    /// Binds pending entity instances to phonon sources (after RunDirect).
    fn flush_entity_steam_wires(&self) {
        let Some(steam) = self.steam.as_ref() else {
            self.pending_steam_wires.borrow_mut().clear();
            return;
        };
        let Some(bridge) = self.steam_bridge.as_ref() else {
            self.pending_steam_wires.borrow_mut().clear();
            return;
        };

        // No BSP scene / still cooling down — keep pending (stay paused).
        if !steam.simulation_manager.sim_ready() {
            return;
        }

        // Start deferred (paused) instances, then wire.
        {
            let mut entities = self.entities.borrow_mut();
            for slot in entities.values_mut() {
                if slot.deferred_start {
                    let _ = slot.instance.set_paused(true);
                    if let Err(err) = slot.instance.start() {
                        log::warn!("[fmod] deferred start({}): {err}", slot.path);
                    }
                    slot.deferred_start = false;
                }
            }
        }

        let pending: Vec<u64> = self.pending_steam_wires.borrow_mut().drain(..).collect();
        let mut retry = Vec::new();
        for key in pending {
            let wired = self.wire_entity(key, steam, bridge);
            if wired {
                self.entity_steam_wired.borrow_mut().insert(key);
                if let Some(slot) = self.entities.borrow().get(&key) {
                    let _ = slot.instance.set_paused(false);
                }
            } else {
                retry.push(key);
            }
        }
        self.pending_steam_wires.borrow_mut().extend(retry);

        // NOTE: do NOT enable APPLY_REFLECTIONS / APPLY_PATHING — needs bake.
    }

    /// Returns true when the event is ready to play (handle bound, or no sim needed).
    fn wire_entity(&self, key: u64, steam: &SteamAudio, bridge: &FmodSteamBridge) -> bool {
        let entities = self.entities.borrow();
        let Some(slot) = entities.get(&key) else {
            return true;
        };
        let Ok(group) = slot.instance.get_channel_group() else {
            return false;
        };

        let Some(spat) = Self::find_spatializer_dsp(&group) else {
            return true;
        };
        if !Self::spatializer_needs_phonon_bind(&spat) {
            return true;
        }

        let entnum = key as i32;
        if !steam.registry.borrow().contains(entnum) {
            let origin = slot
                .instance
                .get_3d_attributes()
                .map(|attrs| {
                    [
                        attrs.position.x / METERS_IN_UNIT,
                        -attrs.position.z / METERS_IN_UNIT,
                        attrs.position.y / METERS_IN_UNIT,
                    ]
                })
                .unwrap_or([0.0; 3]);
            self.register_sim_source(key, origin);
            return false;
        }

        let source = steam.registry.borrow().source_for(entnum).cloned();
        let Some(source) = source else {
            return false;
        };
        let handle = match self.entity_sa_handles.borrow().get(&key).copied() {
            Some(h) => h,
            None => match bridge.add_source(&source) {
                Some(h) => {
                    self.entity_sa_handles.borrow_mut().insert(key, h);
                    h
                }
                None => return false,
            },
        };

        if !Self::apply_spatializer_handle_to_group(&group, handle) {
            return false;
        }
        // Keep reflections/pathing off on the DSP (see flush_entity_steam_wires).
        let _ = Self::set_spatializer_indirect_on_group(&group, false);
        true
    }
}

impl AudioBackend for FmodBackend {
    fn init(&mut self, ctx: &deerpunk_engine_client::audio::AudioInitContext) -> bool {
        let gamedir = ctx.gamedir;
        if !preload_native_dlls(gamedir) {
            return false;
        }
        let cl = gamedir.join("cl_dlls");
        // phonon.dll: do NOT LoadLibrary it. deerpunk.exe already imports it via
        // audionimbus-sys IAT; a second LoadLibrary(absolute cl_dlls path) maps
        // another copy → STATUS_HEAP_CORRUPTION.

        let studio = match Studio::create() {
            Ok(s) => s,
            Err(err) => {
                log::warn!("[fmod] Studio::create failed: {err}");
                return false;
            }
        };

        let system = match studio.get_core_system() {
            Ok(s) => s,
            Err(err) => {
                log::warn!("[fmod] get_core_system failed: {err}");
                let _ = studio.release();
                return false;
            }
        };

        // Steam Audio plugin: banks reference its spatializer DSP (FMOD error 54
        // without it). Plugin handles live in the bank; simulation is wired later.
        {
            let plugin = cl.join("phonon_fmod.dll");
            match system.load_plugin(plugin.to_string_lossy().as_ref(), None) {
                Ok(_) => log::info!("[fmod] Steam Audio plugin loaded ({})", plugin.display()),
                Err(err) => log::warn!(
                    "[fmod] phonon_fmod load_plugin failed: {err} — plugin banks will not load"
                ),
            }
        }

        // Direct output — deerpunk owns no legacy mixer (unlike xash DMA path).
        if let Err(err) =
            system.set_software_format(Some(OUTPUT_RATE), Some(libfmod::SpeakerMode::Stereo), None)
        {
            log::warn!("[fmod] set_software_format failed: {err}");
            let _ = studio.release();
            return false;
        }
        if let Err(err) = system.set_dsp_buffer_size(DSP_BLOCK, DSP_NUM_BUFFERS) {
            log::warn!("[fmod] set_dsp_buffer_size failed: {err}");
            let _ = studio.release();
            return false;
        }

        let studio_flags = StudioInit::NORMAL | StudioInit::LIVEUPDATE;
        let core_flags = Init::NORMAL;
        if let Err(err) = studio.initialize(DSP_BLOCK as i32, studio_flags, core_flags, None) {
            log::warn!("[fmod] Studio::initialize failed: {err}");
            let _ = studio.release();
            return false;
        }

        load_banks(&studio, gamedir);

        // Steam Audio core + FMOD bridge (iplFMODInitialize etc.).
        match SteamAudio::new() {
            Ok(steam) => {
                let bridge = FmodSteamBridge::new_ex(&system, &cl, true);
                bridge.link(&steam);
                self.steam = Some(steam);
                self.steam_bridge = Some(bridge);
            }
            Err(err) => log::warn!("[fmod] Steam Audio init failed: {err} — events play dry"),
        }

        for _ in 0..8 {
            let _ = studio.update();
        }

        log::info!(
            "[fmod] runtime ready (output: system default, banks: {}/{})",
            gamedir.display(),
            BANKS_SUBDIR
        );

        self.studio = Some(studio);
        self.entities = RefCell::new(HashMap::new());
        self.last_volume = 1.0;
        true
    }

    fn update(&mut self, listener: &AudioListener, attached: AttachedSounds, volume: f32) {
        let Some(studio) = self.studio.as_ref() else {
            return;
        };
        if let Err(err) = studio.set_listener_attributes(0, Self::listener_attrs(listener), None) {
            log::warn!("[fmod] set_listener_attributes: {err}");
        }

        // Refresh attached instance positions from the world.
        {
            let entities = self.entities.borrow();
            for (key, origin) in attached {
                if let Some(slot) = entities.get(key) {
                    let _ = slot.instance.set_3d_attributes(Self::attrs(*origin));
                }
            }
        }

        // Steam Audio: DIRECT sim + source wires. Realtime RunReflections races
        // phonon_fmod on the shared Context (0374 after resume) — deferred.
        if let Some(steam) = self.steam.as_ref() {
            let view = ListenerView::from_audio_listener(listener);
            steam.update(&view);
            if steam.simulation_manager.sim_ready() {
                self.flush_entity_steam_wires();
            }
        }

        if (self.last_volume - volume).abs() > f32::EPSILON {
            self.last_volume = volume;
            Self::set_master_volume(studio, volume);
        }

        if let Err(err) = studio.update() {
            log::warn!("[fmod] studio.update: {err}");
        }
    }

    fn set_world_scene(&mut self, mesh: &deerpunk_engine_common::resource::mesh::Mesh) {
        // TEMP: BSP Simulator::SetScene heap-corrupts (0xC0000374) with phonon_fmod.
        // Skip scene/occlusion/reflections; HRTF spatialize still works via bridge.
        const ENABLE_STEAM_SCENE_SIM: bool = false;
        if !ENABLE_STEAM_SCENE_SIM {
            let tris = mesh.indices.len() / 3;
            let verts = mesh.positions.len();
            log::info!(
                "[fmod] set_world_scene SKIPPED (sim off; HRTF only) verts={verts} tris={tris}",
            );
            return;
        }

        let Some(steam) = self.steam.as_ref() else {
            log::warn!("[fmod] set_world_scene: Steam Audio not init — BSP scene skipped");
            return;
        };
        let tris = mesh.indices.len() / 3;
        let verts = mesh.positions.len();

        // Phonon Context is shared with phonon_fmod on the mixer thread.
        // Order matches Unity/SDK: pause → flush → suspend mixer → detach
        // wires / SetScene / Commit → resume. Detach must NOT run while the
        // mixer can still process Spatializer DSPs.
        steam.simulation_manager.set_enabled(false);
        self.pause_all_entities();
        if let Some(studio) = self.studio.as_ref() {
            if let Err(err) = studio.update() {
                log::warn!("[fmod] studio.update before set_scene: {err}");
            }
        }

        let core = self.studio.as_ref().and_then(|s| s.get_core_system().ok());
        if let Some(sys) = core.as_ref() {
            if let Err(err) = sys.mixer_suspend() {
                log::warn!("[fmod] mixer_suspend before set_scene: {err}");
            } else {
                log::info!("[fmod] mixer_suspend ok (before detach/set_scene)");
            }
        }

        self.detach_all_steam_wires();

        // Plugin must not hold the Context across Simulator SetScene (heap
        // corruption / VERIFIER 0xF). Unity-style: detach → Terminate →
        // SetScene+Commit → Initialize again → resume mixer.
        if let Some(bridge) = self.steam_bridge.as_ref() {
            bridge.unlink();
        }

        let context = &steam.context;
        let result = scene::build_scene_from_mesh(context, mesh);
        let build_ok = match result {
            Ok(world_scene) => {
                let scene::WorldScene {
                    scene,
                    static_meshes,
                    aabb: _,
                } = world_scene;
                steam.simulation_manager.set_scene(scene, static_meshes);
                log::info!(
                    "[fmod] steam scene installed (has_scene={}, verts={}, tris={}, mode=DIRECT-only)",
                    steam.simulation_manager.has_scene(),
                    verts,
                    tris,
                );
                true
            }
            Err(err) => {
                log::warn!("[fmod] steam scene build failed: {err} (verts={verts}, tris={tris})");
                false
            }
        };

        if let Some(bridge) = self.steam_bridge.as_ref() {
            bridge.link(steam);
        }

        log::info!("[fmod] mixer_resume before (build_ok={build_ok})");
        if let Some(sys) = core.as_ref() {
            if let Err(err) = sys.mixer_resume() {
                log::warn!("[fmod] mixer_resume after set_scene: {err}");
            } else {
                log::info!("[fmod] mixer_resume ok");
            }
        }
        if !build_ok {
            for slot in self.entities.borrow().values() {
                let _ = slot.instance.set_paused(false);
            }
        }
    }

    fn play_event_2d(
        &mut self,
        _assets: &deerpunk_engine_common::resource::ResourceManager,
        event: &str,
        volume: f32,
        params: &[(&str, f32)],
        _instance_id: u32,
    ) {
        let Some(studio) = self.studio.as_ref() else {
            return;
        };
        let path = Self::event_path(event);
        let Some(instance) = Self::create_instance(studio, &path) else {
            return;
        };
        let _ = instance.set_volume(volume.clamp(0.0, 1.0));
        for (name, value) in params {
            let _ = instance.set_parameter_by_name(name, *value, false);
        }
        if let Err(err) = instance.start() {
            log::warn!("[fmod] start({path}): {err}");
        }
        let _ = instance.release();
    }

    fn play_event_3d(
        &mut self,
        _assets: &deerpunk_engine_common::resource::ResourceManager,
        event: &str,
        origin: [f32; 3],
        volume: f32,
        params: &[(&str, f32)],
    ) {
        let Some(studio) = self.studio.as_ref() else {
            return;
        };
        let path = Self::event_path(event);
        let Some(instance) = Self::create_instance(studio, &path) else {
            return;
        };
        let _ = instance.set_3d_attributes(Self::attrs(origin));
        let _ = instance.set_volume(volume.clamp(0.0, 1.0));
        for (name, value) in params {
            let _ = instance.set_parameter_by_name(name, *value, false);
        }
        if let Err(err) = instance.start() {
            log::warn!("[fmod] start({path}): {err}");
        }
        let _ = instance.release();
    }

    fn start_attached(
        &mut self,
        _assets: &deerpunk_engine_common::resource::ResourceManager,
        key: u64,
        event: &str,
        origin: [f32; 3],
        volume: f32,
    ) {
        let Some(studio) = self.studio.as_ref() else {
            return;
        };
        let path = Self::event_path(event);
        let steam_wiring = self.steam.is_some();
        let mut entities = self.entities.borrow_mut();

        if let Some(slot) = entities.get_mut(&key) {
            if slot.path == path {
                let _ = slot.instance.set_3d_attributes(Self::attrs(origin));
                let _ = slot.instance.set_volume(volume.clamp(0.0, 1.0));
                let wait_scene = steam_wiring
                    && self
                        .steam
                        .as_ref()
                        .map(|s| !s.simulation_manager.sim_ready())
                        .unwrap_or(false);
                if wait_scene {
                    let _ = slot.instance.set_paused(true);
                }
                if let Err(err) = slot.instance.start() {
                    log::warn!("[fmod] restart attached({path}): {err}");
                }
                if steam_wiring {
                    self.entity_steam_wired.borrow_mut().remove(&key);
                    self.entity_indirect_sim.borrow_mut().remove(&key);
                    self.pending_steam_wires.borrow_mut().push(key);
                }
                return;
            }
            drop(entities);
            self.teardown_steam_entity(key);
            Self::stop_entity(key, &mut self.entities.borrow_mut());
            entities = self.entities.borrow_mut();
        }

        let Some(instance) = Self::create_instance(studio, &path) else {
            return;
        };
        if let Err(err) = instance.set_3d_attributes(Self::attrs(origin)) {
            log::warn!("[fmod] set_3d_attributes({path}): {err}");
            let _ = instance.release();
            return;
        }
        let _ = instance.set_volume(volume.clamp(0.0, 1.0));
        // If scene not ready yet, start paused — flush unpauses after DIRECT wire.
        // If scene ready, start audible; wire binds handle next sim_ready flush.
        let wait_scene = steam_wiring
            && self
                .steam
                .as_ref()
                .map(|s| !s.simulation_manager.sim_ready())
                .unwrap_or(false);
        if wait_scene {
            let _ = instance.set_paused(true);
        }
        if let Err(err) = instance.start() {
            log::warn!("[fmod] start_attached({path}): {err}");
            let _ = instance.release();
            return;
        }
        entities.insert(
            key,
            EntitySound {
                path: path.clone(),
                instance,
                deferred_start: false,
            },
        );
        if steam_wiring {
            self.pending_steam_wires.borrow_mut().push(key);
        }
        log::info!("[fmod] start_attached key={key:#x} '{path}' wait_scene={wait_scene}");
    }

    fn stop_attached(&mut self, key: u64) {
        self.teardown_steam_entity(key);
        Self::stop_entity(key, &mut self.entities.borrow_mut());
    }

    fn shutdown(&mut self) {
        let handles: Vec<i32> = self.entity_sa_handles.borrow().values().copied().collect();
        if let Some(bridge) = self.steam_bridge.as_mut() {
            bridge.shutdown(&handles);
        }
        self.entity_sa_handles.borrow_mut().clear();
        self.steam_sim_edicts.borrow_mut().clear();
        self.pending_steam_wires.borrow_mut().clear();
        self.entity_steam_wired.borrow_mut().clear();
        self.entity_indirect_sim.borrow_mut().clear();
        for (_, slot) in self.entities.borrow_mut().drain() {
            let _ = slot.instance.stop(libfmod::StopMode::Immediate);
            let _ = slot.instance.release();
        }
        self.steam = None;
        if let Some(studio) = self.studio.take() {
            let _ = studio.release();
        }
    }

    fn list_events(&self) -> Vec<FmodEventInfo> {
        let Some(studio) = self.studio.as_ref() else {
            return Vec::new();
        };
        let Ok(bank_count) = studio.get_bank_count() else {
            return Vec::new();
        };
        if bank_count <= 0 {
            return Vec::new();
        }
        let Ok(banks) = studio.get_bank_list(bank_count) else {
            return Vec::new();
        };

        let mut out: Vec<FmodEventInfo> = Vec::new();
        let mut seen = std::collections::HashSet::<String>::new();

        for bank in banks {
            let bank_path = bank.get_path().unwrap_or_else(|_| "bank:/?".into());
            // Strings bank has no playable events.
            if bank_path.contains(".strings") || bank_path.ends_with("Master.strings") {
                continue;
            }
            let Ok(ev_count) = bank.get_event_count() else {
                continue;
            };
            if ev_count <= 0 {
                continue;
            }
            let Ok(events) = bank.get_event_list(ev_count) else {
                continue;
            };
            for desc in events {
                let Ok(path) = desc.get_path() else {
                    continue;
                };
                if !seen.insert(path.clone()) {
                    continue;
                }
                let is_3d = desc.is_3d().unwrap_or(false);
                let oneshot = desc.is_oneshot().unwrap_or(false);
                let mut params = Vec::new();
                if let Ok(n) = desc.get_parameter_description_count() {
                    for i in 0..n {
                        if let Ok(p) = desc.get_parameter_description_by_index(i) {
                            if !p.name.is_empty() {
                                params.push(p.name);
                            }
                        }
                    }
                }
                out.push(FmodEventInfo {
                    path,
                    bank: bank_path.clone(),
                    is_3d,
                    oneshot,
                    params,
                });
            }
        }
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }
}

/// LoadLibrary FMOD runtime from `{gamedir}/cl_dlls` before any libfmod call.
/// Pins DLL search dir so `phonon_fmod` resolves `phonon.dll` by basename to the
/// already-IAT-loaded instance (never LoadLibrary phonon by absolute path).
fn preload_native_dlls(gamedir: &Path) -> bool {
    #[cfg(windows)]
    {
        let dir = gamedir.join("cl_dlls");
        set_dll_directory(Some(&dir));
        let mut ok = true;
        for name in FMOD_DLLS {
            let path = dir.join(name);
            if !path.is_file() {
                log::error!("[fmod] missing {name} — expected {}", path.display());
                ok = false;
                continue;
            }
            match load_library_once(&path, name) {
                Ok(true) => log::info!("[fmod] loaded {}", path.display()),
                Ok(false) => {
                    if let Some(have) = module_file_name(name) {
                        log::info!("[fmod] {name} already loaded ({})", have.display());
                    } else {
                        log::info!("[fmod] {name} already loaded");
                    }
                }
                Err(()) => {
                    log::error!("[fmod] LoadLibrary failed for {}", path.display());
                    ok = false;
                }
            }
        }
        ok
    }
    #[cfg(not(windows))]
    {
        let _ = gamedir;
        true
    }
}

#[cfg(windows)]
fn set_dll_directory(dir: Option<&Path>) {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn SetDllDirectoryW(path: *const u16) -> i32;
    }
    match dir {
        Some(p) => {
            let wide: Vec<u16> = p.as_os_str().encode_wide().chain([0]).collect();
            if unsafe { SetDllDirectoryW(wide.as_ptr()) } == 0 {
                log::warn!("[fmod] SetDllDirectory({}) failed", p.display());
            } else {
                log::info!("[fmod] SetDllDirectory({})", p.display());
            }
        }
        None => {
            let _ = unsafe { SetDllDirectoryW(std::ptr::null()) };
        }
    }
}

#[cfg(windows)]
fn module_handle(name: &str) -> Option<*mut core::ffi::c_void> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut core::ffi::c_void;
    }
    let wide: Vec<u16> = std::ffi::OsStr::new(name)
        .encode_wide()
        .chain([0])
        .collect();
    let h = unsafe { GetModuleHandleW(wide.as_ptr()) };
    if h.is_null() {
        None
    } else {
        Some(h)
    }
}

#[cfg(windows)]
fn module_file_name(name: &str) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleFileNameW(module: *mut core::ffi::c_void, buf: *mut u16, size: u32) -> u32;
    }
    let h = module_handle(name)?;
    let mut buf = vec![0u16; 512];
    let n = unsafe { GetModuleFileNameW(h, buf.as_mut_ptr(), buf.len() as u32) };
    if n == 0 {
        return None;
    }
    Some(PathBuf::from(std::ffi::OsString::from_wide(
        &buf[..n as usize],
    )))
}

/// Load `path` only if `module_name` is not already mapped. `Ok(true)` = newly
/// loaded, `Ok(false)` = already present, `Err` = LoadLibrary failed.
#[cfg(windows)]
fn load_library_once(path: &Path, module_name: &str) -> Result<bool, ()> {
    if module_handle(module_name).is_some() {
        return Ok(false);
    }
    if load_library(path) {
        Ok(true)
    } else {
        Err(())
    }
}

#[cfg(windows)]
fn load_library(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    extern "system" {
        fn LoadLibraryW(name: *const u16) -> *mut core::ffi::c_void;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    unsafe { !LoadLibraryW(wide.as_ptr()).is_null() }
}

/// Load every `*.bank` in `{gamedir}/{BANKS_SUBDIR}` — `*.strings.bank` first.
fn load_banks(studio: &Studio, gamedir: &Path) {
    let dir = gamedir.join(BANKS_SUBDIR);
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(err) => {
            log::warn!("[fmod] banks dir missing: {} ({err})", dir.display());
            return;
        }
    };

    let mut strings_banks: Vec<PathBuf> = Vec::new();
    let mut banks: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "bank") {
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(".strings.bank"))
            {
                strings_banks.push(path);
            } else {
                banks.push(path);
            }
        }
    }
    banks.sort();

    for path in strings_banks.iter().chain(banks.iter()) {
        let Some(path_str) = path.to_str() else {
            continue;
        };
        match studio.load_bank_file(path_str, LoadBank::NORMAL) {
            Ok(_) => log::info!("[fmod] loaded bank: {path_str}"),
            Err(err) => log::warn!("[fmod] load bank {path_str} failed: {err}"),
        }
    }
}
