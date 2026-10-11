//! Server world → client world replication (listen-server, in-process).

use std::collections::HashMap;

use deerpunk_engine_common::components::{
    BspWorldDebug, CameraComponent, LightmapId, Material, Mesh3D, SoundEmitter, TransformComponent,
    Worldspawn,
};
use deerpunk_engine_common::physics::CharacterController;
use deerpunk_engine_common::world::World;

/// Maps server entity id → client entity id for mirrored renderables / cameras.
#[derive(Default)]
pub struct ReplicationState {
    map: HashMap<u64, u64>,
}

impl ReplicationState {
    pub fn clear(&mut self, client_world: &World) {
        for &cl in self.map.values() {
            client_world.destroy(cl);
        }
        self.map.clear();
        // Also kill any Mesh3D not tracked in `map` (stale after bad sync / deferred delete).
        let mesh_id = client_world.component_init::<Mesh3D>();
        let orphans: Vec<u64> = client_world
            .inspect_entities()
            .into_iter()
            .filter(|e| client_world.has(e.id, mesh_id))
            .map(|e| e.id)
            .collect();
        for id in orphans {
            client_world.destroy(id);
        }
        // Client world is never simulated elsewhere — flush deletes immediately.
        client_world.simulate(0.0);
    }

    /// Full resync of cameras + mesh drawables from server → client.
    pub fn sync_full(&mut self, server: &World, client: &World) {
        self.clear(client);
        init_client_components(client);

        let t_id = server.component_init::<TransformComponent>();
        let cam_id = server.component_init::<CameraComponent>();
        let mesh_id = server.component_init::<Mesh3D>();
        let mat_id = server.component_init::<Material>();
        let lm_id = server.component_init::<LightmapId>();
        let dbg_id = server.component_init::<BspWorldDebug>();
        let ws_id = server.component_init::<Worldspawn>();
        let se_id = server.component_init::<SoundEmitter>();
        let cc_id = server.component_init::<CharacterController>();

        let ct = client.component_init::<TransformComponent>();
        let ccam = client.component_init::<CameraComponent>();
        let cmesh = client.component_init::<Mesh3D>();
        let cmat = client.component_init::<Material>();
        let clm = client.component_init::<LightmapId>();
        let cdbg = client.component_init::<BspWorldDebug>();
        let cws = client.component_init::<Worldspawn>();
        let cse = client.component_init::<SoundEmitter>();
        let ccc = client.component_init::<CharacterController>();

        for e in server.inspect_entities() {
            let sid = e.id;
            let has_cam = server.has(sid, cam_id);
            let has_mesh = server.has(sid, mesh_id) && server.has(sid, mat_id);
            let has_se = server.has(sid, se_id);
            if !has_cam && !has_mesh && !has_se {
                continue;
            }
            if !server.has(sid, t_id) {
                continue;
            }

            let cid = client.spawn();
            self.map.insert(sid, cid);

            if let Some(t) = copy_component::<TransformComponent>(server, sid, t_id) {
                client.set(cid, ct, t);
            }
            if has_se {
                if let Some(s) = copy_component::<SoundEmitter>(server, sid, se_id) {
                    client.set(cid, cse, s);
                }
            }
            if has_cam {
                if let Some(c) = copy_component::<CameraComponent>(server, sid, cam_id) {
                    client.set(cid, ccam, c);
                }
            }
            // Feet-origin characters: client needs CC for eye-height camera offset.
            if server.has(sid, cc_id) {
                if let Some(cc) = copy_component::<CharacterController>(server, sid, cc_id) {
                    client.set(cid, ccc, cc);
                }
            }
            if has_mesh {
                if let Some(m) = copy_component::<Mesh3D>(server, sid, mesh_id) {
                    client.set(cid, cmesh, m);
                }
                if let Some(m) = copy_component::<Material>(server, sid, mat_id) {
                    client.set(cid, cmat, m);
                }
                if server.has(sid, lm_id) {
                    if let Some(l) = copy_component::<LightmapId>(server, sid, lm_id) {
                        client.set(cid, clm, l);
                    }
                }
                if server.has(sid, dbg_id) {
                    if let Some(d) = copy_component::<BspWorldDebug>(server, sid, dbg_id) {
                        client.set(cid, cdbg, d);
                    }
                }
                if server.has(sid, ws_id) {
                    if let Some(w) = copy_component::<Worldspawn>(server, sid, ws_id) {
                        client.set(cid, cws, w);
                    }
                }
            }
        }
        client.simulate(0.0);
    }

    /// Update mirrored transforms + cameras + CC (cheap per-frame).
    pub fn sync_transforms(&self, server: &World, client: &World) {
        let t_id = server.component_init::<TransformComponent>();
        let cam_id = server.component_init::<CameraComponent>();
        let cc_id = server.component_init::<CharacterController>();
        let ct = client.component_init::<TransformComponent>();
        let ccam = client.component_init::<CameraComponent>();
        let ccc = client.component_init::<CharacterController>();

        for (&sid, &cid) in &self.map {
            if let Some(t) = copy_component::<TransformComponent>(server, sid, t_id) {
                client.set(cid, ct, t);
            }
            if server.has(sid, cam_id) {
                if let Some(c) = copy_component::<CameraComponent>(server, sid, cam_id) {
                    client.set(cid, ccam, c);
                }
            }
            if server.has(sid, cc_id) {
                if let Some(cc) = copy_component::<CharacterController>(server, sid, cc_id) {
                    client.set(cid, ccc, cc);
                }
            }
        }
    }

    /// Client entity mirroring a server entity, if any.
    pub fn client_of(&self, server_entity: u64) -> Option<u64> {
        self.map.get(&server_entity).copied()
    }
}

fn init_client_components(client: &World) {
    use deerpunk_engine_common::components::*;
    client.component_init::<TransformComponent>();
    client.component_init::<Mesh3D>();
    client.component_init::<Material>();
    client.component_init::<CameraComponent>();
    client.component_init::<LightmapId>();
    client.component_init::<BspWorldDebug>();
    client.component_init::<Worldspawn>();
    client.component_init::<Terrain>();
    client.component_init::<TerrainSplatMaterial>();
    client.component_init::<PointLight>();
    client.component_init::<deerpunk_engine_common::physics::CharacterController>();
}

fn copy_component<T: Clone + 'static>(world: &World, ent: u64, comp: u64) -> Option<T> {
    let ptr = world.get_raw(ent, comp) as *const T;
    if ptr.is_null() {
        return None;
    }
    Some(unsafe { (*ptr).clone() })
}
