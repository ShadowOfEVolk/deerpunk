//! Apply prefab JSON to ECS components using registered [`LayoutMeta`] (no gamelib FFI).

use serde_json::Value;

use crate::layout_meta::{FieldMeta, LayoutMeta, WireKind};
use crate::world::{ComponentId, EntityId, World};

pub fn try_apply_prefab_json(
    world: &World,
    entity: EntityId,
    component_id: ComponentId,
    value: &Value,
) -> Result<(), String> {
    let Some(layout) = world.layout_registry().get_by_id(component_id) else {
        return Err("no layout meta".into());
    };
    let Some(obj) = value.as_object() else {
        return Err("prefab value must be object".into());
    };
    ensure_component_slot(world, entity, component_id, &layout)?;
    let base = world.get_raw(entity, component_id);
    if base.is_null() {
        return Err("component slot missing after ensure".into());
    }
    for field in &layout.fields {
        if let Some(v) = obj.get(&field.name) {
            write_field(base, field, v)?;
        }
    }
    Ok(())
}

fn ensure_component_slot(
    world: &World,
    entity: EntityId,
    component_id: ComponentId,
    layout: &LayoutMeta,
) -> Result<(), String> {
    let size = layout.size as usize;
    if size == 0 {
        return Err("layout size 0".into());
    }
    if !world.has(entity, component_id) {
        let buf = vec![0u8; size];
        world.set_raw(
            entity,
            component_id,
            buf.as_ptr() as *const std::ffi::c_void,
            size,
        );
    }
    Ok(())
}

fn write_field(
    base: *const std::ffi::c_void,
    field: &FieldMeta,
    value: &Value,
) -> Result<(), String> {
    let p = unsafe { base.add(field.offset as usize) };
    match field.wire {
        WireKind::U8 => {
            let n = value.as_u64().ok_or("u8 field")? as u8;
            unsafe { *(p as *mut u8) = n };
        }
        WireKind::U16 => {
            let n = value.as_u64().ok_or("u16 field")? as u16;
            unsafe { *(p as *mut u16) = n };
        }
        WireKind::U32 | WireKind::U64 => {
            let n = value.as_u64().ok_or("u32 field")? as u32;
            unsafe { *(p as *mut u32) = n };
        }
        WireKind::I32 => {
            let n = value.as_i64().ok_or("i32 field")? as i32;
            unsafe { *(p as *mut i32) = n };
        }
        WireKind::F32 => {
            let n = value.as_f64().ok_or("f32 field")? as f32;
            unsafe { *(p as *mut f32) = n };
        }
        WireKind::Bool => {
            let b = value.as_bool().ok_or("bool field")?;
            unsafe { *(p as *mut u8) = b as u8 };
        }
        WireKind::Vec3 => {
            let arr = value.as_array().ok_or("vec3 field")?;
            if arr.len() < 3 {
                return Err("vec3 len".into());
            }
            for (i, elem) in arr.iter().take(3).enumerate() {
                let n = elem.as_f64().ok_or("vec3 elem")? as f32;
                unsafe { *(p.add(i * 4) as *mut f32) = n };
            }
        }
        WireKind::Quat => {
            let arr = value.as_array().ok_or("quat field")?;
            if arr.len() < 4 {
                return Err("quat len".into());
            }
            for (i, elem) in arr.iter().take(4).enumerate() {
                let n = elem.as_f64().ok_or("quat elem")? as f32;
                unsafe { *(p.add(i * 4) as *mut f32) = n };
            }
        }
        WireKind::Indirect | WireKind::Void => {}
    }
    Ok(())
}
