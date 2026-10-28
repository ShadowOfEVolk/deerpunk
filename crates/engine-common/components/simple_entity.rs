/// Simple entity component for testing
#[derive(Clone, Copy)]
pub struct SimpleEntity {
    pub velocity_x: f32,
    pub velocity_y: f32,
}

impl SimpleEntity {
    pub fn new() -> Self {
        Self {
            velocity_x: 50.0,
            velocity_y: 50.0,
        }
    }
}

impl Default for SimpleEntity {
    fn default() -> Self {
        Self::new()
    }
}

