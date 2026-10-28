//! Built-in prefab appliers (engine). Registered on server/client world init.

use std::collections::HashMap;
use std::path::Path;

use crate::components::{
    CameraComponent, LuaScript, MapKv, Material, Mesh3D, NetworkEntity, PendingAlbedoAtlas,
    PointLight, SimpleEntity, SoundEmitter, StudioModel, TransformComponent, UiDocumentComponent,
    Worldspawn,
};
use crate::network::field_path::NetFieldId;
use crate::network::net_field::register_engine_layout_net_fields;
use crate::physics::CharacterController;
use crate::resource::{path_to_file_uri, MeshRegistry, ResourceManager};
use crate::world::{ComponentId, EntityId, NativePrefabApplyFn, World};
use glam::{Quat, Vec3};
use serde_json::Value;

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

fn apply_ui_document(
    world: &World,
    _meshes: &mut MeshRegistry,
    _gamedir: &Path,
    _resources: &ResourceManager,
    entity: EntityId,
    comp: ComponentId,
    value: &Value,
) -> Result<(), String> {
    let path = value
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "UiDocumentComponent.path missing".to_string())?;
    let visible = value
        .get("visible")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let mut comp_val = UiDocumentComponent::new(path);
    comp_val.visible = if visible { 1 } else { 0 };
    world.set(entity, comp, comp_val);
    Ok(())
}

fn apply_transform(
    world: &World,
    _meshes: &mut MeshRegistry,
    _gamedir: &Path,
    _resources: &ResourceManager,
    entity: EntityId,
    comp: ComponentId,
    value: &Value,
) -> Result<(), String> {
    let pos = read_vec3(value.get("position")).unwrap_or(Vec3::ZERO);
    let scale = read_vec3(value.get("scale")).unwrap_or(Vec3::ONE);
    world.set(
        entity,
        comp,
        TransformComponent {
            position: pos,
            rotation: Quat::IDENTITY,
            scale,
        },
    );
    Ok(())
}

fn apply_camera(
    world: &World,
    _meshes: &mut MeshRegistry,
    _gamedir: &Path,
    _resources: &ResourceManager,
    entity: EntityId,
    comp: ComponentId,
    value: &Value,
) -> Result<(), String> {
    let mut cam = CameraComponent::default();
    if let Some(fov) = value.get("fov_y").and_then(|v| v.as_f64()) {
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
    if let Some(y) = value.get("view_yaw").and_then(|v| v.as_f64()) {
        cam.view_yaw = y as f32;
    }
    if let Some(p) = value.get("view_pitch").and_then(|v| v.as_f64()) {
        cam.view_pitch = p as f32;
    }
    if let Some(r) = value.get("view_roll").and_then(|v| v.as_f64()) {
        cam.view_roll = r as f32;
    }
    if value.get("client_view").and_then(|v| v.as_bool()) == Some(true) {
        cam.client_view = true;
    }
    world.set(entity, comp, cam);
    Ok(())
}

fn apply_character_controller(
    world: &World,
    _meshes: &mut MeshRegistry,
    _gamedir: &Path,
    _resources: &ResourceManager,
    entity: EntityId,
    comp: ComponentId,
    value: &Value,
) -> Result<(), String> {
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
    world.set(entity, comp, cc);
    Ok(())
}

fn apply_sound_emitter(
    world: &World,
    _meshes: &mut MeshRegistry,
    _gamedir: &Path,
    _resources: &ResourceManager,
    entity: EntityId,
    comp: ComponentId,
    value: &Value,
) -> Result<(), String> {
    let event = value.get("event").and_then(|v| v.as_str()).unwrap_or("");
    let volume = value.get("volume").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
    let autoplay = value
        .get("autoplay")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    world.set(entity, comp, SoundEmitter::new(event, volume, autoplay));
    Ok(())
}

fn apply_lua_script(
    world: &World,
    _meshes: &mut MeshRegistry,
    _gamedir: &Path,
    _resources: &ResourceManager,
    entity: EntityId,
    comp: ComponentId,
    value: &Value,
) -> Result<(), String> {
    let module = value
        .get("module")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "lua.module missing".to_string())?;
    world.set(entity, comp, LuaScript::new(module));
    Ok(())
}

fn apply_material(
    world: &World,
    _meshes: &mut MeshRegistry,
    _gamedir: &Path,
    _resources: &ResourceManager,
    entity: EntityId,
    comp: ComponentId,
    value: &Value,
) -> Result<(), String> {
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
    world.set(entity, comp, mat);
    Ok(())
}

pub(crate) fn apply_mesh3d(
    world: &World,
    meshes: &mut MeshRegistry,
    gamedir: &Path,
    resources: &ResourceManager,
    entity: EntityId,
    comp: ComponentId,
    value: &Value,
) -> Result<(), String> {
    let kind = value
        .get("mesh")
        .or_else(|| value.get("uri"))
        .and_then(|v| v.as_str())
        .unwrap_or("cube");
    let uri = normalize_mesh_prefab_uri(gamedir, kind)?;
    world.set(entity, comp, Mesh3D::with_uri(&uri));
    if let Some(obj_path) = obj_path_for_mesh_uri(gamedir, kind) {
        if let Some(img) = crate::resource::obj::load_obj_diffuse_albedo(&obj_path) {
            let pending_id = world.component_init::<PendingAlbedoAtlas>();
            world.set(entity, pending_id, PendingAlbedoAtlas::new(img));
        }
    }
    let _ = meshes;
    let _ = resources;
    Ok(())
}

fn normalize_mesh_prefab_uri(gamedir: &Path, kind: &str) -> Result<String, String> {
    if kind.is_empty() || kind == "cube" {
        return Ok("cube".into());
    }
    if kind.starts_with("file://") {
        return Ok(kind.to_string());
    }
    if kind.ends_with(".fbx")
        || kind.ends_with(".obj")
        || kind.contains(".obj?")
        || kind.contains('/')
        || kind.contains('\\')
    {
        let p = gamedir.join(kind.replace('\\', "/"));
        return Ok(path_to_file_uri(&p));
    }
    Err(format!("Mesh3D.mesh unsupported: {kind}"))
}

fn obj_path_for_mesh_uri(gamedir: &Path, kind: &str) -> Option<std::path::PathBuf> {
    if !kind.contains(".obj") {
        return None;
    }
    let path_part = kind.split('?').next().unwrap_or(kind);
    Some(if path_part.starts_with("file://") {
        std::path::PathBuf::from(path_part.trim_start_matches("file://"))
    } else {
        gamedir.join(path_part.replace('\\', "/"))
    })
}

fn apply_point_light(
    world: &World,
    _meshes: &mut MeshRegistry,
    _gamedir: &Path,
    _resources: &ResourceManager,
    entity: EntityId,
    comp: ComponentId,
    value: &Value,
) -> Result<(), String> {
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
    world.set(entity, comp, light);
    Ok(())
}

fn json_value_to_kv_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn apply_map_kv(
    world: &World,
    _meshes: &mut MeshRegistry,
    _gamedir: &Path,
    _resources: &ResourceManager,
    entity: EntityId,
    comp: ComponentId,
    value: &Value,
) -> Result<(), String> {
    let obj = value
        .as_object()
        .ok_or_else(|| "MapKv must be a JSON object".to_string())?;
    let mut map = HashMap::new();
    for (k, v) in obj {
        map.insert(k.clone(), json_value_to_kv_string(v));
    }
    world.set(entity, comp, MapKv::new(map));
    super::map_spawn::post_map_kv_applied(world, entity);
    Ok(())
}

fn apply_studio_model(
    world: &World,
    _meshes: &mut MeshRegistry,
    _gamedir: &Path,
    _resources: &ResourceManager,
    entity: EntityId,
    comp: ComponentId,
    value: &Value,
) -> Result<(), String> {
    let body = value
        .get("body")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .unwrap_or(0);
    let skin = value
        .get("skin")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .unwrap_or(0);
    world.set(entity, comp, StudioModel { body, skin });
    super::studio_present::sync_studio_presentation(world, entity);
    Ok(())
}

fn apply_worldspawn(
    world: &World,
    _meshes: &mut MeshRegistry,
    _gamedir: &Path,
    _resources: &ResourceManager,
    entity: EntityId,
    comp: ComponentId,
    value: &Value,
) -> Result<(), String> {
    let map = value.get("map").and_then(|v| v.as_str()).unwrap_or("");
    world.set(entity, comp, Worldspawn::new(map));
    Ok(())
}

fn apply_network_entity(
    world: &World,
    _meshes: &mut MeshRegistry,
    _gamedir: &Path,
    _resources: &ResourceManager,
    entity: EntityId,
    comp: ComponentId,
    value: &Value,
) -> Result<(), String> {
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
    world.set(
        entity,
        comp,
        NetworkEntity::from_prefab_paths(&hot, &cold, dormant),
    );
    Ok(())
}

fn attach<T: 'static>(world: &World, prefab_key: &str, apply: NativePrefabApplyFn) {
    let id = world.component_init::<T>();
    world.attach_native_prefab_applier(prefab_key, id, apply);
}

/// Register ECS types + prefab JSON appliers for engine built-ins.
fn register_builtin_layouts(world: &World) {
    use crate::layout_meta::studio_model_layout;
    let id = world.component_init::<StudioModel>();
    let layout = studio_model_layout();
    if let Err(e) = world.layout_registry().register(id, layout.clone()) {
        log::warn!("StudioModel layout meta: {e}");
    } else {
        crate::network::net_field::sync_net_fields_from_layout(id, &layout);
    }
}

pub fn register_engine_prefab_appliers(world: &World) {
    register_engine_layout_net_fields(world);
    attach::<UiDocumentComponent>(world, "UiDocumentComponent", apply_ui_document);
    attach::<TransformComponent>(world, "TransformComponent", apply_transform);
    attach::<CameraComponent>(world, "CameraComponent", apply_camera);
    attach::<CharacterController>(world, "CharacterController", apply_character_controller);
    attach::<SoundEmitter>(world, "SoundEmitter", apply_sound_emitter);
    attach::<LuaScript>(world, "lua", apply_lua_script);
    attach::<Material>(world, "Material", apply_material);
    attach::<Mesh3D>(world, "Mesh3D", apply_mesh3d);
    attach::<PointLight>(world, "PointLight", apply_point_light);
    attach::<NetworkEntity>(world, "NetworkEntity", apply_network_entity);
    attach::<StudioModel>(world, "StudioModel", apply_studio_model);
    attach::<MapKv>(world, "MapKv", apply_map_kv);
    attach::<Worldspawn>(world, "Worldspawn", apply_worldspawn);

    // Types used by prefab appliers but not standalone prefab keys:
    world.component_init::<PendingAlbedoAtlas>();
    world.component_init::<SimpleEntity>();
    register_builtin_layouts(world);
    super::studio_present::register_studio_present_observers(world);
}
