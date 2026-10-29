use std::path::Path;

use std::collections::HashMap;

use crate::components::{
    CameraComponent, LuaScript, MapKv, Material, NetworkEntity, PointLight, SoundEmitter,
    TransformComponent, UiDocumentComponent, ViewCameraTag, Worldspawn,
};
use crate::network::field_path::NetFieldId;
use crate::physics::CharacterController;
use crate::resource::{MeshRegistry, ResourceManager};
use crate::world::{EntityId, World};
use glam::{Quat, Vec3};
use serde_json::Value;

pub fn apply_component(
    world: &World,
    meshes: &mut MeshRegistry,
    gamedir: &Path,
    resources: &ResourceManager,
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
            let id = world.component_init::<CameraComponent>();
            world.set(entity, id, cam);
            Ok(())
        }
        "ViewCamera" | "ViewCameraTag" => {
            let id = world.component_init::<ViewCameraTag>();
            world.set(entity, id, ViewCameraTag::marker());
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
        "SoundEmitter" => {
            let event = value.get("event").and_then(|v| v.as_str()).unwrap_or("");
            let volume = value.get("volume").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
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
            let uri = value
                .get("uri")
                .or_else(|| value.get("mesh"))
                .and_then(|v| v.as_str())
                .unwrap_or("cube");
            let mesh_id = world.component_init::<crate::components::Mesh3D>();
            world.set(entity, mesh_id, crate::components::Mesh3D::with_uri(uri));
            Ok(())
        }
        "DrawMaterial" => {
            let uri = value
                .get("uri")
                .or_else(|| value.get("material"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            crate::scene::set_draw_material_uri(world, entity, uri);
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
        "Worldspawn" => {
            let map = value.get("map").and_then(|v| v.as_str()).unwrap_or("");
            let id = world.component_init::<Worldspawn>();
            world.set(entity, id, Worldspawn::new(map));
            Ok(())
        }
        "MapKv" => {
            let obj = value
                .as_object()
                .ok_or_else(|| "MapKv must be a JSON object".to_string())?;
            let mut map = HashMap::new();
            for (k, v) in obj {
                let s = match v {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    Value::Null => String::new(),
                    other => other.to_string(),
                };
                map.insert(k.clone(), s);
            }
            let id = world.component_init::<MapKv>();
            world.set(entity, id, MapKv::new(map));
            crate::scene::map_spawn::post_map_kv_applied(world, entity);
            Ok(())
        }
        "NetworkEntity" => {
            let mut hot = Vec::new();
            let mut cold = Vec::new();
            if let Some(arr) = value.get("hot").and_then(|v| v.as_array()) {
                for item in arr {
                    let Some(s) = item.as_str() else {
                        return Err("NetworkEntity.hot entry must be string".into());
                    };
                    let path = NetFieldId::parse(s)
                        .ok_or_else(|| format!("NetworkEntity.hot unknown path: {s}"))?;
                    hot.push(path);
                }
            }
            if let Some(arr) = value.get("cold").and_then(|v| v.as_array()) {
                for item in arr {
                    let Some(s) = item.as_str() else {
                        return Err("NetworkEntity.cold entry must be string".into());
                    };
                    let path = NetFieldId::parse(s)
                        .ok_or_else(|| format!("NetworkEntity.cold unknown path: {s}"))?;
                    cold.push(path);
                }
            }
            let dormant = value
                .get("dormant")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let id = world.component_init::<NetworkEntity>();
            world.set(
                entity,
                id,
                NetworkEntity::from_prefab_paths(&hot, &cold, dormant),
            );
            Ok(())
        }
        other => Err(format!("unknown component in prefab/scene: {other}")),
    }
}

/// Gamelib-registered applier first, then built-in engine prefab apply.
pub fn apply_prefab_component_value(
    world: &World,
    meshes: &mut MeshRegistry,
    gamedir: &Path,
    resources: &ResourceManager,
    entity: EntityId,
    comp_name: &str,
    value: &Value,
) -> Result<(), String> {
    match world.try_prefab_apply_registered(meshes, gamedir, resources, entity, comp_name, value) {
        Ok(true) => Ok(()),
        Ok(false) => apply_component(world, meshes, gamedir, resources, entity, comp_name, value),
        Err(e) => Err(e),
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
