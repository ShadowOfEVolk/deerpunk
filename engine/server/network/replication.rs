pub use deerpunk_engine_common::{NetworkComponent, NetworkField};
use std::io::{Read, Write};

pub struct ServerReplication {}

impl ServerReplication {
    pub fn new() -> Self {
        Self {}
    }

    /// Write component delta from zero (empty/default value)
    pub fn write_component_delta_from_zero<T: NetworkComponent + Default>(
        &self,
        writer: &mut impl Write,
        to: &T,
    ) {
        let zero = T::default();
        self.write_component_delta(writer, &zero, to);
    }

    pub fn write_component_delta<T: NetworkComponent>(
        &self,
        writer: &mut impl Write,
        from: &T,
        to: &T,
    ) {
        let fields = T::declare_fields();

        let from_bytes = unsafe {
            std::slice::from_raw_parts(from as *const _ as *const u8, std::mem::size_of::<T>())
        };
        let to_bytes = unsafe {
            std::slice::from_raw_parts(to as *const _ as *const u8, std::mem::size_of::<T>())
        };

        // Build bitmask first (one pass)
        let mut bitmask: u16 = 0;
        for (field_index, field) in fields.iter().enumerate() {
            let from_field = &from_bytes[field.offset..field.offset + field.size];
            let to_field = &to_bytes[field.offset..field.offset + field.size];

            if from_field != to_field {
                bitmask |= 1 << field_index;
            }
        }

        // Write bitmask first
        println!("Bitmask: {:b}", bitmask);
        writer.write_all(&bitmask.to_le_bytes()).unwrap();

        // Then write changed fields in order
        for (field_index, field) in fields.iter().enumerate() {
            if (bitmask & (1 << field_index)) != 0 {
                let to_field = &to_bytes[field.offset..field.offset + field.size];
                writer.write_all(to_field).unwrap();
            }
        }
    }
}
