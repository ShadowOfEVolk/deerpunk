//! Engine component inspector: JSON Schema + native field dumps.

use serde_json::{json, Value};

use crate::component_inspect::NativeInspectDumpFn;
use crate::components::{
    CameraComponent, LuaScript, MapKv, Material, Mesh3D, NetworkEntity, PointLight, SkinnedMesh3D,
    SoundEmitter, StudioModel, TransformComponent, UiDocumentComponent,
};
use crate::material::DrawMaterial;
use crate::physics::CharacterController;
use crate::world::{ComponentId, EntityId, World};

fn reg(
    world: &World,
    name: &str,
    component_id: ComponentId,
    schema: Value,
    dump: NativeInspectDumpFn,
) {
    if let Err(e) = world.register_native_inspect(name, component_id, schema, dump) {
        log::warn!("[inspect] register {name}: {e}");
    }
}

fn dump_transform(world: &World, entity: EntityId, comp: ComponentId) -> Result<Value, String> {
    let t = world
        .get::<TransformComponent>(entity, comp)
        .ok_or("TransformComponent missing")?;
    Ok(json!({
        "position": [t.position.x, t.position.y, t.position.z],
        "rotation": [t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w],
        "scale": [t.scale.x, t.scale.y, t.scale.z],
    }))
}

fn dump_camera(world: &World, entity: EntityId, comp: ComponentId) -> Result<Value, String> {
    let c = world
        .get::<CameraComponent>(entity, comp)
        .ok_or("CameraComponent missing")?;
    Ok(json!({
        "fov_y": c.fov_y.to_degrees(),
        "near": c.near,
        "far": c.far,
    }))
}

fn dump_character_controller(
    world: &World,
    entity: EntityId,
    comp: ComponentId,
) -> Result<Value, String> {
    let cc = world
        .get::<CharacterController>(entity, comp)
        .ok_or("CharacterController missing")?;
    Ok(json!({
        "radius": cc.radius,
        "half_height": cc.half_height,
        "step_height": cc.step_height,
        "max_slope_deg": cc.max_slope_deg,
        "flags": cc.flags,
    }))
}

fn dump_lua(world: &World, entity: EntityId, comp: ComponentId) -> Result<Value, String> {
    let s = world
        .get::<LuaScript>(entity, comp)
        .ok_or("LuaScript missing")?;
    Ok(json!({
        "module": s.module_str(),
    }))
}

fn dump_ui_document(world: &World, entity: EntityId, comp: ComponentId) -> Result<Value, String> {
    let u = world
        .get::<UiDocumentComponent>(entity, comp)
        .ok_or("UiDocumentComponent missing")?;
    Ok(json!({
        "path": u.path_str(),
        "visible": u.visible != 0,
    }))
}

fn dump_point_light(world: &World, entity: EntityId, comp: ComponentId) -> Result<Value, String> {
    let l = world
        .get::<PointLight>(entity, comp)
        .ok_or("PointLight missing")?;
    Ok(json!({
        "color": l.color,
        "intensity": l.intensity,
        "range": l.range,
    }))
}

fn dump_sound_emitter(world: &World, entity: EntityId, comp: ComponentId) -> Result<Value, String> {
    let s = world
        .get::<SoundEmitter>(entity, comp)
        .ok_or("SoundEmitter missing")?;
    Ok(json!({
        "event": s.event_str(),
        "volume": s.volume,
    }))
}

fn dump_studio_model(world: &World, entity: EntityId, comp: ComponentId) -> Result<Value, String> {
    let s = world
        .get::<StudioModel>(entity, comp)
        .ok_or("StudioModel missing")?;
    Ok(json!({
        "body": s.body,
        "skin": s.skin,
    }))
}

fn dump_map_kv(world: &World, entity: EntityId, comp: ComponentId) -> Result<Value, String> {
    if !world.has(entity, comp) {
        return Err("MapKv missing".into());
    }
    let ptr = world.get_raw(entity, comp) as *const MapKv;
    if ptr.is_null() {
        return Err("MapKv missing".into());
    }
    let kv = unsafe { &*ptr };
    let mut obj = serde_json::Map::new();
    for (k, v) in &kv.kv {
        obj.insert(k.clone(), Value::String(v.clone()));
    }
    Ok(Value::Object(obj))
}

fn dump_network_entity(world: &World, entity: EntityId, comp: ComponentId) -> Result<Value, String> {
    let n = world
        .get::<NetworkEntity>(entity, comp)
        .ok_or("NetworkEntity missing")?;
    Ok(json!({
        "net_id": n.net_id,
        "hot_count": n.hot_count,
        "cold_count": n.cold_count,
    }))
}

fn dump_mesh3d(world: &World, entity: EntityId, comp: ComponentId) -> Result<Value, String> {
    let m = world.get::<Mesh3D>(entity, comp).ok_or("Mesh3D missing")?;
    Ok(json!({
        "uri": m.uri_str(),
        "mesh_handle": m.mesh.0,
        "mesh_valid": m.mesh.is_valid(),
    }))
}

fn dump_skinned_mesh3d(world: &World, entity: EntityId, comp: ComponentId) -> Result<Value, String> {
    let m = world
        .get::<SkinnedMesh3D>(entity, comp)
        .ok_or("SkinnedMesh3D missing")?;
    Ok(json!({
        "mesh_handle": m.mesh.0,
        "mesh_valid": m.mesh.is_valid(),
        "bone_buffer": m.bone_buffer,
    }))
}

fn dump_material(world: &World, entity: EntityId, comp: ComponentId) -> Result<Value, String> {
    let ptr = world.get_raw(entity, comp);
    if ptr.is_null() {
        return Err("Material missing".into());
    }
    let m = unsafe { &*(ptr as *const Material) };
    Ok(json!({
        "color": m.color,
        "metallic": m.metallic,
        "roughness": m.roughness,
        "emission": m.emission,
        "sun_dir": m.sun_dir,
        "ambient": m.ambient,
    }))
}

fn dump_draw_material(world: &World, entity: EntityId, comp: ComponentId) -> Result<Value, String> {
    let dm = world
        .get::<DrawMaterial>(entity, comp)
        .ok_or("DrawMaterial missing")?;
    Ok(json!({
        "uri": dm.uri_str(),
        "material_handle": dm.handle.0,
        "material_valid": !dm.handle.is_none(),
    }))
}

/// Call after [`register_engine_prefab_appliers`]: same component ids, inspector metadata.
pub fn register_engine_component_inspect(world: &World) {
    let id = world.component_init::<TransformComponent>();
    reg(
        world,
        "TransformComponent",
        id,
        json!({
            "type": "object",
            "properties": {
                "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
                "rotation": { "type": "array", "items": { "type": "number" }, "minItems": 4, "maxItems": 4 },
                "scale": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 },
            }
        }),
        dump_transform,
    );

    let id = world.component_init::<CameraComponent>();
    reg(
        world,
        "CameraComponent",
        id,
        json!({
            "type": "object",
            "properties": {
                "fov_y": { "type": "number" },
                "near": { "type": "number" },
                "far": { "type": "number" },
            }
        }),
        dump_camera,
    );

    let id = world.component_init::<CharacterController>();
    reg(
        world,
        "CharacterController",
        id,
        json!({ "type": "object" }),
        dump_character_controller,
    );

    let id = world.component_init::<LuaScript>();
    reg(
        world,
        "lua",
        id,
        json!({
            "type": "object",
            "properties": {
                "module": { "type": "string" },
            }
        }),
        dump_lua,
    );
    reg(world, "LuaScript", id, json!({ "type": "object" }), dump_lua);

    let id = world.component_init::<UiDocumentComponent>();
    reg(world, "UiDocumentComponent", id, json!({ "type": "object" }), dump_ui_document);

    let id = world.component_init::<PointLight>();
    reg(world, "PointLight", id, json!({ "type": "object" }), dump_point_light);

    let id = world.component_init::<SoundEmitter>();
    reg(world, "SoundEmitter", id, json!({ "type": "object" }), dump_sound_emitter);

    let id = world.component_init::<NetworkEntity>();
    reg(world, "NetworkEntity", id, json!({ "type": "object" }), dump_network_entity);

    let id = world.component_init::<StudioModel>();
    reg(
        world,
        "StudioModel",
        id,
        json!({
            "type": "object",
            "properties": {
                "body": { "type": "integer" },
                "skin": { "type": "integer" },
            }
        }),
        dump_studio_model,
    );

    let id = world.component_init::<MapKv>();
    reg(world, "MapKv", id, json!({ "type": "object" }), dump_map_kv);

    let id = world.component_init::<Material>();
    reg(
        world,
        "Material",
        id,
        json!({
            "type": "object",
            "properties": {
                "color": { "type": "array" },
                "metallic": { "type": "number" },
                "roughness": { "type": "number" },
            }
        }),
        dump_material,
    );

    let id = world.component_init::<Mesh3D>();
    reg(
        world,
        "Mesh3D",
        id,
        json!({
            "type": "object",
            "properties": {
                "uri": { "type": "string" },
                "mesh_handle": { "type": "integer" },
                "mesh_valid": { "type": "boolean" },
            }
        }),
        dump_mesh3d,
    );

    let id = world.component_init::<SkinnedMesh3D>();
    reg(
        world,
        "SkinnedMesh3D",
        id,
        json!({ "type": "object" }),
        dump_skinned_mesh3d,
    );

    let id = world.component_init::<DrawMaterial>();
    reg(
        world,
        "DrawMaterial",
        id,
        json!({
            "type": "object",
            "properties": {
                "uri": { "type": "string" },
                "material_handle": { "type": "integer" },
            }
        }),
        dump_draw_material,
    );
}
