#[repr(C)]
#[derive(Clone, Debug)]
pub struct TerrainCollider {
    pub friction: f32,
    pub restitution: f32,
}

impl Default for TerrainCollider {
    fn default() -> Self {
        Self {
            friction: 0.5,
            restitution: 0.0,
        }
    }
}
