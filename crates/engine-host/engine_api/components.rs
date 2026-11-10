//! `v1:world:component_register` — gamelib ECS + prefab apply (CBOR).

use deerpunk_engine_api::components::ComponentRegisterDesc;
use deerpunk_engine_common::world::ComponentDescriptor;

use super::resources::read_str;

/// Shared FFI descriptor validation/build (host + facade paths).
pub(crate) fn build_component_descriptor(
    desc: &ComponentRegisterDesc,
) -> Option<ComponentDescriptor> {
    deerpunk_engine_common::install_property_layout_hook();
    let name = read_str(desc.name_ptr, desc.name_len);
    if name.is_empty() {
        log::error!("[world] component_register: empty component name");
        return None;
    }
    if desc.size == 0 {
        log::error!(
            "[world] component_register \"{name}\": size must be > 0 \
             (Flecs cannot store zero-sized components; add a `_tag: u8` field)"
        );
        return None;
    }
    let prefab_apply = desc.apply_fn;
    let inspect_schema = if desc.inspect_schema_ptr.is_null() || desc.inspect_schema_len == 0 {
        Vec::new()
    } else {
        unsafe {
            std::slice::from_raw_parts(desc.inspect_schema_ptr, desc.inspect_schema_len).to_vec()
        }
    };
    let inspect_dump = desc.inspect_dump_fn;
    let layout_meta = if desc.layout_meta_ptr.is_null() || desc.layout_meta_len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(desc.layout_meta_ptr, desc.layout_meta_len).to_vec() }
    };
    Some(ComponentDescriptor {
        name,
        size: desc.size,
        align: desc.align.max(1),
        prefab_apply_user: desc.user_data,
        prefab_apply,
        inspect_schema_cbor: inspect_schema,
        inspect_dump_user: desc.inspect_dump_user,
        inspect_dump_native: None,
        inspect_dump_ffi: inspect_dump,
        layout_meta,
    })
}

/// Main-thread bind: client-world registration only (server components
/// register on the sim thread via the facade bind).
pub(crate) fn register_component_desc(host: &mut crate::Host, desc: &ComponentRegisterDesc) -> u64 {
    #[cfg(any(feature = "client", feature = "full"))]
    if host.client_engine.is_some() {
        return register_client_component_desc(host, desc);
    }
    log::warn!("[world] component_register: no client world on the main host");
    0
}

pub(crate) fn register_client_component_desc(
    host: &crate::Host,
    desc: &ComponentRegisterDesc,
) -> u64 {
    let Some(reg) = build_component_descriptor(desc) else {
        return 0;
    };
    let Some(world) = host.client_engine.as_ref().map(|c| &c.world) else {
        log::trace!(
            "[client_world] component_register: skipped (no client world on this host)"
        );
        return 0;
    };
    world.component_init_raw(&reg)
}
