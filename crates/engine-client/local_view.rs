//! Reapply local camera after replication overwrote the mirrored pawn.

use deerpunk_engine_common::components::CameraComponent;
use deerpunk_engine_common::physics::CHAR_EYE_HEIGHT;
use deerpunk_engine_server::network::ServerPrefabReplication;
use deerpunk_engine_server::players::PlayerConnections;

use crate::client_engine::{ClientEngine, LocalViewStamp};
use crate::network::ClientPrefabReplication;

/// Client-only view stamp: body yaw stays on replicated [`TransformComponent`]; pitch/roll on camera.
pub fn stamp_view_on_entity(
    client_world: &deerpunk_engine_common::world::World,
    entity: u64,
    stamp: LocalViewStamp,
) {
    let cam_id = client_world.component_init::<CameraComponent>();
    let c_ptr = client_world.get_raw(entity, cam_id) as *mut CameraComponent;
    if !c_ptr.is_null() {
        unsafe {
            let c = &mut *c_ptr;
            c.client_view = true;
            c.view_yaw = stamp.yaw;
            c.view_pitch = stamp.pitch.clamp(-1.5, 1.5);
            c.view_roll = stamp.roll;
            c.eye_height = stamp.eye_height;
            c.fov_y = stamp.fov_y.max(1.0_f32.to_radians());
        }
    }
}

impl ClientEngine {
    /// Listen-client / post-replication: restore client-only pitch on the view pawn.
    pub fn reapply_stamped_local_view(&mut self) {
        let Some(stamp) = self.local_view else {
            return;
        };
        let Some(cid) = self.rep.view_entity() else {
            return;
        };
        stamp_view_on_entity(&self.world, cid, stamp);
    }

    /// Prefer `SetView` net target; else local player slot pawn.
    pub fn reapply_local_view(
        &mut self,
        players: &PlayerConnections,
        server_rep: &ServerPrefabReplication,
    ) {
        let cid = self.rep.view_entity().or_else(|| {
            let slot = players.local_slot()?;
            let sid = players.get(slot)?.entity?;
            let net_id = server_rep.net_id_of(sid)?;
            self.rep.client_of_net(net_id)
        });
        let Some(cid) = cid else {
            return;
        };
        let stamp = self.local_view.unwrap_or_else(|| {
            let (yaw, pitch) = players
                .local_slot()
                .and_then(|s| players.get(s))
                .map(|p| (p.usercmd.yaw, p.usercmd.pitch))
                .unwrap_or((0.0, 0.0));
            LocalViewStamp {
                yaw,
                pitch,
                roll: 0.0,
                eye_height: CHAR_EYE_HEIGHT,
                fov_y: 60.0_f32.to_radians(),
            }
        });
        stamp_view_on_entity(&self.world, cid, stamp);
    }

    pub fn set_local_view_stamp(
        &mut self,
        yaw: f32,
        pitch: f32,
        roll: f32,
        eye_height: f32,
        fov_y: f32,
    ) {
        self.local_view = Some(LocalViewStamp {
            yaw,
            pitch,
            roll,
            eye_height,
            fov_y,
        });
    }

    pub fn apply_local_view_stamp_to_pawn(
        &mut self,
        players: &PlayerConnections,
        server_rep: &ServerPrefabReplication,
    ) {
        let stamp = self.local_view.expect("apply_local_view: missing stamp");
        let cid = self.rep.view_entity().or_else(|| {
            let slot = players.local_slot()?;
            let sid = players.get(slot)?.entity?;
            let net_id = server_rep.net_id_of(sid)?;
            self.rep.client_of_net(net_id)
        });
        let Some(cid) = cid else {
            return;
        };
        stamp_view_on_entity(&self.world, cid, stamp);
    }
}
