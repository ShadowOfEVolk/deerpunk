//! Typed property registry: stable [`PropertyId`], [`PropertyType`] (`WireKind`), channel-aware read/write.

mod access;
mod id;
mod registry;

pub use crate::layout_meta::WireKind as PropertyType;
pub use crate::network::field_path::FieldValue as PropertyValue;
pub use access::{
    read, read_by_path, snapshot_component_save, write, write_by_path, PropertyChannel,
    PropertyError,
};
pub use id::PropertyId;
pub use registry::{
    register_engine_layout_properties, sync_properties_from_layout, PropertyDescriptor,
    PropertyRegistry, PropertyStorage,
};
