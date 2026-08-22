use std::any::TypeId;
use std::collections::HashMap;

use hecs::{Entity, World};

/// Трекер изменений компонентов поверх hecs.
/// Записывает какие компоненты были добавлены/удалены за кадр.
pub struct ChangeTracker {
    added: HashMap<TypeId, Vec<Entity>>,
    removed: HashMap<TypeId, Vec<Entity>>,
}

impl ChangeTracker {
    pub fn new() -> Self {
        Self {
            added: HashMap::new(),
            removed: HashMap::new(),
        }
    }

    pub fn track_add<T: 'static>(&mut self, entity: Entity) {
        self.added.entry(TypeId::of::<T>()).or_default().push(entity);
    }

    pub fn track_remove<T: 'static>(&mut self, entity: Entity) {
        self.removed.entry(TypeId::of::<T>()).or_default().push(entity);
    }

    pub fn was_added<T: 'static>(&self, entity: Entity) -> bool {
        self.added
            .get(&TypeId::of::<T>())
            .is_some_and(|v| v.contains(&entity))
    }

    pub fn was_removed<T: 'static>(&self, entity: Entity) -> bool {
        self.removed
            .get(&TypeId::of::<T>())
            .is_some_and(|v| v.contains(&entity))
    }

    pub fn added_entities<T: 'static>(&self) -> &[Entity] {
        self.added
            .get(&TypeId::of::<T>())
            .map_or(&[], |v| v.as_slice())
    }

    pub fn removed_entities<T: 'static>(&self) -> &[Entity] {
        self.removed
            .get(&TypeId::of::<T>())
            .map_or(&[], |v| v.as_slice())
    }

    pub fn clear(&mut self) {
        self.added.clear();
        self.removed.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

impl Default for ChangeTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Insert + трекинг добавления.
pub fn insert_tracked<T: hecs::Component + Clone>(
    world: &mut World,
    entity: Entity,
    component: T,
    tracker: &mut ChangeTracker,
) {
    let _ = world.insert_one(entity, component);
    tracker.track_add::<T>(entity);
}

/// Remove + трекинг удаления.
pub fn remove_tracked<T: hecs::Component>(
    world: &mut World,
    entity: Entity,
    tracker: &mut ChangeTracker,
) {
    let _ = world.remove_one::<T>(entity);
    tracker.track_remove::<T>(entity);
}
