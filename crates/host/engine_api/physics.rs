/// Out-param for `v1:physics:cast_capsule`. Must match engine-api layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct CapsuleCastHitOut {
    pub hit: u32,
    pub start_solid: u32,
    pub toi: f32,
    pub normal: [f32; 3],
    pub point: [f32; 3],
    pub entity: u64,
}
