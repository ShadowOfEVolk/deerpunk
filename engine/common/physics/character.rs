//! ECS [`CharacterController`] — gameplay writes `velocity` (+ optional origin).
//!
//! **Entity origin = feet** (Z-up). Rapier uses a centered `capsule_z`; collider local
//! translation is feet + [`CharacterController::center_offset`].
//!
//! Default path: engine Rapier KCC integrates `velocity`. With [`CHAR_FLAG_MANUAL_MOVE`],
//! gamelib owns slide/step via `cast_capsule` and writes Transform + grounded.

/// HL standing hull height 72 → capsule radius 16, cylinder half-height 20
/// (`2 * (half_height + radius) = 72`).
pub const CHAR_RADIUS: f32 = 16.0;
pub const CHAR_HALF_HEIGHT: f32 = 20.0;
pub const CHAR_STEP_HEIGHT: f32 = 28.0;
/// Eye height above feet (HL standing: hull half 36 + view_ofs 28).
pub const CHAR_EYE_HEIGHT: f32 = 64.0;

/// [`CharacterController::flags`]: ignore world collision, fly with view.
pub const CHAR_FLAG_NOCLIP: u32 = 1 << 0;
/// Gamelib owns integration (move-and-slide); engine skips Rapier KCC.
pub const CHAR_FLAG_MANUAL_MOVE: u32 = 1 << 1;

/// Flecs POD: kinematic character driven by authored linear velocity.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CharacterController {
    /// Gameplay velocity (units/sec). Written by gamelib / shared-player-move.
    pub velocity: [f32; 3],
    /// Set by engine after `physics.writeback` (1 = on ground).
    pub grounded: u32,
    pub radius: f32,
    pub half_height: f32,
    pub step_height: f32,
    pub max_slope_deg: f32,
    pub flags: u32,
}

impl Default for CharacterController {
    fn default() -> Self {
        Self {
            velocity: [0.0; 3],
            grounded: 0,
            radius: CHAR_RADIUS,
            half_height: CHAR_HALF_HEIGHT,
            step_height: CHAR_STEP_HEIGHT,
            max_slope_deg: 45.0,
            flags: 0,
        }
    }
}

impl CharacterController {
    pub fn hl_standing() -> Self {
        Self::default()
    }

    #[inline]
    pub fn noclip(&self) -> bool {
        self.flags & CHAR_FLAG_NOCLIP != 0
    }

    #[inline]
    pub fn set_noclip(&mut self, on: bool) {
        if on {
            self.flags |= CHAR_FLAG_NOCLIP;
        } else {
            self.flags &= !CHAR_FLAG_NOCLIP;
        }
    }

    #[inline]
    pub fn manual_move(&self) -> bool {
        self.flags & CHAR_FLAG_MANUAL_MOVE != 0
    }

    #[inline]
    pub fn set_manual_move(&mut self, on: bool) {
        if on {
            self.flags |= CHAR_FLAG_MANUAL_MOVE;
        } else {
            self.flags &= !CHAR_FLAG_MANUAL_MOVE;
        }
    }

    /// Feet → capsule center along +Z.
    #[inline]
    pub fn center_offset(&self) -> f32 {
        self.half_height.max(0.1) + self.radius.max(0.1)
    }

    /// Eye height above feet for camera / +use traces.
    #[inline]
    pub fn eye_height(&self) -> f32 {
        CHAR_EYE_HEIGHT
    }
}
