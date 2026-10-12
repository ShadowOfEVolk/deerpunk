//! Rapier trace helpers (`v1:physics:trace_line`).

/// Out-param for `v1:physics:trace_line` (gamelib mirrors this layout).
#[repr(C)]
pub struct PhysicsTraceResult {
    pub hit: u32,
    pub entity: u64,
    pub fraction: f32,
    pub point: [f32; 3],
    pub normal: [f32; 3],
}

impl crate::Host {
    #[allow(clippy::too_many_arguments)]
    pub fn physics_trace_line(
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
        let origin = [ox, oy, oz];
        let delta = [ex - ox, ey - oy, ez - oz];
        let len = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt();
        if len < 1e-6 {
            *out = PhysicsTraceResult {
                hit: 0,
                entity: 0,
                fraction: 1.0,
                point: origin,
                normal: [0.0, 0.0, 1.0],
            };
            return 0;
        }
        let dir = [delta[0] / len, delta[1] / len, delta[2] / len];
        match self
            .server_engine
            .physics
            .cast_ray(origin, dir, len, exclude_entity)
        {
            Some(h) => {
                *out = PhysicsTraceResult {
                    hit: 1,
                    entity: h.entity,
                    fraction: h.fraction,
                    point: h.point,
                    normal: h.normal,
                };
                1
            }
            None => {
                *out = PhysicsTraceResult {
                    hit: 0,
                    entity: 0,
                    fraction: 1.0,
                    point: [ex, ey, ez],
                    normal: [0.0, 0.0, 1.0],
                };
                0
            }
        }
    }
}
