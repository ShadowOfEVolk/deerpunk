use crate::particle::LiveParticle;

pub struct IntegrateCtx<'a> {
    pub time: f32,
    pub dt: f32,
    pub live: &'a mut Vec<LiveParticle>,
}

/// Age particles, remove dead, run per-particle integration (affectors + position).
pub fn integrate_live<F>(ctx: IntegrateCtx<'_>, mut step: F)
where
    F: FnMut(&mut LiveParticle, f32, f32),
{
    let IntegrateCtx { time, dt, live } = ctx;
    let mut i = 0;
    while i < live.len() {
        live[i].age += dt;
        if live[i].age >= live[i].lifetime {
            live.swap_remove(i);
            continue;
        }
        live[i].phase += live[i].phase_rate * dt;
        step(&mut live[i], dt, time);
        i += 1;
    }
}
