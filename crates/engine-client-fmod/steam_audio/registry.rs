//! Phonon Source registry: one `Source` per sound key, cached simulation outputs.
//! Ported from game-lib client-audio.

use std::collections::BTreeMap;

use audionimbus::{
    BakedDataIdentifier, CoordinateSystem, DeviationModel, DirectSimulationParameters,
    DistanceAttenuationModel, Occlusion, OcclusionAlgorithm, PathingSimulationParameters,
    ProbeBatch, ReflectionsSimulationParameters, SimulationFlags, SimulationInputs, Simulator,
    Source, SourceSettings, TransmissionParameters, Vector3,
};
use glam::Vec3;

use super::AMBISONICS_ORDER;
use super::PATHING_VISIBILITY_RADIUS_METERS;

unsafe extern "C" fn dist_atten_callback(distance: f32, user_data: *mut core::ffi::c_void) -> f32 {
    if user_data.is_null() {
        return 1.0;
    }
    let dist_mult = unsafe { *(user_data as *const f32) };
    (1.0 - distance * dist_mult).max(0.0)
}

struct SourceEntry {
    source: Source,
    ref_count: u32,
    dist_mult_box: Box<f32>,
    last_origin: Vec3,
    dirty: bool,
    results: audionimbus::SimulationOutputs,
}

/// Registry of Phonon sources keyed by sound key (as i32).
pub struct SimRegistry {
    map: BTreeMap<i32, SourceEntry>,
    dirty: bool,
    key_buf: Vec<i32>,
    graveyard: Vec<SourceEntry>,
}

impl Default for SimRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SimRegistry {
    pub fn new() -> Self {
        Self {
            map: BTreeMap::new(),
            dirty: false,
            key_buf: Vec::new(),
            graveyard: Vec::new(),
        }
    }

    pub fn contains(&self, key: i32) -> bool {
        self.map.contains_key(&key)
    }

    pub fn source_for(&self, key: i32) -> Option<&Source> {
        self.map.get(&key).map(|entry| &entry.source)
    }

    pub fn listener_to_coords(
        origin: Vec3,
        right: Vec3,
        up: Vec3,
        forward: Vec3,
    ) -> CoordinateSystem {
        let o = origin * crate::METERS_IN_UNIT;
        let x2s = |v: Vec3| Vector3::new(v.x, v.z, -v.y);
        CoordinateSystem {
            right: x2s(right),
            up: x2s(up),
            ahead: x2s(forward),
            origin: Vector3::new(o.x, o.z, -o.y),
        }
    }

    fn vec_to_coord(origin: Vec3) -> CoordinateSystem {
        let o = origin * crate::METERS_IN_UNIT;
        CoordinateSystem {
            right: Vector3::new(1.0, 0.0, 0.0),
            up: Vector3::new(0.0, 1.0, 0.0),
            ahead: Vector3::new(0.0, 0.0, -1.0),
            origin: Vector3::new(o.x, o.z + 0.1, -o.y),
        }
    }

    fn sim_flags(has_probes: bool) -> SimulationFlags {
        if has_probes {
            SimulationFlags::DIRECT | SimulationFlags::REFLECTIONS | SimulationFlags::PATHING
        } else {
            // Without bake, REFLECTIONS/PATHING inputs + getOutputs heap-corrupt phonon.
            SimulationFlags::DIRECT
        }
    }

    fn make_inputs<'a>(
        origin: Vec3,
        dist_mult_box: &Box<f32>,
        probe_batch: Option<&'a ProbeBatch>,
        reflections_baked_id: Option<&'a BakedDataIdentifier>,
    ) -> SimulationInputs<'a> {
        let has_probes = probe_batch.is_some();
        SimulationInputs {
            source: Self::vec_to_coord(origin),
            direct_simulation: Some(DirectSimulationParameters {
                distance_attenuation: Some(DistanceAttenuationModel::Callback {
                    callback: dist_atten_callback,
                    user_data: &**dist_mult_box as *const f32 as *mut core::ffi::c_void,
                    dirty: false,
                }),
                air_absorption: Some(audionimbus::AirAbsorptionModel::Default),
                directivity: None,
                occlusion: Some(Occlusion {
                    transmission: Some(TransmissionParameters {
                        num_transmission_rays: 4,
                    }),
                    algorithm: OcclusionAlgorithm::Volumetric {
                        radius: 0.5,
                        num_occlusion_samples: 8,
                    },
                }),
            }),
            reflections_simulation: if has_probes {
                Some(ReflectionsSimulationParameters::Parametric {
                    reverb_scale: [1.2, 0.8, 0.4],
                    baked_data_identifier: reflections_baked_id.copied(),
                })
            } else {
                None
            },
            pathing_simulation: probe_batch.map(|pb| PathingSimulationParameters {
                pathing_probes: pb,
                visibility_radius: PATHING_VISIBILITY_RADIUS_METERS,
                visibility_threshold: 0.15,
                visibility_range: 1000.0,
                pathing_order: AMBISONICS_ORDER,
                enable_validation: true,
                find_alternate_paths: true,
                deviation: DeviationModel::default(),
            }),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push<D, R, P>(
        &mut self,
        simulator: &mut Simulator<D, R, P>,
        key: i32,
        origin: Vec3,
        dist_mult: f32,
        probe_batch: Option<&ProbeBatch>,
        reflections_baked_id: Option<&BakedDataIdentifier>,
    ) -> Result<(), audionimbus::SteamAudioError> {
        let sim_flags = Self::sim_flags(probe_batch.is_some());
        if let Some(entry) = self.map.get_mut(&key) {
            entry.ref_count = entry.ref_count.saturating_add(1);
            *entry.dist_mult_box = dist_mult;
            if (entry.last_origin - origin).length_squared() > 0.001 {
                entry.dirty = true;
            }
            entry.source.set_inputs(
                sim_flags,
                Self::make_inputs(
                    origin,
                    &entry.dist_mult_box,
                    probe_batch,
                    reflections_baked_id,
                ),
            );
            return Ok(());
        }

        let source_settings = SourceSettings { flags: sim_flags };
        let source: Source = Source::try_new(simulator, &source_settings)?;
        simulator.add_source(&source);
        self.dirty = true;
        self.map.insert(
            key,
            SourceEntry {
                source,
                ref_count: 1,
                dist_mult_box: Box::new(dist_mult),
                last_origin: origin,
                dirty: true,
                results: audionimbus::SimulationOutputs::default(),
            },
        );
        let entry = self.map.get_mut(&key).unwrap();
        entry.source.set_inputs(
            sim_flags,
            Self::make_inputs(
                origin,
                &entry.dist_mult_box,
                probe_batch,
                reflections_baked_id,
            ),
        );
        Ok(())
    }

    pub fn pop<D, R, P>(&mut self, simulator: &mut Simulator<D, R, P>, key: i32) -> bool {
        let Some(entry) = self.map.get_mut(&key) else {
            return false;
        };
        entry.ref_count = entry.ref_count.saturating_sub(1);
        if entry.ref_count != 0 {
            return false;
        }
        let src = entry.source.clone();
        simulator.remove_source(&src);
        self.dirty = true;
        if let Some(dead) = self.map.remove(&key) {
            self.graveyard.push(dead);
        }
        true
    }

    pub fn update_position(
        &mut self,
        key: i32,
        origin: Vec3,
        dist_mult: f32,
        probe_batch: Option<&ProbeBatch>,
        reflections_baked_id: Option<&BakedDataIdentifier>,
    ) {
        if let Some(entry) = self.map.get_mut(&key) {
            if (entry.last_origin - origin).length_squared() > 0.001 {
                entry.dirty = true;
            }
            entry.last_origin = origin;
            *entry.dist_mult_box = dist_mult;
            entry.source.set_inputs(
                Self::sim_flags(probe_batch.is_some()),
                Self::make_inputs(
                    origin,
                    &entry.dist_mult_box,
                    probe_batch,
                    reflections_baked_id,
                ),
            );
        }
    }

    pub fn save_outputs_for_all(&mut self, flags: SimulationFlags) {
        self.key_buf.clear();
        self.key_buf.extend(self.map.keys().copied());
        for i in 0..self.key_buf.len() {
            let key = self.key_buf[i];
            if let Some(entry) = self.map.get_mut(&key) {
                unsafe {
                    audionimbus_sys::iplSourceGetOutputs(
                        entry.source.raw_ptr(),
                        flags.into(),
                        entry.results.raw_ptr(),
                    );
                }
            }
        }
    }

    pub fn mark_reflections_clean(&mut self) {
        for entry in self.map.values_mut() {
            entry.dirty = false;
        }
    }

    pub fn get_cached_outputs(&self, key: i32) -> Option<&audionimbus::SimulationOutputs> {
        self.map.get(&key).map(|e| &e.results)
    }

    pub fn get_cached_outputs_mut(
        &mut self,
        key: i32,
    ) -> Option<&mut audionimbus::SimulationOutputs> {
        self.map.get_mut(&key).map(|e| &mut e.results)
    }

    pub fn commit_if_dirty<D, R, P>(&mut self, simulator: &mut Simulator<D, R, P>) {
        if self.dirty {
            simulator.commit();
            self.dirty = false;
        }
    }

    pub fn harvest_graveyard(&mut self) {
        self.graveyard.clear();
    }
}
