use crate::network::component_registry::get_component_type;
use deerpunk_engine_common::network::ComponentType;
use deerpunk_engine_common::{NetReplicatedComponent, SimpleCircleRenderer, TransformComponent};
pub use deerpunk_engine_common::{NetworkComponent, NetworkField};
use hecs::World;
use std::io::Read;

pub struct ClientReplication {}

impl ClientReplication {
    pub fn new() -> Self {
        Self {}
    }

    /// Parse and apply delta entities packet to world
    pub fn parse_delta_entities_packet(
        &self,
        reader: &mut impl Read,
        world: &mut World,
    ) -> Result<(u8, u8), String> {
        // Read frame numbers
        let mut frame_bytes = [0u8; 2];
        reader
            .read_exact(&mut frame_bytes)
            .map_err(|e| format!("Failed to read frame numbers: {}", e))?;
        let frame_from = frame_bytes[0];
        let frame_to = frame_bytes[1];

        // Read entities one by one
        loop {
            let mut network_id_bytes = [0u8; 2];
            if reader.read_exact(&mut network_id_bytes).is_err() {
                break; // End of packet
            }
            let network_id = u16::from_le_bytes(network_id_bytes);

            let mut is_deleted = [0u8; 1];
            if reader.read_exact(&mut is_deleted).is_err() {
                break;
            }

            if is_deleted[0] != 0 {
                // Entity deleted - skip for now
                continue;
            }

            // Read component changes count
            let mut component_count = [0u8; 1];
            if reader.read_exact(&mut component_count).is_err() {
                break;
            }
            let component_count = component_count[0] as usize;

            // Read component IDs
            let mut component_ids = Vec::new();
            for _ in 0..component_count {
                let mut component_id_bytes = [0u8; 2];
                if reader.read_exact(&mut component_id_bytes).is_err() {
                    break;
                }
                component_ids.push(u16::from_le_bytes(component_id_bytes));
            }

            // Find entity by network_id using NetReplicatedComponent
            let mut entity_id_opt = None;
            for (entity_id, net_replicated) in
                world.query_mut::<(hecs::Entity, &mut NetReplicatedComponent)>()
            {
                if net_replicated.network_id as u16 == network_id {
                    entity_id_opt = Some(entity_id);
                    break;
                }
            }

            // Process each component using registry
            for component_id in component_ids {
                if let Some(component_type) = get_component_type(component_id) {
                    let entity_id = match entity_id_opt {
                        Some(id) => id,
                        None => {
                            // Create new entity with NetReplicatedComponent
                            // First component should be Transform (ID 0)
                            if component_type == ComponentType::Transform {
                                let mut transform = TransformComponent::default();
                                self.apply_component_delta_from_zero(reader, &mut transform);

                                // Create NetReplicatedComponent with component_ids from packet
                                let net_replicated = NetReplicatedComponent {
                                    network_id: network_id as u32,
                                    replicated_component_ids: component_ids.clone(),
                                };

                                let new_entity = world.spawn((transform, net_replicated));
                                entity_id_opt = Some(new_entity);
                                new_entity
                            } else {
                                // Skip non-Transform components if entity doesn't exist
                                continue;
                            }
                        }
                    };

                    // Apply component delta using registry-based dispatch
                    self.apply_component_by_type(reader, world, entity_id, component_type)?;
                }
            }

            // Update NetReplicatedComponent with current component list
            if let Some(entity_id) = entity_id_opt {
                if let Ok(mut net_replicated) = world.get::<&mut NetReplicatedComponent>(entity_id) {
                    net_replicated.replicated_component_ids = component_ids.clone();
                }
            }
        }

        Ok((frame_from, frame_to))
    }

    /// Apply component delta by ComponentType using registry
    fn apply_component_by_type(
        &self,
        reader: &mut impl Read,
        world: &mut World,
        entity_id: hecs::Entity,
        component_type: ComponentType,
    ) -> Result<(), String> {
        match component_type {
            ComponentType::Transform => {
                if let Ok(mut transform) = world.get::<&mut TransformComponent>(entity_id) {
                    self.apply_component_delta_from_zero(reader, &mut *transform);
                }
            }
            ComponentType::SimpleCircleRenderer => {
                let renderer_opt = world.get::<&mut SimpleCircleRenderer>(entity_id).ok();
                if let Some(mut renderer) = renderer_opt {
                    self.apply_component_delta_from_zero(reader, &mut *renderer);
                } else {
                    drop(renderer_opt);
                    let mut renderer = SimpleCircleRenderer::default();
                    self.apply_component_delta_from_zero(reader, &mut renderer);
                    world
                        .insert_one(entity_id, renderer)
                        .map_err(|e| format!("Failed to insert SimpleCircleRenderer: {:?}", e))?;
                }
            }
        }
        Ok(())
    }

    /// Apply component delta from zero (empty/default value)
    fn apply_component_delta_from_zero<T: NetworkComponent + Default>(
        &self,
        reader: &mut impl Read,
        component: &mut T,
    ) {
        let mut zero = T::default();
        self.apply_component_delta(reader, &mut zero, component);
    }

    fn apply_component_delta<T: NetworkComponent>(
        &self,
        reader: &mut impl Read,
        from: &T,
        to: &mut T,
    ) {
        let fields = T::declare_fields();

        // Read bitmask
        let mut bitmask_bytes = [0u8; 2];
        reader.read_exact(&mut bitmask_bytes).unwrap();
        let bitmask = u16::from_le_bytes(bitmask_bytes);

        let from_bytes = unsafe {
            std::slice::from_raw_parts(from as *const _ as *const u8, std::mem::size_of::<T>())
        };
        let to_bytes = unsafe {
            std::slice::from_raw_parts_mut(to as *mut _ as *mut u8, std::mem::size_of::<T>())
        };

        // Copy from to to first
        to_bytes.copy_from_slice(from_bytes);

        // Apply changed fields
        for (field_index, field) in fields.iter().enumerate() {
            if (bitmask & (1 << field_index)) != 0 {
                let to_field = &mut to_bytes[field.offset..field.offset + field.size];
                reader.read_exact(to_field).unwrap();
            }
        }
    }
}

deerpunk_engine_common::singleton_system!(client_replication, ClientReplication);
