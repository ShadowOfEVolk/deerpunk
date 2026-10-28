/// Component that marks entity for network replication
/// Stores network_id and list of replicated component IDs from registry
pub struct NetReplicatedComponent {
    pub network_id: u32,
    /// List of component IDs from registry that are replicated for this entity
    pub replicated_component_ids: Vec<u16>,
}
