// Server to Client commands
pub enum SVC {
    Nop = 0,

    /// Send a delta packet of entities to the client
    /// Format:
    /// - u8: frame_from (baseline frame to apply delta from)
    /// - u8: frame_to (target frame)
    /// - u8: has_component_changes (0 = no non-Transform component changes, 1 = has changes)
    ///       This flag allows client to skip component processing when components don't change
    /// [each entity]
    /// - u16: network_id
    /// - u8: is_deleted (0 = exists, 1 = deleted, skip rest if deleted)
    /// - if not deleted:
    ///   - u8: component_bitmask (bit 0 = TransformComponent, bit 1+ = other components)
    ///   - [for each component in bitmask]:
    ///     - u16: field_bitmask (indicates which fields changed)
    ///     - [for each changed field]:
    ///       - field_value (format depends on field type)
    /// 
    /// 
    /// Example (binary representation):
    /// ```
    /// frame_from: 10 (0x0A)
    /// frame_to: 15 (0x0F)
    /// 
    /// Entity 1 (network_id=1):
    ///   network_id: 0x0001 (u16, 2 bytes)
    ///   is_deleted: 0 (1 bit)
    ///   component_bitmask: 0b00000110 (components 1 and 2 changed, 1 byte)
    ///   
    ///   Component 1 (Transform):
    ///     field_bitmask: 0b00000101 (fields 0 and 2 changed: x and z, 1 byte)
    ///     field 0 (x): 100.5 (f32, 4 bytes: 0x42C90000)
    ///     field 2 (z): 50.0 (f32, 4 bytes: 0x42480000)
    ///   
    ///   Component 2 (Health):
    ///     field_bitmask: 0b00000001 (field 0 changed: health, 1 byte)
    ///     field 0 (health): 75 (u8, 1 byte: 0x4B)
    /// 
    /// Entity 2 (network_id=5):
    ///   network_id: 0x0005 (u16, 2 bytes)
    ///   is_deleted: 1 (1 bit) - entity removed, skip rest
    /// 
    /// Total packet size: ~20 bytes
    /// ```
    DeltaPacketEntities = 41,

    /// Sound event RPC (see [`deerpunk_engine_common::network::sound_event::SoundEventRpc`]
    /// for the payload wire format).
    SoundEvent = 42,
}

/// Client to Server commands
pub enum CLC {
    Nop = 0,

    /// Say current frame to the server
    /// Format:
    /// - u8: frame
    CurrentFrame = 1,
}