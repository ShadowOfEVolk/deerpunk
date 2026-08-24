/*
 * Thin C shim for Flecs — exposes helpers that are inline-only in flecs.h
 * so we can link to them from Rust FFI without relying on struct layout assumptions.
 */
#include "flecs.h"

ecs_entity_t flecs_iter_entities_first(ecs_iter_t *it) {
    return it->entities[0];
}

int32_t flecs_iter_get_count(ecs_iter_t *it) {
    return it->count;
}

int8_t flecs_iter_get_field_count(ecs_iter_t *it) {
    return it->field_count;
}

ecs_world_t* flecs_iter_get_world(ecs_iter_t *it) {
    return it->world;
}

const ecs_entity_t* flecs_iter_get_entities_ptr(ecs_iter_t *it) {
    return it->entities;
}

ecs_entity_t flecs_iter_get_entity_at(ecs_iter_t *it, int32_t row) {
    return it->entities[row];
}

ecs_entity_t flecs_iter_get_system(ecs_iter_t *it) {
    return it->system;
}

ecs_entity_t flecs_register_component(
    ecs_world_t *world,
    ecs_entity_t existing_entity,
    const char *name,
    int32_t size,
    int32_t alignment)
{
    ecs_entity_t comp = existing_entity;
    if (!comp) {
        comp = ecs_new(world);
    }
    if (name) {
        ecs_set_name(world, comp, name);
    }
    ecs_entity_t result = ecs_component_init(world, &(ecs_component_desc_t){
        .entity = comp,
        .type = {
            .size = size,
            .alignment = alignment,
        }
    });
    return result ? result : comp;
}
