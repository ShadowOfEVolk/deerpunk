//! Engine particle spawn plumbing on the host.
//!
//! Bursts enter through [`Host::spawn_particles_burst`] (FFI bind
//! `v1:particles:spawn_burst`); the payload JSON is a plain serializable
//! `SpawnPayload` so the net layer (`emit_particles` svc message, later agent)
//! can encode/decode it verbatim.

use std::ffi::c_void;

use deerpunk_engine_particles::SpawnPayload;
use glam::Vec3;

/// FFI raycast hook: `origin`/`dir` are `*const f32[3]`, `dir` is unit length.
/// On hit the implementor writes `point[3]`, `normal[3]`, `dist` (7 floats)
/// to `hit_out` and returns non-zero; zero = no hit. `user_data` is passed
/// through verbatim and must stay alive for the session (no drop callback,
/// same convention as `v1:client_world:register_observer`).
pub type ParticleRaycastFn = extern "C-unwind" fn(
    origin: *const f32,
    dir: *const f32,
    max_dist: f32,
    hit_out: *mut f32,
    user_data: *mut c_void,
) -> i32;

/// Adapter: FFI raycast closure → [`WorldCollider`].
struct FfiCollider {
    f: ParticleRaycastFn,
    user_data: *mut c_void,
}

unsafe impl Send for FfiCollider {}
unsafe impl Sync for FfiCollider {}

impl deerpunk_engine_particles::WorldCollider for FfiCollider {
    fn raycast(
        &self,
        origin: Vec3,
        dir: Vec3,
        max_dist: f32,
    ) -> Option<deerpunk_engine_particles::ColliderHit> {
        let mut out = [0f32; 7];
        let hit = (self.f)(
            origin.to_array().as_ptr(),
            dir.to_array().as_ptr(),
            max_dist,
            out.as_mut_ptr(),
            self.user_data,
        );
        if hit == 0 {
            return None;
        }
        Some(deerpunk_engine_particles::ColliderHit {
            point: Vec3::new(out[0], out[1], out[2]),
            normal: Vec3::new(out[3], out[4], out[5]),
            dist: out[6],
        })
    }
}

#[cfg(any(feature = "client", feature = "full"))]
impl crate::Host {
    /// Spawn a burst of `effect` (client catalog name) at `pos` with `base_vel`.
    /// `payload_json` = serialized `SpawnPayload`; `{}` uses template defaults.
    pub fn spawn_particles_burst(
        &mut self,
        effect: &str,
        pos: [f32; 3],
        base_vel: [f32; 3],
        payload_json: &str,
    ) -> u32 {
        let payload: SpawnPayload = if payload_json.trim().is_empty() {
            SpawnPayload::default()
        } else {
            match serde_json::from_str(payload_json) {
                Ok(p) => p,
                Err(e) => {
                    log::error!("[particles] spawn_burst '{effect}': bad payload json: {e}");
                    return 0;
                }
            }
        };
        let Some(client) = self.client_engine.as_mut() else {
            log::warn!("[particles] spawn_burst '{effect}': no client engine");
            return 0;
        };
        client.spawn_particles_burst(effect, pos, base_vel, &payload);
        1
    }

    /// Register the game's world raycast for `collide_world` affectors.
    /// Wrapped in a `Send + Sync` adapter. Must be called before the particle
    /// catalog loads (`collide_world` effects hard-error at load without it).
    pub fn set_particles_world_collider(
        &mut self,
        raycast: ParticleRaycastFn,
        user_data: *mut c_void,
    ) -> u32 {
        let Some(client) = self.client_engine.as_mut() else {
            log::warn!("[particles] set_world_collider: no client engine");
            return 0;
        };
        client.set_particles_world_collider(std::sync::Arc::new(FfiCollider {
            f: raycast,
            user_data,
        }));
        1
    }
}

#[cfg(not(any(feature = "client", feature = "full")))]
impl crate::Host {
    pub fn spawn_particles_burst(
        &mut self,
        _effect: &str,
        _pos: [f32; 3],
        _base_vel: [f32; 3],
        _payload_json: &str,
    ) -> u32 {
        0
    }
}
