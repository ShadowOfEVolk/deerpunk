//! Player locomotion intent: usercmd → effective max speed + stamina.
//!
//! `CharacterController` holds capsule/velocity; this component decides walk vs sprint
//! and drains/regens stamina (ported from darkdemo-lib `PlayerSprint`).

/// Horizontal speed below which regen uses the "standing" rate.
const STANDING_VELOCITY_THRESHOLD: f32 = 10.0;
/// Must be moving at least this fast while sprinting to drain stamina.
const SPRINT_DRAIN_SPEED: f32 = 60.0;
/// Stamina at/above this clears the tired latch.
const TIRED_RECOVER: f32 = 0.3;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PlayerMovement {
    /// Base wish speed (units/sec).
    pub walk_speed: f32,
    /// Multiplier while sprinting (`BUTTON_SPEED`).
    pub sprint_mult: f32,
    /// Current stamina 0..1.
    pub stamina: f32,
    /// 1 = exhausted; blocks sprint until stamina recovers to [`TIRED_RECOVER`].
    pub is_tired: u8,
    pub _pad: [u8; 3],
    /// Seconds to drain full bar while sprinting.
    pub stamina_drain_time: f32,
    /// Seconds to refill while standing still.
    pub stamina_regen_standing: f32,
    /// Seconds to refill while walking.
    pub stamina_regen_moving: f32,
}

impl Default for PlayerMovement {
    fn default() -> Self {
        Self {
            // darkdemo-lib `PlayerSprint`: walk 50, sprint 110 → mult 2.2
            walk_speed: 50.0,
            sprint_mult: 2.2,
            stamina: 1.0,
            is_tired: 0,
            _pad: [0; 3],
            stamina_drain_time: 5.0,
            stamina_regen_standing: 10.0,
            stamina_regen_moving: 30.0,
        }
    }
}

impl PlayerMovement {
    #[inline]
    pub fn tired(&self) -> bool {
        self.is_tired != 0
    }

    /// Tick stamina from this frame's input; return wish `max_speed` for the move executor.
    ///
    /// `want_sprint` = Shift held. Sprint requires grounded + not tired.
    /// Drain only when actually sprinting and horizontal speed ≥ 60 (lib behaviour).
    pub fn tick_and_max_speed(
        &mut self,
        dt: f32,
        want_sprint: bool,
        grounded: bool,
        horiz_speed: f32,
    ) -> f32 {
        let dt = dt.max(0.0);
        let can_sprint = want_sprint && grounded && !self.tired();
        let sprinting = can_sprint;

        if sprinting && horiz_speed >= SPRINT_DRAIN_SPEED {
            let rate = 1.0 / self.stamina_drain_time.max(0.001);
            self.stamina = (self.stamina - rate * dt).max(0.0);
        } else if !sprinting {
            if self.stamina < 1.0 {
                let regen_time = if horiz_speed < STANDING_VELOCITY_THRESHOLD {
                    self.stamina_regen_standing
                } else {
                    self.stamina_regen_moving
                };
                let rate = 1.0 / regen_time.max(0.001);
                self.stamina = (self.stamina + rate * dt).min(1.0);
            }
        }

        self.update_tired();

        // Re-check after tired latch may flip at stamina == 0 this frame.
        if want_sprint && grounded && !self.tired() {
            self.walk_speed.max(0.0) * self.sprint_mult.max(1.0)
        } else {
            self.walk_speed.max(0.0)
        }
    }

    fn update_tired(&mut self) {
        if self.stamina <= 0.0 && !self.tired() {
            self.is_tired = 1;
        } else if self.stamina >= TIRED_RECOVER && self.tired() {
            self.is_tired = 0;
        }
    }
}
