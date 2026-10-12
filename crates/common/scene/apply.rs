use std::path::Path;

use crate::components::{
    CameraComponent, FallRescue, LuaScript, Material, Mesh3D, NetworkEntity, PendingAlbedoAtlas,
    PlayerMovement, PlayerUse, PointLight, SoundEmitter, TransformComponent, UiDocumentComponent,
};
use crate::network::field_path::FieldPathId;
use crate::physics::CharacterController;
use crate::resource::mesh::Mesh;
use crate::resource::{path_to_file_uri, LoadOpts, ResourceManager};
use crate::world::{EntityId, World};
use glam::{Quat, Vec3};
use serde_json::Value;

pub fn apply_component(
    world: &World,
    gamedir: &Path,
    resources: &mut ResourceManager,
    entity: EntityId,
    name: &str,
    value: &Value,
) -> Result<(), String> {
    match name {
        "UiDocumentComponent" => {
            let path = value
                .get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "UiDocumentComponent.path missing".to_string())?;
            let visible = value
                .get("visible")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let mut comp = UiDocumentComponent::new(path);
            comp.visible = if visible { 1 } else { 0 };
            let id = world.component_init::<UiDocumentComponent>();
            world.set(entity, id, comp);
            Ok(())
        }
        "TransformComponent" => {
            let pos = read_vec3(value.get("position")).unwrap_or(Vec3::ZERO);
            let scale = read_vec3(value.get("scale")).unwrap_or(Vec3::ONE);
            let t = TransformComponent {
                position: pos,
                rotation: Quat::IDENTITY,
                scale,
            };
            let id = world.component_init::<TransformComponent>();
            world.set(entity, id, t);
            Ok(())
        }
        "CameraComponent" => {
            let mut cam = CameraComponent::default();
            if let Some(fov) = value.get("fov_y").and_then(|v| v.as_f64()) {
                // degrees in prefab → radians
                cam.fov_y = (fov as f32).to_radians();
            }
            if let Some(n) = value.get("near").and_then(|v| v.as_f64()) {
                cam.near = n as f32;
            }
            if let Some(f) = value.get("far").and_then(|v| v.as_f64()) {
                cam.far = f as f32;
            }
            if let Some(e) = value.get("eye_height").and_then(|v| v.as_f64()) {
                cam.eye_height = e as f32;
            }
            let id = world.component_init::<CameraComponent>();
            world.set(entity, id, cam);
            Ok(())
        }
        "CharacterController" => {
            let mut cc = CharacterController::hl_standing();
            if let Some(r) = value.get("radius").and_then(|v| v.as_f64()) {
                cc.radius = r as f32;
            }
            if let Some(h) = value.get("half_height").and_then(|v| v.as_f64()) {
                cc.half_height = h as f32;
            }
            if let Some(s) = value.get("step_height").and_then(|v| v.as_f64()) {
                cc.step_height = s as f32;
            }
            if let Some(s) = value.get("max_slope_deg").and_then(|v| v.as_f64()) {
                cc.max_slope_deg = s as f32;
            }
            if let Some(f) = value.get("flags").and_then(|v| v.as_u64()) {
                cc.flags = f as u32;
            }
            if value
                .get("manual_move")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                cc.set_manual_move(true);
            }
            if value
                .get("noclip")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                cc.set_noclip(true);
            }
            let id = world.component_init::<CharacterController>();
            world.set(entity, id, cc);
            Ok(())
        }
        "PlayerUse" => {
            let mut pu = PlayerUse::default();
            if let Some(c) = value.get("cooldown_until").and_then(|v| v.as_f64()) {
                pu.cooldown_until = c as f32;
            }
            let id = world.component_init::<PlayerUse>();
            world.set(entity, id, pu);
            Ok(())
        }
        "FallRescue" => {
            let mut fr = FallRescue::default();
            if let Some(z) = value.get("min_z").and_then(|v| v.as_f64()) {
                fr.min_z = z as f32;
            }
            let id = world.component_init::<FallRescue>();
            world.set(entity, id, fr);
            Ok(())
        }
        "PlayerMovement" => {
            let mut pm = PlayerMovement::default();
            if let Some(v) = value.get("walk_speed").and_then(|v| v.as_f64()) {
                pm.walk_speed = v as f32;
            }
            if let Some(v) = value.get("sprint_mult").and_then(|v| v.as_f64()) {
                pm.sprint_mult = v as f32;
            }
            if let Some(v) = value.get("stamina").and_then(|v| v.as_f64()) {
                pm.stamina = (v as f32).clamp(0.0, 1.0);
            }
            if let Some(v) = value.get("stamina_drain_time").and_then(|v| v.as_f64()) {
                pm.stamina_drain_time = v as f32;
            }
            if let Some(v) = value.get("stamina_regen_standing").and_then(|v| v.as_f64()) {
                pm.stamina_regen_standing = v as f32;
            }
            if let Some(v) = value.get("stamina_regen_moving").and_then(|v| v.as_f64()) {
                pm.stamina_regen_moving = v as f32;
            }
            let id = world.component_init::<PlayerMovement>();
            world.set(entity, id, pm);
            Ok(())
        }
        "SoundEmitter" => {
            let event = value
                .get("event")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let volume = value
                .get("volume")
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0) as f32;
            let autoplay = value
                .get("autoplay")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let id = world.component_init::<SoundEmitter>();
            world.set(entity, id, SoundEmitter::new(event, volume, autoplay));
            Ok(())
        }
        // Prefab-declared Luau module (server attaches after map load).
        "lua" => {
            let module = value
                .get("module")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "lua.module missing".to_string())?;
            let id = world.component_init::<LuaScript>();
            world.set(entity, id, LuaScript::new(module));
            Ok(())
        }
        "Material" => {
            let mut mat = Material::default();
            mat.roughness = 0.9;
            mat.ambient = [0.0, 0.0, 0.0, 0.0];
            mat.sun_dir = [0.0, 0.0, 1.0, 0.0];
            if let Some(c) = value.get("color").and_then(|v| v.as_array()) {
                if c.len() >= 3 {
                    mat.color = [
                        c[0].as_f64().unwrap_or(1.0) as f32,
                        c[1].as_f64().unwrap_or(1.0) as f32,
                        c[2].as_f64().unwrap_or(1.0) as f32,
                        c.get(3).and_then(|x| x.as_f64()).unwrap_or(1.0) as f32,
                    ];
                }
            }
            if let Some(e) = value.get("emission").and_then(|v| v.as_array()) {
                if e.len() >= 3 {
                    mat.emission = [
                        e[0].as_f64().unwrap_or(0.0) as f32,
                        e[1].as_f64().unwrap_or(0.0) as f32,
                        e[2].as_f64().unwrap_or(0.0) as f32,
                        e.get(3).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
                    ];
                }
            }
            let id = world.component_init::<Material>();
            world.set(entity, id, mat);
            Ok(())
        }
        "Mesh3D" => {
            let kind = value.get("mesh").and_then(|v| v.as_str()).unwrap_or("cube");
            let half = read_vec3(value.get("half_extents")).unwrap_or(Vec3::ONE);
            let center = read_vec3(value.get("center")).unwrap_or(Vec3::ZERO);
            let mut obj_fs_path: Option<std::path::PathBuf> = None;
            let mesh = match kind {
                "cube" | "" => Mesh::cube_sized(half, center),
                other if other.ends_with(".fbx")
                    || other.ends_with(".obj")
                    || other.contains(".obj?")
                    || other.starts_with("file://")
                    || other.contains('/')
                    || other.contains('\\') =>
                {
                    // Relative to gamedir, or absolute / file:// URI.
                    let uri = if other.starts_with("file://") {
                        other.to_string()
                    } else {
                        let p = gamedir.join(other.replace('\\', "/"));
                        path_to_file_uri(&p)
                    };
                    if other.contains(".obj") {
                        // Strip query (`?scale=…`) for MTL/texture side-car lookup.
                        let path_part = other.split('?').next().unwrap_or(other);
                        let p = if path_part.starts_with("file://") {
                            std::path::PathBuf::from(path_part.trim_start_matches("file://"))
                        } else {
                            gamedir.join(path_part.replace('\\', "/"))
                        };
                        obj_fs_path = Some(p);
                    }
                    let handle = resources.load::<Mesh>(&uri);
                    handle.try_get().ok_or_else(|| {
                        format!("Mesh3D load failed: {uri} (asset={:?})", handle.id())
                    })?
                }
                other => {
                    return Err(format!("Mesh3D.mesh unsupported: {other}"));
                }
            };
            let id = world.component_init::<Mesh3D>();
            world.set(entity, id, Mesh3D::with_mesh(mesh));
            if let Some(obj_path) = obj_fs_path {
                if let Some(img) = crate::resource::obj::load_obj_diffuse_albedo(&obj_path) {
                    let pending_id = world.component_init::<PendingAlbedoAtlas>();
                    world.set(entity, pending_id, PendingAlbedoAtlas::new(img));
                }
            }
            Ok(())
        }
        "PointLight" => {
            let mut light = PointLight::default();
            if let Some(c) = value.get("color").and_then(|v| v.as_array()) {
                if c.len() >= 3 {
                    light.color = [
                        c[0].as_f64().unwrap_or(1.0) as f32,
                        c[1].as_f64().unwrap_or(0.9) as f32,
                        c[2].as_f64().unwrap_or(0.55) as f32,
                    ];
                }
            }
            if let Some(i) = value.get("intensity").and_then(|v| v.as_f64()) {
                light.intensity = i as f32;
            }
            if let Some(r) = value.get("range").and_then(|v| v.as_f64()) {
                light.range = (r as f32).max(1.0);
            }
            let id = world.component_init::<PointLight>();
            world.set(entity, id, light);
            Ok(())
        }
        "Worldspawn" => Err(
            "Worldspawn BSP load moved to gamelib (darkdemo-bsp); scenes removed".into(),
        ),
        "NetworkEntity" => {
            let mut hot = Vec::new();
            let mut cold = Vec::new();
            if let Some(arr) = value.get("hot").and_then(|v| v.as_array()) {
                for item in arr {
                    let Some(s) = item.as_str() else {
                        return Err("NetworkEntity.hot entry must be string".into());
                    };
                    let path = FieldPathId::parse(s).ok_or_else(|| {
                        format!("NetworkEntity.hot unknown path: {s}")
                    })?;
                    hot.push(path);
                }
            }
            if let Some(arr) = value.get("cold").and_then(|v| v.as_array()) {
                for item in arr {
                    let Some(s) = item.as_str() else {
                        return Err("NetworkEntity.cold entry must be string".into());
                    };
                    let path = FieldPathId::parse(s).ok_or_else(|| {
                        format!("NetworkEntity.cold unknown path: {s}")
                    })?;
                    cold.push(path);
                }
            }
            let id = world.component_init::<NetworkEntity>();
            world.set(entity, id, NetworkEntity::from_paths(&hot, &cold));
            Ok(())
        }
        other => Err(format!("unknown component in prefab/scene: {other}")),
    }
}

fn read_vec3(v: Option<&Value>) -> Option<Vec3> {
    let a = v?.as_array()?;
    if a.len() < 3 {
        return None;
    }
    Some(Vec3::new(
        a[0].as_f64()? as f32,
        a[1].as_f64()? as f32,
        a[2].as_f64()? as f32,
    ))
}
