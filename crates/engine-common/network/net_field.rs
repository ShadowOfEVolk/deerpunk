//! Network field registry — aliases onto [`PropertyRegistry`] / [`PropertyId`].

pub use crate::property::{
    register_engine_layout_properties as register_engine_layout_net_fields,
    sync_properties_from_layout as sync_net_fields_from_layout, PropertyDescriptor, PropertyId,
    PropertyRegistry, PropertyStorage,
};

/// Back-compat name for replicated field ids (same as [`PropertyId`]).
pub type NetFieldId = PropertyId;

pub type FieldPathId = PropertyId;

pub type NetFieldKind = PropertyStorage;

pub type NetFieldDescriptor = PropertyDescriptor;

pub type NetFieldRegistry = PropertyRegistry;
