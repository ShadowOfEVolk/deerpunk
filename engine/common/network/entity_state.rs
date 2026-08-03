// Entity state storage using serialized components
use std::collections::HashMap;
use super::component_types::ComponentType;
use super::replication::NetworkComponent;

/// Component metadata in the buffer
#[derive(Clone, Copy, Debug)]
struct ComponentInfo {
    component_type: ComponentType,
    offset: usize,
    size: usize,
}

/// Entity state containing all its components in a single buffer
#[derive(Clone)]
pub struct EntityState {
    pub network_id: u16,
    /// Single continuous buffer containing all component data
    buffer: Vec<u8>,
    /// Map from component type to its location in buffer
    component_info: HashMap<ComponentType, ComponentInfo>,
}

impl EntityState {
    pub fn new(network_id: u16) -> Self {
        Self {
            network_id,
            buffer: Vec::new(),
            component_info: HashMap::new(),
        }
    }
    
    /// Add or update a component in the buffer
    pub fn set_component<T: NetworkComponent + Copy>(&mut self, component_type: ComponentType, component: &T) {
        let size = std::mem::size_of::<T>();
        let data = unsafe {
            std::slice::from_raw_parts(
                component as *const _ as *const u8,
                size
            )
        };
        
        if let Some(info) = self.component_info.get_mut(&component_type) {
            // Component already exists - update in place if size matches
            if info.size == size {
                self.buffer[info.offset..info.offset + size].copy_from_slice(data);
            } else {
                // Size changed - need to reallocate
                self.remove_component(component_type);
                self.insert_component(component_type, data);
            }
        } else {
            // New component - append to buffer
            self.insert_component(component_type, data);
        }
    }
    
    /// Insert component at the end of buffer
    fn insert_component(&mut self, component_type: ComponentType, data: &[u8]) {
        let offset = self.buffer.len();
        let size = data.len();
        
        self.buffer.extend_from_slice(data);
        self.component_info.insert(component_type, ComponentInfo {
            component_type,
            offset,
            size,
        });
    }
    
    /// Get component data slice from buffer
    pub fn get_component_data(&self, component_type: ComponentType) -> Option<&[u8]> {
        self.component_info.get(&component_type).map(|info| {
            &self.buffer[info.offset..info.offset + info.size]
        })
    }
    
    /// Get component data as mutable slice
    pub fn get_component_data_mut(&mut self, component_type: ComponentType) -> Option<&mut [u8]> {
        self.component_info.get(&component_type).map(|info| {
            &mut self.buffer[info.offset..info.offset + info.size]
        })
    }
    
    /// Check if component exists
    pub fn has_component(&self, component_type: ComponentType) -> bool {
        self.component_info.contains_key(&component_type)
    }
    
    /// Remove component from buffer (compacts buffer)
    pub fn remove_component(&mut self, component_type: ComponentType) {
        if let Some(info) = self.component_info.remove(&component_type) {
            // Remove from buffer and shift remaining components
            let end = info.offset + info.size;
            self.buffer.drain(info.offset..end);
            
            // Update offsets of components after the removed one
            for (_, comp_info) in self.component_info.iter_mut() {
                if comp_info.offset > info.offset {
                    comp_info.offset -= info.size;
                }
            }
        }
    }
    
    /// Get component bitmask for all present components
    pub fn get_component_bitmask(&self) -> u8 {
        let mut bitmask = 0u8;
        for component_type in self.component_info.keys() {
            bitmask |= 1 << component_type.to_id();
        }
        bitmask
    }
    
    /// Compare with another EntityState and return changed component types
    pub fn get_changed_components(&self, other: &EntityState) -> Vec<ComponentType> {
        let mut changed = Vec::new();
        
        // Check all components in self
        for component_type in self.component_info.keys() {
            let self_data = self.get_component_data(*component_type).unwrap();
            if let Some(other_data) = other.get_component_data(*component_type) {
                if self_data != other_data {
                    changed.push(*component_type);
                }
            } else {
                // Component exists in self but not in other
                changed.push(*component_type);
            }
        }
        
        // Check components that exist in other but not in self (removed)
        for component_type in other.component_info.keys() {
            if !self.component_info.contains_key(component_type) {
                changed.push(*component_type);
            }
        }
        
        changed
    }
    
    /// Get the entire buffer (useful for serialization)
    pub fn buffer(&self) -> &[u8] {
        &self.buffer
    }
    
    /// Get all component types present
    pub fn component_types(&self) -> impl Iterator<Item = ComponentType> + '_ {
        self.component_info.keys().copied()
    }
}

