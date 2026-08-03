// Server-side component registry
// Maps component_id (u16) to ComponentType
// Order must match CLIENT_COMPONENT_REGISTRY!

use drifter_engine_common::network::ComponentType;

pub const SERVER_COMPONENT_REGISTRY: &[ComponentType] = &[
    ComponentType::Transform,
    ComponentType::SimpleCircleRenderer,
];

/// Get component type by ID from registry
pub fn get_component_type(component_id: u16) -> Option<ComponentType> {
    SERVER_COMPONENT_REGISTRY.get(component_id as usize).copied()
}

/// Get component ID from registry
pub fn get_component_id(component_type: ComponentType) -> Option<u16> {
    SERVER_COMPONENT_REGISTRY.iter()
        .position(|&ct| ct == component_type)
        .map(|idx| idx as u16)
}



