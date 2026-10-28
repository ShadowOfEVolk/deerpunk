//! Simulation manager: DIRECT occlusion/attenuation on the frame tick.
//!
//! Realtime parametric REFLECTIONS (`RunReflections` + listener reverb source)
//! heap-corrupts when phonon_fmod holds the same Context (even with
//! Terminate/SetScene/Init). Keep DIRECT stable; bus SA Reverb needs a later
//! approach (bake and/or stricter Context ownership).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use audionimbus::{
    CoordinateSystem, Direct, DirectSimulationSettings, SimulationFlags,
    SimulationSharedInputs, Simulator,
};
use glam::Vec3;

use super::AMBISONICS_ORDER;
use super::registry::SimRegistry;
use crate::ListenerView;

pub fn listener_to_coords(origin: Vec3, right: Vec3, up: Vec3, forward: Vec3) -> CoordinateSystem {
    SimRegistry::listener_to_coords(origin, right, up, forward)
}

fn vec_to_listener_coords(origin: Vec3) -> CoordinateSystem {
    SimRegistry::listener_to_coords(origin, Vec3::X, Vec3::Z, Vec3::Y)
}

/// Internal simulation state. Phonon Context is not thread-safe vs FMOD mixer.
pub(crate) struct SimulationInner {
    simulator: Simulator<Direct, (), ()>,
    /// Kept alive: `iplSimulatorSetScene` does not retain; Drop would UAF.
    scene: Option<audionimbus::Scene>,
    /// Kept alive: `add_static_mesh` may not keep our Rust owner.
    static_meshes: Vec<audionimbus::StaticMesh>,
    has_scene: bool,
    /// Frames to skip `run_direct` after scene swap (phonon settles).
    sim_cooldown: u32,
    listener_coords: CoordinateSystem,
    dirty: AtomicBool,
    scene_dirty: AtomicBool,
}

impl SimulationInner {
    fn push_entity(
        &mut self,
        registry: &mut SimRegistry,
        key: i32,
        origin: Vec3,
        dist_mult: f32,
    ) -> Result<(), audionimbus::SteamAudioError> {
        registry.push(
            &mut self.simulator,
            key,
            origin,
            dist_mult,
            None,
            None,
        )
    }

    fn update_entity(&mut self, registry: &mut SimRegistry, key: i32, origin: Vec3, dist_mult: f32) {
        registry.update_position(key, origin, dist_mult, None, None);
    }

    fn pop_entity(&mut self, registry: &mut SimRegistry, key: i32) -> bool {
        registry.pop(&mut self.simulator, key)
    }

    fn update_results(&mut self, registry: &mut SimRegistry) -> bool {
        registry.save_outputs_for_all(SimulationFlags::DIRECT);
        self.dirty.swap(false, Ordering::SeqCst)
    }
}

pub struct SimulationManager {
    inner: Arc<Mutex<SimulationInner>>,
    listener_reverb_params: std::cell::RefCell<Option<audionimbus::ReflectionEffectParams>>,
    enabled: Arc<AtomicBool>,
}

impl SimulationManager {
    pub fn new(context: &audionimbus::Context, audio_settings: &audionimbus::AudioSettings) -> Self {
        let mut simulator = Simulator::builder(
            audionimbus::SceneParams::Default,
            audio_settings.sampling_rate,
            audio_settings.frame_size,
            1,
        )
        .with_direct(DirectSimulationSettings {
            max_num_occlusion_samples: 8,
        })
        .try_build(context)
        .unwrap();

        let listener_coords = vec_to_listener_coords(Vec3::ZERO);
        simulator.commit();

        let inner = Arc::new(Mutex::new(SimulationInner {
            simulator,
            scene: None,
            static_meshes: Vec::new(),
            has_scene: false,
            sim_cooldown: 0,
            listener_coords,
            dirty: AtomicBool::new(false),
            scene_dirty: AtomicBool::new(true),
        }));

        log::info!("[fmod] Steam Audio simulator: DIRECT-only (reflections deferred — Context race with phonon_fmod)");

        Self {
            inner,
            listener_reverb_params: std::cell::RefCell::new(None),
            enabled: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    pub fn tick_listener(&self, listener: &ListenerView) {
        self.push_listener_full(
            listener.origin,
            listener.right,
            listener.up,
            listener.forward,
        );
    }

    pub fn run_paint_frame(&self, listener: &ListenerView, registry: &mut SimRegistry) {
        self.tick_listener(listener);
        {
            let mut guard = self.inner.lock().unwrap();
            if !guard.has_scene {
                return;
            }
            if guard.sim_cooldown > 0 {
                guard.sim_cooldown -= 1;
                return;
            }
        }

        self.push_listener_full(
            listener.origin,
            listener.right,
            listener.up,
            listener.forward,
        );

        let reverb_params = self.run_direct_and_collect(registry);
        self.listener_reverb_params.replace(reverb_params);
    }

    pub fn with_listener_reverb_source<R>(
        &self,
        f: impl FnOnce(&audionimbus::Source) -> R,
    ) -> Option<R> {
        let _ = f;
        None
    }

    /// Caller: pause → studio.update → mixer_suspend → iplFMODTerminate →
    /// this → iplFMODInitialize → mixer_resume.
    pub fn set_scene(
        &self,
        scene: audionimbus::Scene,
        static_meshes: Vec<audionimbus::StaticMesh>,
    ) {
        self.set_enabled(false);
        let mut guard = self.inner.lock().unwrap();
        guard.simulator.set_scene(&scene);
        guard.simulator.commit();
        guard.scene = Some(scene);
        guard.static_meshes = static_meshes;
        guard.has_scene = true;
        guard.sim_cooldown = 2;
        guard.scene_dirty.store(true, Ordering::Relaxed);
    }

    pub fn set_probe_batch(&self, _batch: audionimbus::ProbeBatch) {
        log::warn!("[fmod] set_probe_batch ignored — pathing/bake not enabled yet");
    }

    pub fn sim_ready(&self) -> bool {
        let guard = self.inner.lock().unwrap();
        guard.has_scene && guard.sim_cooldown == 0
    }

    pub fn push_entity(
        &self,
        registry: &mut SimRegistry,
        key: i32,
        origin: Vec3,
        dist_mult: f32,
    ) -> Result<(), audionimbus::SteamAudioError> {
        let mut guard = self.inner.lock().unwrap();
        guard.scene_dirty.store(true, Ordering::Relaxed);
        guard.push_entity(registry, key, origin, dist_mult)
    }

    pub fn upsert_entity(
        &self,
        registry: &mut SimRegistry,
        key: i32,
        origin: Vec3,
        dist_mult: f32,
    ) -> Result<(), audionimbus::SteamAudioError> {
        if registry.contains(key) {
            self.update_entity(registry, key, origin, dist_mult);
            Ok(())
        } else {
            self.push_entity(registry, key, origin, dist_mult)
        }
    }

    pub fn update_entity(&self, registry: &mut SimRegistry, key: i32, origin: Vec3, dist_mult: f32) {
        let mut guard = self.inner.lock().unwrap();
        guard.scene_dirty.store(true, Ordering::Relaxed);
        guard.update_entity(registry, key, origin, dist_mult);
    }

    pub fn pop_entity(&self, registry: &mut SimRegistry, key: i32) -> bool {
        self.inner.lock().unwrap().pop_entity(registry, key)
    }

    pub fn push_listener_full(&self, origin: Vec3, right: Vec3, up: Vec3, forward: Vec3) {
        let mut guard = self.inner.lock().unwrap();
        let new_coords = listener_to_coords(origin, right, up, forward);
        let old = &guard.listener_coords.origin;
        if (old.x - new_coords.origin.x).abs() > 0.001
            || (old.y - new_coords.origin.y).abs() > 0.001
            || (old.z - new_coords.origin.z).abs() > 0.001
        {
            guard.scene_dirty.store(true, Ordering::Relaxed);
        }
        guard.listener_coords = new_coords;
    }

    fn run_direct_and_collect(
        &self,
        registry: &mut SimRegistry,
    ) -> Option<audionimbus::ReflectionEffectParams> {
        let mut guard = self.inner.lock().unwrap();
        let listener_coords = guard.listener_coords;

        let shared = SimulationSharedInputs {
            listener: listener_coords,
            num_rays: 0,
            num_bounces: 0,
            duration: 0.0,
            order: AMBISONICS_ORDER,
            irradiance_min_distance: 1.0,
            pathing_visualization_callback: None,
        };
        guard
            .simulator
            .set_shared_inputs(SimulationFlags::DIRECT, &shared);

        registry.commit_if_dirty(&mut guard.simulator);
        guard.simulator.run_direct();
        registry.harvest_graveyard();
        let _ = guard.update_results(registry);
        None
    }

    pub fn has_scene(&self) -> bool {
        self.inner.lock().unwrap().has_scene
    }

    pub fn has_probes(&self) -> bool {
        false
    }

    pub fn shutdown(&mut self) {}

    pub fn indirect_sim_outputs_ready(&self, _registry: &SimRegistry, _key: i32) -> bool {
        false
    }
}
