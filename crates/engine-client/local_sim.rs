//! Client-only predicted feet (`cl.simorg`) — separate from authoritative `server_feet`.

use deerpunk_engine_common::components::TransformComponent;
use glam::{Quat, Vec3};

use crate::client_engine::ClientEngine;

const MIN_CORRECTION_DIST: f32 = 0.45;

#[derive(Clone, Copy, Debug, Default)]
pub struct LocalSimOrg {
    /// Raw prediction (gamelib).
    pub feet: Vec3,
    pub valid: bool,
    /// Last replicated server feet (transform position not applied when skipping).
    pub server_feet: Vec3,
    pub server_valid: bool,
    /// Pawn entity — skip `TransformComponent.position` replication when predict on.
    pub local_pawn: Option<u64>,
    pub skip_repl_transform: bool,
    correction_time: f32,
    /// After `tick_display` — camera / stamp / view ray.
    pub display: Vec3,
}

impl LocalSimOrg {
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn set_local_pawn(&mut self, entity: Option<u64>, predict: bool) {
        self.local_pawn = entity;
        self.skip_repl_transform = predict && entity.is_some();
    }

    pub fn set(&mut self, feet: Vec3) {
        self.feet = feet;
        self.valid = true;
    }

    pub fn set_server_feet(&mut self, feet: Vec3) {
        self.server_feet = feet;
        self.server_valid = true;
    }

    pub fn should_skip_transform_pos(&self, entity: u64) -> bool {
        self.skip_repl_transform && self.local_pawn == Some(entity)
    }

    pub fn push_correction(&mut self, predicted: Vec3, authoritative: Vec3, smooth_time: f32) {
        let err = authoritative - predicted;
        if err.length() > MIN_CORRECTION_DIST && smooth_time > 0.0 {
            self.correction_time = smooth_time;
        }
    }

    /// Frame-rate independent blend toward predicted feet (+ optional error ease).
    pub fn tick_display(&mut self, dt: f32, smooth_time: f32, nosmooth: bool, view_smooth: f32) {
        if !self.valid {
            self.display = self.feet;
            return;
        }
        let target = self.feet;
        if self.correction_time > 0.0 && !nosmooth && smooth_time > 0.0 {
            self.correction_time = (self.correction_time - dt).clamp(0.0, smooth_time);
        }
        if nosmooth || view_smooth <= 0.0 {
            self.display = target;
            return;
        }
        let alpha = (1.0 - (-view_smooth * dt).exp()).clamp(0.0, 1.0);
        self.display = self.display.lerp(target, alpha);
    }
}

impl ClientEngine {
    pub fn set_simorg(&mut self, feet: Vec3) {
        self.local_sim.set(feet);
    }

    pub fn clear_simorg(&mut self) {
        self.local_sim.clear();
    }

    pub fn push_prediction_correction(
        &mut self,
        predicted: Vec3,
        authoritative: Vec3,
        smooth_time: f32,
    ) {
        self.local_sim.push_correction(predicted, authoritative, smooth_time);
    }

    pub fn stamp_simorg_on_entity(&mut self, entity: u64) {
        if !self.local_sim.valid {
            return;
        }
        let tid = self.world.component_init::<TransformComponent>();
        let Some(t) = self.world.get::<TransformComponent>(entity, tid) else {
            return;
        };
        let pos = self.local_sim.display;
        self.world.set(
            entity,
            tid,
            TransformComponent {
                position: pos,
                rotation: t.rotation,
                scale: t.scale,
            },
        );
    }
}
