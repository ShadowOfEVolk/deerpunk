// Simple circle renderer component with color and radius
#[derive(Clone, Copy, PartialEq)]
pub struct SimpleCircleRenderer {
    pub color: u32,    // RGBA color as u32
    pub radius: f32,   // Circle radius
}

impl Default for SimpleCircleRenderer {
    fn default() -> Self {
        Self {
            color: 0xFFFFFFFF, // White, fully opaque
            radius: 10.0,
        }
    }
}

// Network replication support
impl crate::NetworkComponent for SimpleCircleRenderer {
    fn declare_fields() -> &'static [crate::NetworkField] {
        static FIELDS: [crate::NetworkField; 2] = [
            crate::NetworkField { offset: 0, size: 4 },  // color (u32)
            crate::NetworkField { offset: 4, size: 4 },  // radius (f32)
        ];

        &FIELDS
    }
}

