/// Entity handle — opaque u64 wrapping hecs::Entity bits.
pub type EntityId = u64;

/// Type-erased system closure: receives world reference + delta time.
pub type SystemFn = Box<dyn FnMut(&mut hecs::World, f32) + Send>;

/// Access to the ECS world: entity lifecycle, component management,
/// system registration, and prefab spawning.
pub trait WorldApi {
    // ── Registration ──────────────────────────────────────────

    /// Register a component type by name for prefab deserialization.
    fn components_register(&self, type_name: &str);

    /// Register a system closure that runs each frame on the world.
    fn add_system(&self, system: SystemFn);

    // ── Entity lifecycle ──────────────────────────────────────

    /// Spawn an empty entity. Returns its id.
    fn spawn(&self) -> EntityId;

    /// Destroy an entity and all its components.
    fn destroy(&self, entity: EntityId);

    /// Spawn entities from a prefab file at `path`.
    fn spawn_prefab(&self, path: &str) -> Vec<EntityId>;

    /// Remove all entities from the world.
    fn clear(&self);

    // ── Component access ──────────────────────────────────────

    /// Add a component to an existing entity.
    fn components_add(&self, entity: EntityId, type_name: &str, data: &[u8]);

    /// Read a component's raw bytes (copy).
    fn components_get(&self, entity: EntityId, type_name: &str) -> Option<Vec<u8>>;

    /// Mutable access to a component's raw bytes (copy).
    fn components_get_mut(&self, entity: EntityId, type_name: &str) -> Option<Vec<u8>>;

    /// Remove a component from an entity.
    fn components_remove(&self, entity: EntityId, type_name: &str);

    // ── Queries ───────────────────────────────────────────────

    /// Entity ids that possess every component in `require`
    /// and none in `exclude`.
    fn query(&self, require: &[&str], exclude: &[&str]) -> Vec<EntityId>;

    /// Count entities matching the query.
    fn query_count(&self, require: &[&str], exclude: &[&str]) -> usize;

    /// Check if a specific entity matches the query.
    fn query_matches(&self, entity: EntityId, require: &[&str], exclude: &[&str]) -> bool;
}
