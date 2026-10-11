use std::path::Path;

use deerpunk_engine_common::bsp30::BspWorldMesh;
use deerpunk_engine_common::components::{
    CameraComponent, Material, Mesh3D, PointLight, TransformComponent, UiDocumentComponent,
};
use deerpunk_engine_common::resource::mesh::Mesh;
use deerpunk_engine_common::resource::{path_to_file_uri, LoadOpts, ResourceManager};
use deerpunk_engine_common::world::{EntityId, World};
use glam::{Quat, Vec3};
use serde_json::Value;

use super::bsp_place::place_bsp_map;

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
            let mesh = match value.get("mesh").and_then(|v| v.as_str()).unwrap_or("cube") {
                "cube" | "" => Mesh::cube(),
                other => {
                    return Err(format!("Mesh3D.mesh unsupported: {other}"));
                }
            };
            let id = world.component_init::<Mesh3D>();
            world.set(entity, id, Mesh3D::with_mesh(mesh));
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
            let map = value
                .get("map")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "Worldspawn.map missing".to_string())?;
            let map = map.trim().trim_end_matches(".bsp");
            let bsp_path = gamedir.join("maps").join(format!("{map}.bsp"));
            let uri = path_to_file_uri(&bsp_path);
            let handle = resources.load_with::<BspWorldMesh>(
                &uri,
                LoadOpts::default().loader("bsp"),
            );
            let loaded = handle.try_get().ok_or_else(|| {
                format!("bsp load failed via ResourceManager: {uri} (asset={:?})", handle.id())
            })?;
            log::info!(
                "[scene] Worldspawn map={map} asset={:?} uri={uri}",
                handle.id()
            );
            place_bsp_map(world, gamedir, resources, entity, loaded, map)?;
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
