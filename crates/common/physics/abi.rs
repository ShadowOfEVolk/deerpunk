//! ABI-stable physics shape / collider descriptors (Flecs POD + arena handles).
//!
//! Geometry lives in a [`PhysicsBackend`] arena; [`Collider`] only stores a
//! [`PartsHandle`] (list of shape handles + local transforms). Compound-of-boxes
//! is simply several `box` parts — no special kind.

/// Wire / arena blob version.
pub const SHAPE_ABI_VERSION: u32 = 1;

pub const SHAPE_KIND_BOX: u32 = 1;
pub const SHAPE_KIND_BALL: u32 = 2;
pub const SHAPE_KIND_CONVEX: u32 = 3;
pub const SHAPE_KIND_HALFSPACE: u32 = 4;

pub const COLLIDER_FLAG_SENSOR: u32 = 1 << 0;

pub const BODY_TYPE_STATIC: u32 = 0;
pub const BODY_TYPE_DYNAMIC: u32 = 1;
pub const BODY_TYPE_KINEMATIC: u32 = 2;

/// Handle into the physics shape arena. `0` = invalid.
pub type ShapeHandle = u32;
pub const SHAPE_HANDLE_INVALID: ShapeHandle = 0;

/// Handle into the parts-list arena (`ColliderPart` slice). `0` = invalid.
pub type PartsHandle = u32;
pub const PARTS_HANDLE_INVALID: PartsHandle = 0;

/// One shape instance on a body (local transform relative to entity).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct ColliderPart {
    pub shape: ShapeHandle,
    pub local_pos: [f32; 3],
    /// Unit quaternion xyzw.
    pub local_rot: [f32; 4],
}

impl ColliderPart {
    pub fn identity(shape: ShapeHandle) -> Self {
        Self {
            shape,
            local_pos: [0.0; 3],
            local_rot: [0.0, 0.0, 0.0, 1.0],
        }
    }

    pub fn with_pos(shape: ShapeHandle, local_pos: [f32; 3]) -> Self {
        Self {
            shape,
            local_pos,
            local_rot: [0.0, 0.0, 0.0, 1.0],
        }
    }
}

/// Flecs component: rigid body params + parts handle (POD, no heap).
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Collider {
    pub body_type: u32,
    pub category_bits: u32,
    pub mask_bits: u32,
    pub friction: f32,
    pub restitution: f32,
    pub density: f32,
    pub flags: u32,
    pub parts: PartsHandle,
}

impl Default for Collider {
    fn default() -> Self {
        Self {
            body_type: BODY_TYPE_STATIC,
            category_bits: 0,
            mask_bits: 0xFFFF,
            friction: 0.5,
            restitution: 0.0,
            density: 0.0,
            flags: 0,
            parts: PARTS_HANDLE_INVALID,
        }
    }
}

impl Collider {
    pub fn static_parts(parts: PartsHandle, category_bits: u32, mask_bits: u32) -> Self {
        Self {
            body_type: BODY_TYPE_STATIC,
            category_bits,
            mask_bits,
            friction: 0.5,
            restitution: 0.0,
            density: 0.0,
            flags: 0,
            parts,
        }
    }

    pub fn is_sensor(&self) -> bool {
        self.flags & COLLIDER_FLAG_SENSOR != 0
    }
}

/// Encode a box shape ABI blob (`half_extents` = full half-size XYZ).
pub fn encode_shape_box(half_extents: [f32; 3]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 * 5);
    out.extend_from_slice(&SHAPE_ABI_VERSION.to_le_bytes());
    out.extend_from_slice(&SHAPE_KIND_BOX.to_le_bytes());
    for v in half_extents {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Encode a ball shape ABI blob.
pub fn encode_shape_ball(radius: f32) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 * 3);
    out.extend_from_slice(&SHAPE_ABI_VERSION.to_le_bytes());
    out.extend_from_slice(&SHAPE_KIND_BALL.to_le_bytes());
    out.extend_from_slice(&radius.to_le_bytes());
    out
}

/// Encode a half-space (`outward_normal` unit-ish; solid is the side opposite the normal).
pub fn encode_shape_halfspace(outward_normal: [f32; 3]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 * 5);
    out.extend_from_slice(&SHAPE_ABI_VERSION.to_le_bytes());
    out.extend_from_slice(&SHAPE_KIND_HALFSPACE.to_le_bytes());
    for v in outward_normal {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Encode a convex hull ABI blob (vertex list, world/local space of the shape).
pub fn encode_shape_convex(vertices: &[[f32; 3]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(16 + vertices.len() * 12);
    out.extend_from_slice(&SHAPE_ABI_VERSION.to_le_bytes());
    out.extend_from_slice(&SHAPE_KIND_CONVEX.to_le_bytes());
    out.extend_from_slice(&(vertices.len() as u32).to_le_bytes());
    for v in vertices {
        for c in v {
            out.extend_from_slice(&c.to_le_bytes());
        }
    }
    out
}

/// Decoded shape geometry (host-side; not an ECS component).
#[derive(Clone, Debug)]
pub enum ShapeGeom {
    Box { half_extents: [f32; 3] },
    Ball { radius: f32 },
    Convex { vertices: Vec<[f32; 3]> },
    HalfSpace { outward_normal: [f32; 3] },
}

/// Decode a shape ABI blob. `None` on truncated / unknown version/kind.
pub fn decode_shape(abi: &[u8]) -> Option<ShapeGeom> {
    if abi.len() < 8 {
        return None;
    }
    let version = u32::from_le_bytes(abi[0..4].try_into().ok()?);
    if version != SHAPE_ABI_VERSION {
        return None;
    }
    let kind = u32::from_le_bytes(abi[4..8].try_into().ok()?);
    let rest = &abi[8..];
    match kind {
        SHAPE_KIND_BOX => {
            if rest.len() < 12 {
                return None;
            }
            Some(ShapeGeom::Box {
                half_extents: [
                    f32::from_le_bytes(rest[0..4].try_into().ok()?),
                    f32::from_le_bytes(rest[4..8].try_into().ok()?),
                    f32::from_le_bytes(rest[8..12].try_into().ok()?),
                ],
            })
        }
        SHAPE_KIND_BALL => {
            if rest.len() < 4 {
                return None;
            }
            Some(ShapeGeom::Ball {
                radius: f32::from_le_bytes(rest[0..4].try_into().ok()?),
            })
        }
        SHAPE_KIND_CONVEX => {
            if rest.len() < 4 {
                return None;
            }
            let n = u32::from_le_bytes(rest[0..4].try_into().ok()?) as usize;
            let need = 4 + n * 12;
            if rest.len() < need {
                return None;
            }
            let mut vertices = Vec::with_capacity(n);
            let mut off = 4;
            for _ in 0..n {
                vertices.push([
                    f32::from_le_bytes(rest[off..off + 4].try_into().ok()?),
                    f32::from_le_bytes(rest[off + 4..off + 8].try_into().ok()?),
                    f32::from_le_bytes(rest[off + 8..off + 12].try_into().ok()?),
                ]);
                off += 12;
            }
            Some(ShapeGeom::Convex { vertices })
        }
        SHAPE_KIND_HALFSPACE => {
            if rest.len() < 12 {
                return None;
            }
            Some(ShapeGeom::HalfSpace {
                outward_normal: [
                    f32::from_le_bytes(rest[0..4].try_into().ok()?),
                    f32::from_le_bytes(rest[4..8].try_into().ok()?),
                    f32::from_le_bytes(rest[8..12].try_into().ok()?),
                ],
            })
        }
        _ => None,
    }
}
