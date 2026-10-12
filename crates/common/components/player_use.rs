//! Player +use input latch / cooldown (prefab-authored on the pawn).

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct PlayerUse {
    pub e_was_down: u8,
    pub _pad: [u8; 3],
    pub cooldown_until: f32,
}
