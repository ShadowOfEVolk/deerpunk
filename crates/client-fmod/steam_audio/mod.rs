//! Steam Audio core: context, HRTF, simulation. Ported from darkdemo-lib.

pub mod registry;
pub mod simulation_manager;

use audionimbus::{AudioSettings, Context, ContextSettings, Hrtf, HrtfSettings};
use crate::ListenerView;

/// GoldSrc: 40 world units per meter.
pub const METERS_IN_UNIT: f32 = 1.0 / 40.0;
pub const PATHING_VISIBILITY_RADIUS_HL: f32 = 256.0;
pub const PATHING_VISIBILITY_RADIUS_METERS: f32 =
    PATHING_VISIBILITY_RADIUS_HL * METERS_IN_UNIT;
pub const OUTPUT_RATE: u32 = 44100;
pub const BLOCK_FRAME_SIZE: u32 = 512;
/// Ambisonics order for pathing/reflections. Order 1 = 4 channels.
/// (Order 2 crashes Phonon in HybridReverbEstimator::estimate — see reference.)
pub const AMBISONICS_ORDER: u32 = 1;

pub struct SteamAudio {
    pub context: Context,
    pub hrtf: Hrtf,
    pub audio_settings: AudioSettings,
    pub simulation_manager: simulation_manager::SimulationManager,
    pub registry: std::cell::RefCell<registry::SimRegistry>,
}

impl SteamAudio {
    pub fn new() -> Result<Self, audionimbus::SteamAudioError> {
        let audio_settings = AudioSettings {
            sampling_rate: OUTPUT_RATE,
            frame_size: BLOCK_FRAME_SIZE,
        };

        let context = Context::try_new(&ContextSettings::default())?;
        let hrtf = Hrtf::try_new(&context, &audio_settings, &HrtfSettings::default())?;
        let simulation_manager = simulation_manager::SimulationManager::new(&context, &audio_settings);

        Ok(Self {
            context,
            hrtf,
            audio_settings,
            simulation_manager,
            registry: std::cell::RefCell::new(registry::SimRegistry::new()),
        })
    }

    /// Frame tick: listener pose → sim (direct) → outputs cached in registry.
    pub fn update(&self, listener: &ListenerView) {
        self.simulation_manager.tick_listener(listener);
        let mut registry = self.registry.borrow_mut();
        self.simulation_manager.run_paint_frame(listener, &mut registry);
    }
}

impl Drop for SteamAudio {
    fn drop(&mut self) {
        self.simulation_manager.shutdown();
    }
}
