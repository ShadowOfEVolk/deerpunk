//! `v1:world:component_register` — gamelib ECS + prefab apply (CBOR).

use deerpunk_engine_api::components::ComponentRegisterDesc;
use deerpunk_engine_common::world::ComponentDescriptor;

use super::resources::read_str;

pub(crate) fn register_component_desc(host: &crate::Host, desc: &ComponentRegisterDesc) -> u64 {
    let name = read_str(desc.name_ptr, desc.name_len);
    if name.is_empty() || desc.size == 0 {
        return 0;
    }
    let apply = desc.apply_fn;
    let prefab_apply = if apply as usize == 0 {
        None
    } else {
        Some(apply)
    };
    let inspect_schema = if desc.inspect_schema_ptr.is_null() || desc.inspect_schema_len == 0 {
        Vec::new()
    } else {
        unsafe {
            std::slice::from_raw_parts(desc.inspect_schema_ptr, desc.inspect_schema_len).to_vec()
        }
    };
    let inspect_dump = desc.inspect_dump_fn;
    let reg = ComponentDescriptor {
        name,
        size: desc.size,
        align: desc.align.max(1),
        prefab_apply_user: desc.user_data,
        prefab_apply,
        inspect_schema_cbor: inspect_schema,
        inspect_dump_user: desc.inspect_dump_user,
        inspect_dump_native: None,
        inspect_dump_ffi: if inspect_dump as usize == 0 {
            None
        } else {
            Some(inspect_dump)
        },
    };
    host.server_engine.world.component_init_raw(&reg)
}

pub(crate) fn register_client_component_desc(
    host: &crate::Host,
    desc: &ComponentRegisterDesc,
) -> u64 {
    let name = read_str(desc.name_ptr, desc.name_len);
    if name.is_empty() || desc.size == 0 {
        return 0;
    }
    let Some(world) = host.client_engine.as_ref().map(|c| &c.world) else {
        return 0;
    };
    let apply = desc.apply_fn;
    let prefab_apply = if apply as usize == 0 {
        None
    } else {
        Some(apply)
    };
    let inspect_schema = if desc.inspect_schema_ptr.is_null() || desc.inspect_schema_len == 0 {
        Vec::new()
    } else {
        unsafe {
            std::slice::from_raw_parts(desc.inspect_schema_ptr, desc.inspect_schema_len).to_vec()
        }
    };
    let inspect_dump = desc.inspect_dump_fn;
    let reg = ComponentDescriptor {
        name,
        size: desc.size,
        align: desc.align.max(1),
        prefab_apply_user: desc.user_data,
        prefab_apply,
        inspect_schema_cbor: inspect_schema,
        inspect_dump_user: desc.inspect_dump_user,
        inspect_dump_native: None,
        inspect_dump_ffi: if inspect_dump as usize == 0 {
            None
        } else {
            Some(inspect_dump)
        },
    };
    world.component_init_raw(&reg)
}
