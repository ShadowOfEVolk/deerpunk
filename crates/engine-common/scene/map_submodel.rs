//! HL brush submodel AABB (map entity `model` `*N`); not BSP file parsing.

#[derive(Clone, Copy, Debug, Default)]
pub struct BspSubmodel {
    pub origin: [f32; 3],
    pub mins: [f32; 3],
    pub maxs: [f32; 3],
}

impl BspSubmodel {
    pub fn center(&self) -> [f32; 3] {
        [
            (self.mins[0] + self.maxs[0]) * 0.5,
            (self.mins[1] + self.maxs[1]) * 0.5,
            (self.mins[2] + self.maxs[2]) * 0.5,
        ]
    }
}
