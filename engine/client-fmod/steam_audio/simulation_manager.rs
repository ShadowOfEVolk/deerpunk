//! Simulation manager: entities + listener. Reflections/pathing run in a
//! worker thread, direct sim runs on the frame tick. Ported from darkdemo-lib.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use audionimbus::{
    BakedDataIdentifier, CallbackInformation, CoordinateSystem, Direct,
    DirectSimulationSettings, Pathing, PathingSimulationSettings, Reflections,
    ReflectionsSimulationParameters, ReflectionsSimulationSettings, SimulationFlags,
    SimulationInputs, SimulationSharedInputs, Simulator, Source, SourceSettings,
};
use glam::Vec3;

use super::AMBISONICS_ORDER;
use super::registry::SimRegistry;
use crate::ListenerView;

const SIMULATION_UPDATE_INTERVAL_MS: u64 = 100;

pub fn listener_to_coords(origin: Vec3, right: Vec3, up: Vec3, forward: Vec3) -> CoordinateSystem {
    SimRegistry::listener_to_coords(origin, right, up, forward)
}

fn vec_to_listener_coords(origin: Vec3) -> CoordinateSystem {
    SimRegistry::listener_to_coords(origin, Vec3::X, Vec3::Z, Vec3::Y)
}

fn make_listener_reverb_inputs(listener: CoordinateSystem) -> SimulationInputs<'static> {
    let reverb_identifier = BakedDataIdentifier::Reflections {
        variation: audionimbus::BakedDataVariation::Reverb,
    };
    SimulationInputs {
        source: listener,
        direct_simulation: None,
        reflections_simulation: Some(ReflectionsSimulationParameters::Parametric {
            reverb_scale: [1.0, 1.0, 1.0],
            baked_data_identifier: Some(reverb_identifier),
        }),
        pathing_simulation: None,
    }
}

/// Internal simulation state. Phonon is not thread-safe — one lock.
pub(crate) struct SimulationInner {
    simulator: Simulator<Direct, Reflections, Pathing>,
    listener_source: Source,
    probe_batch: Option<Arc<audionimbus::ProbeBatch>>,
    reflections_baked_id: Option<BakedDataIdentifier>,
    has_scene: bool,
    listener_coords: CoordinateSystem,
    dirty: AtomicBool,
    /// Any source/listener movement — reflections/pathing need a re-run.
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
        let probe = self.probe_batch.as_ref().map(Arc::clone);
        registry.push(
            &mut self.simulator,
            key,
            origin,
            dist_mult,
            probe.as_deref(),
            self.reflections_baked_id.as_ref(),
        )
    }

    fn update_entity(&mut self, registry: &mut SimRegistry, key: i32, origin: Vec3, dist_mult: f32) {
        let probe = self.probe_batch.as_ref().map(Arc::clone);
        registry.update_position(
            key,
            origin,
            dist_mult,
            probe.as_deref(),
            self.reflections_baked_id.as_ref(),
        );
    }

    fn pop_entity(&mut self, registry: &mut SimRegistry, key: i32) -> bool {
        registry.pop(&mut self.simulator, key)
    }

    /// After run_direct — save all outputs. Returns true if the worker ran.
    fn update_results(&mut self, registry: &mut SimRegistry) -> bool {
        registry.save_outputs_for_all(
            SimulationFlags::DIRECT | SimulationFlags::PATHING | SimulationFlags::REFLECTIONS,
        );
        self.dirty.swap(false, Ordering::SeqCst)
    }
}

pub struct SimulationManager {
    inner: Arc<Mutex<SimulationInner>>,
    listener_reverb_params: std::cell::RefCell<Option<audionimbus::ReflectionEffectParams>>,
    enabled: Arc<AtomicBool>,
    _worker: thread::JoinHandle<()>,
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
        .with_reflections(ReflectionsSimulationSettings::Parametric {
            max_num_rays: 32,
            num_diffuse_samples: 1,
            max_duration: 1.0,
            max_num_sources: 32,
            num_threads: 2,
        })
        .with_pathing(PathingSimulationSettings {
            num_visibility_samples: 8,
        })
        .try_build(context)
        .unwrap();

        let listener_coords = vec_to_listener_coords(Vec3::ZERO);
        let listener_source =
            Source::try_new(&simulator, &SourceSettings {
                flags: SimulationFlags::REFLECTIONS,
            })
            .expect("listener reverb source");
        simulator.add_source(&listener_source);
        simulator.commit();

        let inner = Arc::new(Mutex::new(SimulationInner {
            simulator,
            listener_source,
            probe_batch: None,
            reflections_baked_id: None,
            has_scene: false,
            listener_coords,
            dirty: AtomicBool::new(false),
            scene_dirty: AtomicBool::new(true),
        }));

        let enabled = Arc::new(AtomicBool::new(true));
        let enabled_worker = Arc::clone(&enabled);
        let inner_worker = Arc::clone(&inner);

        let worker = thread::spawn(move || loop {
            thread::sleep(Duration::from_millis(SIMULATION_UPDATE_INTERVAL_MS));
            if !enabled_worker.load(Ordering::Relaxed) {
                continue;
            }
            let guard = match inner_worker.lock() {
                Ok(g) => g,
                Err(e) => e.into_inner(),
            };
            if !guard.scene_dirty.load(Ordering::SeqCst) || !guard.has_scene {
                continue;
            }

            let any_movement = guard.scene_dirty.swap(false, Ordering::SeqCst);

            if any_movement {
                let shared = SimulationSharedInputs {
                    listener: guard.listener_coords,
                    num_rays: 32,
                    num_bounces: 2,
                    duration: 1.0,
                    order: AMBISONICS_ORDER,
                    irradiance_min_distance: 1.0,
                    pathing_visualization_callback: None,
                };
                guard.simulator.set_shared_inputs(
                    SimulationFlags::REFLECTIONS,
                    &shared,
                );
                guard.simulator.run_reflections();

                if guard.probe_batch.is_some() {
                    guard.simulator.set_shared_inputs(
                        SimulationFlags::PATHING,
                        &shared,
                    );
                    guard.simulator.run_pathing();
                }
            }
            guard.dirty.store(true, Ordering::SeqCst);
        });

        Self {
            inner,
            listener_reverb_params: std::cell::RefCell::new(None),
            enabled,
            _worker: worker,
        }
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Game-frame tick: listener pose only.
    pub fn tick_listener(&self, listener: &ListenerView) {
        if !self.is_enabled() {
            return;
        }
        self.push_listener_full(
            listener.origin,
            listener.right,
            listener.up,
            listener.forward,
        );
    }

    /// Frame tick on the audio side: commit → run_direct → save outputs.
    pub fn run_paint_frame(&self, listener: &ListenerView, registry: &mut SimRegistry) {
        if !self.is_enabled() {
            return;
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

    pub fn with_listener_reverb_source<R>(&self, f: impl FnOnce(&audionimbus::Source) -> R) -> R {
        let guard = self.inner.lock().unwrap();
        f(&guard.listener_source)
    }

    pub fn set_scene(&self, scene: &audionimbus::Scene) {
        let mut guard = self.inner.lock().unwrap();
        guard.simulator.set_scene(scene);
        guard.simulator.commit();
        guard.has_scene = true;
        guard.scene_dirty.store(true, Ordering::Relaxed);
    }

    pub fn set_probe_batch(&self, batch: audionimbus::ProbeBatch) {
        let mut guard = self.inner.lock().unwrap();
        guard.simulator.add_probe_batch(&batch);
        guard.simulator.commit();
        guard.probe_batch = Some(Arc::new(batch));
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

    /// run_direct → update_results.
    fn run_direct_and_collect(
        &self,
        registry: &mut SimRegistry,
    ) -> Option<audionimbus::ReflectionEffectParams> {
        let mut guard = self.inner.lock().unwrap();
        let listener_coords = guard.listener_coords;

        let shared = SimulationSharedInputs {
            listener: listener_coords,
            num_rays: 32,
            num_bounces: 2,
            duration: 1.0,
            order: AMBISONICS_ORDER,
            irradiance_min_distance: 1.0,
            pathing_visualization_callback: None,
        };
        guard.simulator.set_shared_inputs(
            SimulationFlags::DIRECT | SimulationFlags::REFLECTIONS | SimulationFlags::PATHING,
            &shared,
        );
        guard.listener_source.set_inputs(
            SimulationFlags::REFLECTIONS,
            make_listener_reverb_inputs(listener_coords),
        );

        registry.commit_if_dirty(&mut guard.simulator);
        guard.simulator.run_direct();
        registry.harvest_graveyard();
        let worker_ran = guard.update_results(registry);

        if worker_ran {
            registry.mark_reflections_clean();
        }

        read_listener_reverb(&mut guard.listener_source)
    }

    pub fn has_probes(&self) -> bool {
        self.inner.lock().unwrap().probe_batch.is_some()
    }

    pub fn shutdown(&mut self) {}

    /// Cached indirect outputs ready (reflections + pathing bound)?
    pub fn indirect_sim_outputs_ready(&self, registry: &SimRegistry, key: i32) -> bool {
        let Some(outputs) = registry.get_cached_outputs(key) else {
            return false;
        };
        let reflections = outputs.reflections();
        if reflections.num_channels <= 0 {
            return false;
        }
        !outputs.pathing().sh_coeffs.raw_ptr().is_null()
    }
}

/// Reads listener reverb params from listener_source.
fn read_listener_reverb(
    listener_source: &mut Source,
) -> Option<audionimbus::ReflectionEffectParams> {
    let out = listener_source.get_outputs(SimulationFlags::REFLECTIONS);
    let params = out.reflections();
    let ok = params.num_channels > 0;
    if ok {
        Some(unsafe { std::ptr::read(&*params) })
    } else {
        None
    }
}
