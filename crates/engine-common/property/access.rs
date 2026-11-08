//! Read/write ECS properties by [`PropertyId`] with channel checks.

use crate::layout_meta::{FieldFlags, WireKind};
use crate::network::field_apply;
use crate::network::field_path::FieldValue;
use crate::property::id::PropertyId;
use crate::property::registry::{PropertyRegistry, PropertyStorage};
use crate::world::{EntityId, World};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PropertyChannel {
    Prefab,
    Inspect,
    ReplicateHot,
    ReplicateCold,
    Save,
    Script,
}

impl PropertyChannel {
    pub fn allowed(self, flags: FieldFlags) -> bool {
        match self {
            Self::Prefab => flags.contains(FieldFlags::PREFAB),
            Self::Inspect => flags.contains(FieldFlags::INSPECT),
            Self::ReplicateHot => flags.contains(FieldFlags::REPLICATE_HOT),
            Self::ReplicateCold => flags.contains(FieldFlags::REPLICATE_COLD),
            Self::Save => flags.contains(FieldFlags::SAVE),
            Self::Script => flags.contains(FieldFlags::SCRIPT),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PropertyError {
    UnknownId,
    UnknownPath,
    ChannelDenied,
    MissingComponent,
    WriteFailed,
}

pub fn read_by_path(
    world: &World,
    entity: EntityId,
    path: &str,
) -> Result<FieldValue, PropertyError> {
    let id = PropertyRegistry::global()
        .lock()
        .map_err(|_| PropertyError::UnknownId)?
        .resolve_path(path)
        .ok_or(PropertyError::UnknownPath)?;
    read(world, entity, id)
}

pub fn read(world: &World, entity: EntityId, id: PropertyId) -> Result<FieldValue, PropertyError> {
    PropertyRegistry::global()
        .lock()
        .map_err(|_| PropertyError::UnknownId)?
        .get(id)
        .ok_or(PropertyError::UnknownId)?;
    field_apply::read_field(world, entity, id).ok_or(PropertyError::MissingComponent)
}

pub fn write_by_path(
    world: &World,
    entity: EntityId,
    path: &str,
    value: FieldValue,
    channel: PropertyChannel,
) -> Result<(), PropertyError> {
    let id = PropertyRegistry::global()
        .lock()
        .map_err(|_| PropertyError::UnknownId)?
        .resolve_path(path)
        .ok_or(PropertyError::UnknownPath)?;
    write(world, entity, id, value, channel)
}

pub fn write(
    world: &World,
    entity: EntityId,
    id: PropertyId,
    value: FieldValue,
    channel: PropertyChannel,
) -> Result<(), PropertyError> {
    let reg = PropertyRegistry::global()
        .lock()
        .map_err(|_| PropertyError::UnknownId)?;
    let desc = reg.get(id).ok_or(PropertyError::UnknownId)?.clone();
    drop(reg);

    if !channel.allowed(desc.flags) {
        return Err(PropertyError::ChannelDenied);
    }

    if field_apply::apply_field_ex(world, None, None, None, entity, id, &value) {
        return Ok(());
    }

    if write_memory_from_desc(world, entity, &desc.storage, &value) {
        Ok(())
    } else {
        Err(PropertyError::WriteFailed)
    }
}

fn write_memory_from_desc(
    world: &World,
    entity: EntityId,
    storage: &PropertyStorage,
    value: &FieldValue,
) -> bool {
    let PropertyStorage::Memory {
        component_id,
        offset,
        wire,
        ..
    } = storage
    else {
        return false;
    };
    write_memory(world, entity, *component_id, *offset, *wire, value)
}

/// All fields on `component_name` marked [`FieldFlags::SAVE`].
pub fn snapshot_component_save(
    world: &World,
    entity: EntityId,
    component_name: &str,
) -> Vec<(PropertyId, FieldValue)> {
    let reg = match PropertyRegistry::global().lock() {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };
    let ids: Vec<PropertyId> = reg
        .by_id
        .values()
        .filter(|d| d.component_key() == component_name && d.flags.contains(FieldFlags::SAVE))
        .map(|d| d.id)
        .collect();
    drop(reg);

    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Ok(v) = read(world, entity, id) {
            out.push((id, v));
        }
    }
    out
}

fn read_memory(
    world: &World,
    entity: EntityId,
    component_id: crate::world::ComponentId,
    offset: u32,
    wire: WireKind,
) -> Option<FieldValue> {
    if !world.has(entity, component_id) {
        return None;
    }
    let base = world.get_raw(entity, component_id);
    if base.is_null() {
        return None;
    }
    let p = unsafe { base.add(offset as usize) };
    match wire {
        WireKind::U32 => Some(FieldValue::U32(unsafe { *(p as *const u32) })),
        WireKind::U16 => Some(FieldValue::U32(unsafe { *(p as *const u16) as u32 })),
        WireKind::U8 => Some(FieldValue::U32(unsafe { *(p as *const u8) as u32 })),
        WireKind::I32 => Some(FieldValue::U32(unsafe { *(p as *const i32) as u32 })),
        WireKind::F32 => Some(FieldValue::U32(unsafe { (*(p as *const f32)).to_bits() })),
        WireKind::Bool => Some(FieldValue::U32(unsafe { *(p as *const u8) as u32 })),
        WireKind::Vec3 => {
            let x = unsafe { *(p as *const f32) };
            let y = unsafe { *(p.add(4) as *const f32) };
            let z = unsafe { *(p.add(8) as *const f32) };
            Some(FieldValue::Vec3([x, y, z]))
        }
        WireKind::Quat => {
            let mut arr = [0.0f32; 4];
            for i in 0..4 {
                arr[i] = unsafe { *(p.add(i * 4) as *const f32) };
            }
            Some(FieldValue::Quat(arr))
        }
        _ => None,
    }
}

fn write_memory(
    world: &World,
    entity: EntityId,
    component_id: crate::world::ComponentId,
    offset: u32,
    wire: WireKind,
    value: &FieldValue,
) -> bool {
    match wire {
        WireKind::Vec3 | WireKind::Quat | WireKind::U8 | WireKind::U16 | WireKind::U32 => {
            field_apply::apply_memory_at(world, entity, component_id, offset, wire, value)
        }
        WireKind::I32 => {
            let FieldValue::U32(v) = value else {
                return false;
            };
            write_pod_i32(world, entity, component_id, offset, *v as i32)
        }
        WireKind::F32 => {
            let FieldValue::U32(bits) = value else {
                return false;
            };
            write_pod_f32(world, entity, component_id, offset, f32::from_bits(*bits))
        }
        WireKind::Bool => {
            let FieldValue::U32(v) = value else {
                return false;
            };
            write_pod_u8(world, entity, component_id, offset, (*v != 0) as u8)
        }
        _ => false,
    }
}

fn write_pod_u8(
    world: &World,
    entity: EntityId,
    component_id: crate::world::ComponentId,
    offset: u32,
    v: u8,
) -> bool {
    if !world.has(entity, component_id) {
        return false;
    }
    let ptr = world.get_raw(entity, component_id);
    if ptr.is_null() {
        return false;
    }
    unsafe {
        *(ptr.add(offset as usize) as *mut u8) = v;
    }
    true
}

fn write_pod_i32(
    world: &World,
    entity: EntityId,
    component_id: crate::world::ComponentId,
    offset: u32,
    v: i32,
) -> bool {
    if !world.has(entity, component_id) {
        return false;
    }
    let ptr = world.get_raw(entity, component_id);
    if ptr.is_null() {
        return false;
    }
    unsafe {
        *(ptr.add(offset as usize) as *mut i32) = v;
    }
    true
}

fn write_pod_f32(
    world: &World,
    entity: EntityId,
    component_id: crate::world::ComponentId,
    offset: u32,
    v: f32,
) -> bool {
    if !world.has(entity, component_id) {
        return false;
    }
    let ptr = world.get_raw(entity, component_id);
    if ptr.is_null() {
        return false;
    }
    unsafe {
        *(ptr.add(offset as usize) as *mut f32) = v;
    }
    true
}
