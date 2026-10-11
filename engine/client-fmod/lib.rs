//! FMOD Studio backend for the deerpunk [`AudioBackend`] trait.
//!
//! Ported (simplified) from darkdemo-lib `client-audio`: Studio events over
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
use libfmod::{Attributes3d, ChannelGroup, Dsp, EventInstance, Init, LoadBank, Studio, StudioInit, Vector};

use steam_audio::SteamAudio;
use steam_audio_bridge::{FmodSteamBridge, IPL_SPATIALIZE_APPLY_REFLECTIONS, IPL_SPATIALIZE_APPLY_PATHING, IPL_SPATIALIZE_SIMULATION_DEFINED, IPL_SPATIALIZE_SIMULATION_OUTPUTS_HANDLE};

/// HL world units → FMOD meters (same basis as darkdemo-lib).
const METERS_IN_UNIT: f32 = 1.0 / 40.0;
const OUTPUT_RATE: i32 = 44100;
const DSP_BLOCK: u32 = 512;
const DSP_NUM_BUFFERS: i32 = 4;
/// Built banks directory relative to gamedir.
const BANKS_SUBDIR: &str = "sound";
/// Native FMOD runtime DLLs (loaded from `{gamedir}/cl_dlls`).
const FMOD_DLLS: [&str; 2] = ["fmod.dll", "fmodstudio.dll"];
/// Steam Audio FMOD plugin (+ its phonon.dll dependency). Banks reference its
/// spatializer DSP — without it banks fail with FMOD error 54.
const PHONON_DLLS: [&str; 2] = ["phonon.dll", "phonon_fmod.dll"];
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
    /// HL world position → FMOD (HL is Z-up, FMOD is Y-up; same axis swap as darkdemo-lib).
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
            up: Self::hl_dir([
                -listener.up[0],
                -listener.up[1],
                -listener.up[2],
            ]),
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
        Self::dsp_uses_sim_defined_int(dsp, steam_audio_bridge::IPL_SPATIALIZE_APPLY_DISTANCEATTENUATION)
            || Self::dsp_uses_sim_defined_int(dsp, steam_audio_bridge::IPL_SPATIALIZE_APPLY_AIRABSORPTION)
            || Self::dsp_uses_sim_defined_int(dsp, steam_audio_bridge::IPL_SPATIALIZE_APPLY_DIRECTIVITY)
            || Self::dsp_uses_sim_defined_int(dsp, steam_audio_bridge::IPL_SPATIALIZE_APPLY_OCCLUSION)
            || Self::dsp_uses_sim_defined_int(dsp, steam_audio_bridge::IPL_SPATIALIZE_APPLY_TRANSMISSION)
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
                        let refl = dsp.set_parameter_bool(IPL_SPATIALIZE_APPLY_REFLECTIONS, enabled);
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

    /// Builds the phonon scene + probes from the client world mesh.
    /// Called by the client when a map is loaded.
    pub fn set_world_scene(&mut self, mesh: &deerpunk_engine_common::resource::mesh::Mesh, debug: &deerpunk_engine_common::bsp_debug::BspWorldDebug) {
        let Some(steam) = self.steam.as_ref() else {
            return;
        };
        let context = &steam.context;
        match scene::build_scene_from_mesh(context, mesh, debug) {
            Ok(world_scene) => {
                steam.simulation_manager.set_scene(&world_scene.scene);
                match scene::build_probe_batch(context, &world_scene.scene, world_scene.aabb) {
                    Ok(batch) => {
                        log::info!("[fmod] steam audio scene installed (probes={})", batch.num_probes());
                        steam.simulation_manager.set_probe_batch(batch);
                    }
                    Err(err) => log::warn!("[fmod] probe batch failed: {err}"),
                }
            }
            Err(err) => log::warn!("[fmod] steam audio scene build failed: {err}"),
        }
    }

    fn teardown_steam_entity(&self, key: u64) {
        if let Some(handle) = self.entity_sa_handles.borrow_mut().remove(&key) {
            if let Some(bridge) = self.steam_bridge.as_ref() {
                bridge.remove_source(handle);
            }
        }
        if self.steam_sim_edicts.borrow_mut().remove(&key) {
            if let Some(steam) = self.steam.as_ref() {
                let mut registry = steam.registry.borrow_mut();
                steam.simulation_manager.pop_entity(&mut registry, key as i32);
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

    /// Binds pending entity instances to phonon sources (runs after direct sim).
    fn flush_entity_steam_wires(&self) {
        let Some(steam) = self.steam.as_ref() else {
            self.pending_steam_wires.borrow_mut().clear();
            return;
        };
        let Some(bridge) = self.steam_bridge.as_ref() else {
            self.pending_steam_wires.borrow_mut().clear();
            return;
        };

        // Start deferred (paused) instances.
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
            } else {
                retry.push(key);
            }
        }
        self.pending_steam_wires.borrow_mut().extend(retry);

        // Enable reflections/pathing once worker outputs exist.
        let wired: Vec<u64> = self
            .entity_steam_wired
            .borrow()
            .iter()
            .copied()
            .filter(|k| !self.entity_indirect_sim.borrow().contains(k))
            .collect();
        for key in wired {
            if !steam
                .simulation_manager
                .indirect_sim_outputs_ready(&steam.registry.borrow(), key as i32)
            {
                continue;
            }
            let entities = self.entities.borrow();
            let Some(slot) = entities.get(&key) else {
                continue;
            };
            if slot.instance.is_valid() {
                let group_ok = slot
                    .instance
                    .get_channel_group()
                    .map(|g| Self::set_spatializer_indirect_on_group(&g, true))
                    .unwrap_or(false);
                if group_ok {
                    self.entity_indirect_sim.borrow_mut().insert(key);
                }
            }
        }
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

        let uses_indirect = Self::spatializer_uses_indirect(&spat);
        if uses_indirect
            && !steam
                .simulation_manager
                .indirect_sim_outputs_ready(&steam.registry.borrow(), entnum)
        {
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

        if uses_indirect {
            let _ = Self::set_spatializer_indirect_on_group(&group, true);
            self.entity_indirect_sim.borrow_mut().insert(key);
        }
        true
    }
}

impl AudioBackend for FmodBackend {
    fn init(&mut self, gamedir: &Path) -> bool {
        if !preload_native_dlls(gamedir) {
            return false;
        }
        // Preload phonon.dll so phonon_fmod.dll resolves its dependency, then
        // register the FMOD plugin. Optional: without it, non-plugin banks load.
        let cl = gamedir.join("cl_dlls");
        let _ = load_library(&cl.join("phonon.dll"));

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
        if let Err(err) = system.set_software_format(Some(OUTPUT_RATE), Some(libfmod::SpeakerMode::Stereo), None) {
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

        log::info!("[fmod] runtime ready (output: system default, banks: {}/{})", gamedir.display(), BANKS_SUBDIR);

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

        // Steam Audio: tick sim (direct) + wire pending instances.
        if let Some(steam) = self.steam.as_ref() {
            let view = ListenerView::from_audio_listener(listener);
            steam.update(&view);
            self.flush_entity_steam_wires();
            if let Some(bridge) = self.steam_bridge.as_ref() {
                bridge.sync_reverb_source(&steam.simulation_manager);
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

    fn play_event_2d(&mut self, event: &str, volume: f32, params: &[(&str, f32)]) {
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

    fn start_attached(&mut self, key: u64, event: &str, origin: [f32; 3], volume: f32) {
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
                self.entity_steam_wired.borrow_mut().remove(&key);
                self.entity_indirect_sim.borrow_mut().remove(&key);
                if !steam_wiring {
                    let _ = slot.instance.start();
                } else {
                    slot.deferred_start = true;
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
        if !steam_wiring {
            if let Err(err) = instance.start() {
                log::warn!("[fmod] start({path}): {err}");
                let _ = instance.release();
                return;
            }
        }
        entities.insert(
            key,
            EntitySound {
                path,
                instance,
                deferred_start: steam_wiring,
            },
        );
        if steam_wiring {
            self.pending_steam_wires.borrow_mut().push(key);
        }
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

/// LoadLibrary the FMOD runtime from `{gamedir}/cl_dlls` (absolute paths, no
/// process search-path changes). Must run before any libfmod call.
fn preload_native_dlls(gamedir: &Path) -> bool {
    #[cfg(windows)]
    {
        let dir = gamedir.join("cl_dlls");
        let mut ok = true;
        for name in FMOD_DLLS {
            let path = dir.join(name);
            if !path.is_file() {
                log::error!("[fmod] missing {name} — expected {}", path.display());
                ok = false;
                continue;
            }
            if load_library(&path) {
                log::info!("[fmod] loaded {}", path.display());
            } else {
                log::error!("[fmod] LoadLibrary failed for {}", path.display());
                ok = false;
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
        let Some(path_str) = path.to_str() else { continue };
        match studio.load_bank_file(path_str, LoadBank::NORMAL) {
            Ok(_) => log::info!("[fmod] loaded bank: {path_str}"),
            Err(err) => log::warn!("[fmod] load bank {path_str} failed: {err}"),
        }
    }
}
