//! FMOD Studio ↔ Steam Audio bridge: shared context, reverb source, per-entity sources.
//! Loads `iplFMOD*` entry points from phonon_fmod.dll at runtime.
//! Ported from darkdemo-lib client-audio/fmod_steam_audio.rs.

use std::sync::atomic::{AtomicBool, Ordering};

use audionimbus::{DirectSimulationSettings, SceneParams, SimulationSettings};
use libfmod::System;
use libloading::Library;
use log::{info, warn};

use crate::steam_audio::simulation_manager::SimulationManager;
use crate::steam_audio::SteamAudio;
use crate::steam_audio::{AMBISONICS_ORDER, BLOCK_FRAME_SIZE, OUTPUT_RATE};

const PHONON_FMOD_DLL: &str = "phonon_fmod.dll";

/// `IPLSpatializerParams` in `steamaudio_fmod.h` — Steam Audio Spatializer DSP params.
pub const IPL_SPATIALIZE_APPLY_DISTANCEATTENUATION: i32 = 2;
pub const IPL_SPATIALIZE_APPLY_AIRABSORPTION: i32 = 3;
pub const IPL_SPATIALIZE_APPLY_DIRECTIVITY: i32 = 4;
pub const IPL_SPATIALIZE_APPLY_OCCLUSION: i32 = 5;
pub const IPL_SPATIALIZE_APPLY_TRANSMISSION: i32 = 6;
pub const IPL_SPATIALIZE_APPLY_REFLECTIONS: i32 = 7;
pub const IPL_SPATIALIZE_APPLY_PATHING: i32 = 8;
pub const IPL_SPATIALIZE_SIMULATION_OUTPUTS_HANDLE: i32 = 33;

/// `ParameterApplyType::PARAMETER_SIMULATIONDEFINED` in phonon_fmod.
pub const IPL_SPATIALIZE_SIMULATION_DEFINED: i32 = 1;

type FnInit = unsafe extern "C" fn(audionimbus_sys::IPLContext);
type FnTerminate = unsafe extern "C" fn();
type FnSetHrtf = unsafe extern "C" fn(audionimbus_sys::IPLHRTF);
type FnSetSim = unsafe extern "C" fn(audionimbus_sys::IPLSimulationSettings);
type FnSetReverb = unsafe extern "C" fn(audionimbus_sys::IPLSource);
type FnAddSource = unsafe extern "C" fn(audionimbus_sys::IPLSource) -> i32;
type FnRemoveSource = unsafe extern "C" fn(i32);

struct SteamFmodApi {
    _lib: Library,
    init: FnInit,
    terminate: FnTerminate,
    set_hrtf: FnSetHrtf,
    set_simulation_settings: FnSetSim,
    set_reverb_source: FnSetReverb,
    add_source: FnAddSource,
    remove_source: FnRemoveSource,
}

impl SteamFmodApi {
    fn load(path: &std::path::Path) -> Option<Self> {
        unsafe {
            let lib = Library::new(path).ok()?;
            let init = *lib.get::<FnInit>(b"iplFMODInitialize\0").ok()?;
            let terminate = *lib.get::<FnTerminate>(b"iplFMODTerminate\0").ok()?;
            let set_hrtf = *lib.get::<FnSetHrtf>(b"iplFMODSetHRTF\0").ok()?;
            let set_simulation_settings =
                *lib.get::<FnSetSim>(b"iplFMODSetSimulationSettings\0").ok()?;
            let set_reverb_source =
                *lib.get::<FnSetReverb>(b"iplFMODSetReverbSource\0").ok()?;
            let add_source = *lib.get::<FnAddSource>(b"iplFMODAddSource\0").ok()?;
            let remove_source = *lib.get::<FnRemoveSource>(b"iplFMODRemoveSource\0").ok()?;
            Some(Self {
                _lib: lib,
                init,
                terminate,
                set_hrtf,
                set_simulation_settings,
                set_reverb_source,
                add_source,
                remove_source,
            })
        }
    }
}

pub struct FmodSteamBridge {
    plugin_loaded: AtomicBool,
    linked: AtomicBool,
    api: Option<SteamFmodApi>,
}

impl FmodSteamBridge {
    /// Resolves the iplFMOD* entry points. `plugin_already_loaded` — the
    /// backend already called `system.load_plugin` for phonon_fmod.dll.
    pub fn new_ex(system: &System, cl_dlls: &std::path::Path, plugin_already_loaded: bool) -> Self {
        let plugin_path = cl_dlls.join(PHONON_FMOD_DLL);
        let mut bridge = Self {
            plugin_loaded: AtomicBool::new(plugin_already_loaded),
            linked: AtomicBool::new(false),
            api: None,
        };

        if !plugin_already_loaded {
            match system.load_plugin(&plugin_path.to_string_lossy(), None) {
                Ok(_) => {
                    info!("[fmod] FMOD loaded Steam Audio plugin: {}", plugin_path.display());
                    bridge.plugin_loaded.store(true, Ordering::Relaxed);
                }
                Err(err) => {
                    warn!("[fmod] load_plugin({}): {err}", plugin_path.display());
                    return bridge;
                }
            }
        }

        match SteamFmodApi::load(&plugin_path) {
            Some(api) => {
                info!("[fmod] Steam Audio API resolved from {}", plugin_path.display());
                bridge.api = Some(api);
            }
            None => warn!("[fmod] phonon_fmod.dll loaded but iplFMOD* symbols missing"),
        }

        bridge
    }

    pub fn link(&self, steam: &SteamAudio) {
        let Some(api) = self.api.as_ref() else {
            return;
        };
        if self.linked.swap(true, Ordering::SeqCst) {
            return;
        }

        let sim = audionimbus_sys::IPLSimulationSettings::from(simulation_settings());
        unsafe {
            (api.init)(steam.context.raw_ptr());
            (api.set_hrtf)(steam.hrtf.raw_ptr());
            (api.set_simulation_settings)(sim);
        }
        info!("[fmod] Steam Audio bridge linked (iplFMODInitialize + HRTF + sim settings)");
    }

    /// Drop plugin hold on the shared Context so the game can SetScene safely.
    /// Pair with [`Self::link`] after Simulator SetScene/Commit.
    pub fn unlink(&self) {
        let Some(api) = self.api.as_ref() else {
            return;
        };
        if !self.linked.swap(false, Ordering::SeqCst) {
            return;
        }
        unsafe {
            (api.terminate)();
        }
        info!("[fmod] Steam Audio bridge unlinked (iplFMODTerminate)");
    }

    pub fn add_source(&self, source: &audionimbus::Source) -> Option<i32> {
        let api = self.api.as_ref()?;
        if !self.linked.load(Ordering::Relaxed) {
            return None;
        }
        let handle = unsafe { (api.add_source)(source.raw_ptr()) };
        if handle < 0 {
            warn!("[fmod] iplFMODAddSource failed");
            return None;
        }
        Some(handle)
    }

    pub fn remove_source(&self, handle: i32) {
        let Some(api) = self.api.as_ref() else {
            return;
        };
        if handle < 0 {
            return;
        }
        unsafe {
            (api.remove_source)(handle);
        }
    }

    pub fn sync_reverb_source(&self, sim: &SimulationManager) {
        let Some(api) = self.api.as_ref() else {
            return;
        };
        if !self.linked.load(Ordering::Relaxed) {
            return;
        }
        let _ = sim.with_listener_reverb_source(|source| unsafe {
            (api.set_reverb_source)(source.raw_ptr());
        });
    }

    pub fn shutdown(&mut self, entity_handles: &[i32]) {
        for &handle in entity_handles {
            self.remove_source(handle);
        }
        if self.linked.swap(false, Ordering::SeqCst) {
            if let Some(api) = self.api.as_ref() {
                unsafe {
                    (api.terminate)();
                }
            }
        }
        self.api = None;
    }
}

fn simulation_settings() -> SimulationSettings<'static> {
    // Match SimulationManager: DIRECT-only until reflections Context race is solved.
    SimulationSettings {
        scene_params: SceneParams::Default,
        direct_simulation: Some(DirectSimulationSettings {
            max_num_occlusion_samples: 8,
        }),
        reflections_simulation: None,
        pathing_simulation: None,
        sampling_rate: OUTPUT_RATE,
        frame_size: BLOCK_FRAME_SIZE,
        max_order: AMBISONICS_ORDER,
    }
}
