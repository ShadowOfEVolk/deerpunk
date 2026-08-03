use std::collections::HashMap;
use std::io::Write;
use hecs::World;
use drifter_engine_common::{TransformComponent, SimpleCircleRenderer};
use crate::network::component_registry::{get_component_id, get_component_type};

use drifter_engine_common::network::FrameNumber;

/// Maximum number of frames to keep in history
const MAX_FRAMES: usize = 64;

/// Entity state in a frame
#[derive(Clone)]
pub struct PacketEntity {
    pub network_id: u16,
    pub transform: TransformComponent,
    pub circle_renderer: Option<SimpleCircleRenderer>,
}

/// Frame snapshot containing all entities
#[derive(Clone)]
pub struct Frame {
    pub frame_number: FrameNumber,
    pub entities: Vec<PacketEntity>,
}

/// Server frame history and client tracking
/// Uses circular buffer with modulo indexing
pub struct ServerFrames {
    frames: [Option<Frame>; MAX_FRAMES],
    frame_start: FrameNumber, // First frame index in buffer
    current_frame: FrameNumber,
    client_known_frames: HashMap<u32, FrameNumber>, // client_id -> known_frame
}

impl ServerFrames {
    pub fn new() -> Self {
        Self {
            frames: [(); MAX_FRAMES].map(|_| None),
            frame_start: 0,
            current_frame: 0,
            client_known_frames: HashMap::new(),
        }
    }

    /// Create a new frame from current world state
    pub fn create_frame(&mut self, world: &mut World) {
        let mut entities = Vec::new();
        
        // Collect all entities with TransformComponent
        // Use query_mut for iteration (hecs allows read-only access via &T in query_mut)
        for (entity_id, transform) in world.query::<&TransformComponent>().iter() {
            // Use entity ID as network_id (in production, use proper network_id component)
            let network_id = entity_id.id() as u16;
            
            // Try to get SimpleCircleRenderer if it exists
            let circle_renderer = world.get::<SimpleCircleRenderer>(entity_id).ok().map(|r| *r);
            
            entities.push(PacketEntity {
                network_id,
                transform: *transform,
                circle_renderer,
            });
        }

        let frame = Frame {
            frame_number: self.current_frame,
            entities,
        };

        // Store frame in circular buffer using modulo
        let buffer_index = self.current_frame as usize % MAX_FRAMES;
        self.frames[buffer_index] = Some(frame);
        
        // Update frame_start if buffer wraps around
        if self.current_frame.wrapping_sub(self.frame_start) as usize >= MAX_FRAMES {
            self.frame_start = self.current_frame.wrapping_sub((MAX_FRAMES - 1) as u8);
        }

        // Increment frame number (wraps around)
        self.current_frame = self.current_frame.wrapping_add(1);
    }

    /// Get current frame number
    pub fn current_frame(&self) -> FrameNumber {
        self.current_frame.wrapping_sub(1) // Last created frame
    }

    /// Update client's known frame
    pub fn set_client_known_frame(&mut self, client_id: u32, known_frame: FrameNumber) {
        self.client_known_frames.insert(client_id, known_frame);
    }

    /// Get client's known frame (defaults to 0 if not set)
    pub fn get_client_known_frame(&self, client_id: u32) -> FrameNumber {
        self.client_known_frames.get(&client_id).copied().unwrap_or(0)
    }

    /// Write delta packet from client's known frame to current frame
    pub fn write_delta_packet(
        &self,
        writer: &mut impl Write,
        client_id: u32,
    ) -> Result<(), String> {
        let known_frame = self.get_client_known_frame(client_id);
        let current_frame = self.current_frame();

        // Find frames
        let from_frame = self.find_frame(known_frame)
            .ok_or_else(|| format!("Frame {} not found", known_frame))?;
        let to_frame = self.find_frame(current_frame)
            .ok_or_else(|| format!("Frame {} not found", current_frame))?;

        // Write frame numbers
        writer.write_all(&[from_frame.frame_number]).map_err(|e| format!("Write error: {}", e))?;
        writer.write_all(&[to_frame.frame_number]).map_err(|e| format!("Write error: {}", e))?;

        // Create maps for efficient lookup
        let mut from_entities: HashMap<u16, &PacketEntity> = HashMap::new();
        for entity in &from_frame.entities {
            from_entities.insert(entity.network_id, entity);
        }

        let mut to_entities: HashMap<u16, &PacketEntity> = HashMap::new();
        for entity in &to_frame.entities {
            to_entities.insert(entity.network_id, entity);
        }

        // Write entities delta
        // First, write entities that exist in 'to' frame
        for entity in &to_frame.entities {
            let network_id = entity.network_id;
            writer.write_all(&network_id.to_le_bytes()).map_err(|e| format!("Write error: {}", e))?;

            let from_entity = from_entities.get(&network_id);

            if let Some(from_entity) = from_entity {
                // Entity exists in both frames - write delta
                writer.write_all(&[0u8]).map_err(|e| format!("Write error: {}", e))?; // is_deleted = 0

                // Collect changed component IDs
                let mut changed_component_ids = Vec::new();
                
                // Check TransformComponent
                if from_entity.transform != entity.transform {
                    if let Some(id) = get_component_id(drifter_engine_common::network::ComponentType::Transform) {
                        changed_component_ids.push(id);
                    }
                }
                
                // Check SimpleCircleRenderer
                if from_entity.circle_renderer != entity.circle_renderer {
                    if let Some(id) = get_component_id(drifter_engine_common::network::ComponentType::SimpleCircleRenderer) {
                        changed_component_ids.push(id);
                    }
                }

                // Write component changes count
                writer.write_all(&[changed_component_ids.len() as u8])
                    .map_err(|e| format!("Write error: {}", e))?;

                // Write component IDs if any changed
                if !changed_component_ids.is_empty() {
                    for component_id in &changed_component_ids {
                        writer.write_all(&component_id.to_le_bytes())
                            .map_err(|e| format!("Write error: {}", e))?;
                    }
                }

                // Write component deltas
                for component_id in &changed_component_ids {
                    if let Some(component_type) = get_component_type(*component_id) {
                        match component_type {
                            drifter_engine_common::network::ComponentType::Transform => {
                                self.write_transform_delta(writer, &from_entity.transform, &entity.transform)?;
                            }
                            drifter_engine_common::network::ComponentType::SimpleCircleRenderer => {
                                self.write_circle_renderer_delta(
                                    writer,
                                    from_entity.circle_renderer.as_ref(),
                                    entity.circle_renderer.as_ref()
                                )?;
                            }
                        }
                    }
                }
            } else {
                // Entity is new - write full state
                writer.write_all(&[0u8]).map_err(|e| format!("Write error: {}", e))?; // is_deleted = 0
                
                // Collect all present component IDs
                let mut present_component_ids = Vec::new();
                
                // TransformComponent always present for new entities
                if let Some(id) = get_component_id(drifter_engine_common::network::ComponentType::Transform) {
                    present_component_ids.push(id);
                }
                
                // SimpleCircleRenderer if present
                if entity.circle_renderer.is_some() {
                    if let Some(id) = get_component_id(drifter_engine_common::network::ComponentType::SimpleCircleRenderer) {
                        present_component_ids.push(id);
                    }
                }

                // Write component count
                writer.write_all(&[present_component_ids.len() as u8])
                    .map_err(|e| format!("Write error: {}", e))?;

                // Write component IDs
                for component_id in &present_component_ids {
                    writer.write_all(&component_id.to_le_bytes())
                        .map_err(|e| format!("Write error: {}", e))?;
                }

                // Write component data from zero (new entity)
                for component_id in &present_component_ids {
                    if let Some(component_type) = get_component_type(*component_id) {
                        match component_type {
                            drifter_engine_common::network::ComponentType::Transform => {
                                let zero = TransformComponent::default();
                                self.write_transform_delta(writer, &zero, &entity.transform)?;
                            }
                            drifter_engine_common::network::ComponentType::SimpleCircleRenderer => {
                                self.write_circle_renderer_delta(
                                    writer,
                                    None,
                                    entity.circle_renderer.as_ref()
                                )?;
                            }
                        }
                    }
                }
            }
        }

        // Write entities that were deleted (exist in 'from' but not in 'to')
        for entity in &from_frame.entities {
            if !to_entities.contains_key(&entity.network_id) {
                writer.write_all(&entity.network_id.to_le_bytes()).map_err(|e| format!("Write error: {}", e))?;
                writer.write_all(&[1u8]).map_err(|e| format!("Write error: {}", e))?; // is_deleted = 1
            }
        }

        Ok(())
    }

    /// Find frame by frame number
    fn find_frame(&self, frame_number: FrameNumber) -> Option<&Frame> {
        // Check if frame is in valid range
        let frame_diff = self.current_frame.wrapping_sub(frame_number);
        if frame_diff as usize > MAX_FRAMES {
            return None; // Frame too old
        }
        
        // Calculate buffer index using modulo
        let buffer_index = frame_number as usize % MAX_FRAMES;
        
        // Check if frame exists at this index
        if let Some(ref frame) = self.frames[buffer_index] {
            if frame.frame_number == frame_number {
                return Some(frame);
            }
        }
        
        None
    }

    /// Write TransformComponent delta
    fn write_transform_delta(
        &self,
        writer: &mut impl Write,
        from: &TransformComponent,
        to: &TransformComponent,
    ) -> Result<(), String> {
        crate::network::replication::server_replication().write_component_delta(writer, from, to);
        
        Ok(())
    }
    
    /// Write SimpleCircleRenderer delta
    fn write_circle_renderer_delta(
        &self,
        writer: &mut impl Write,
        from: Option<&SimpleCircleRenderer>,
        to: Option<&SimpleCircleRenderer>,
    ) -> Result<(), String> {
        let from = from.copied().unwrap_or_default();
        let to = to.copied().unwrap_or_default();
        crate::network::replication::server_replication().write_component_delta(writer, &from, &to);
        
        Ok(())
    }
}

impl Default for ServerFrames {
    fn default() -> Self {
        Self::new()
    }
}

