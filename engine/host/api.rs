use std::any::Any;
use std::sync::Arc;

use hecs::World;

pub type EntityId = u64;
pub type ComponentId = u64;

pub fn entity_to_hecs(id: EntityId) -> hecs::Entity {
    hecs::Entity::from_bits(id).expect("invalid EntityId")
}

pub fn entity_from_hecs(entity: hecs::Entity) -> EntityId {
    entity.to_bits().get()
}

pub struct ComponentEntry {
    pub has: Arc<dyn Fn(&World, hecs::Entity) -> bool + Send + Sync>,
    pub get_boxed: Arc<dyn Fn(&World, hecs::Entity) -> Option<Box<dyn Any>> + Send + Sync>,
    pub set_boxed: Arc<dyn Fn(&mut World, hecs::Entity, Box<dyn Any>) + Send + Sync>,
    pub remove: Arc<dyn Fn(&mut World, hecs::Entity) + Send + Sync>,
}

impl Clone for ComponentEntry {
    fn clone(&self) -> Self {
        Self {
            has: Arc::clone(&self.has),
            get_boxed: Arc::clone(&self.get_boxed),
            set_boxed: Arc::clone(&self.set_boxed),
            remove: Arc::clone(&self.remove),
        }
    }
}
