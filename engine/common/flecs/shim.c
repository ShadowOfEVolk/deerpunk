/* shim.c — Thin wrappers around Flecs macros/inline helpers */
#include "flecs.h"
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>

ecs_entity_t flecs_register_component(
    ecs_world_t *world,
    ecs_entity_t existing_entity,
    const char *name,
    ecs_size_t size,
    ecs_size_t alignment)
{
    ecs_entity_t comp = existing_entity;
    if (!comp) {
        comp = ecs_new(world);
    }
    if (name) {
        ecs_set_name(world, comp, name);
    }
    ecs_entity_t result = 0;
    if (size > 0) {
        result = ecs_component_init(world, &(ecs_component_desc_t){
            .entity = comp,
            .type = {
                .size = size,
                .alignment = alignment,
            }
        });
    }
    return result ? result : comp;
}

const void* flecs_get_id_wrapper(
    ecs_world_t *world,
    ecs_entity_t entity,
    ecs_id_t component)
{
    return ecs_get_id(world, entity, component);
}

int32_t flecs_iter_get_count(const ecs_iter_t *it) {
    return it->count;
}

ecs_entity_t flecs_id_from_name(ecs_world_t *world, const char *name) {
    return ecs_lookup(world, name);
}

const ecs_entity_t* flecs_iter_get_entities_ptr(const ecs_iter_t *it) {
    return it->entities;
}

ecs_entity_t flecs_iter_get_entity_at(const ecs_iter_t *it, int32_t row) {
    return it->entities[row];
}

bool flecs_has_id_wrapper(
    const ecs_world_t *world,
    ecs_entity_t entity,
    ecs_id_t component)
{
    return ecs_has_id(world, entity, component);
}

void* flecs_get_mut_id_wrapper(
    ecs_world_t *world,
    ecs_entity_t entity,
    ecs_id_t component)
{
    return ecs_get_mut_id(world, entity, component);
}

/* Opaque query iterator — kept entirely in C, never copied to Rust. */
typedef struct {
    ecs_world_t *world;
    ecs_query_t *query;
    ecs_iter_t it;
} flecs_query_iter_t;

flecs_query_iter_t* flecs_query_iter_init(
    ecs_world_t *world,
    const ecs_query_t *query)
{
    flecs_query_iter_t *fit = (flecs_query_iter_t*)calloc(1, sizeof(flecs_query_iter_t));
    fit->world = world;
    fit->query = (ecs_query_t*)query;
    fit->it = ecs_query_iter(world, query);
    return fit;
}

bool flecs_query_iter_next(flecs_query_iter_t *fit) {
    return ecs_query_next(&fit->it);
}

int32_t flecs_query_iter_get_count(flecs_query_iter_t *fit) {
    return fit->it.count;
}

ecs_entity_t flecs_query_iter_get_entity_at(flecs_query_iter_t *fit, int32_t row) {
    return fit->it.entities[row];
}

const ecs_entity_t* flecs_query_iter_get_entities_ptr(flecs_query_iter_t *fit) {
    return fit->it.entities;
}

void* flecs_query_iter_field(flecs_query_iter_t *fit, size_t size, int8_t index) {
    return ecs_field_w_size(&fit->it, size, index);
}

void* flecs_query_iter_field_at(flecs_query_iter_t *fit, size_t size, int8_t index, int32_t row) {
    return ecs_field_at_w_size(&fit->it, size, index, row);
}

int8_t flecs_query_iter_get_field_count(flecs_query_iter_t *fit) {
    return fit->it.field_count;
}

ecs_world_t* flecs_query_iter_get_world(flecs_query_iter_t *fit) {
    return fit->it.world;
}

void flecs_query_iter_fini(flecs_query_iter_t *fit) {
    if (fit) {
        free(fit);
    }
}

/* ── PhysicsBodyDesc observer ─────────────────────────────────────────────── */

/* PhysicsBodyDesc layout (must match Rust #[repr(C)]):
 *   u32 body_type, u32 shape_type, f32[4] shape_params,
 *   u32 category_bits, u32 mask_bits, f32 friction,
 *   f32 restitution, f32 density, u32 is_sensor, f32 yaw
 */
#define PHYSICS_BODY_DESC_SIZE (sizeof(uint32_t)*4 + sizeof(float)*7 + sizeof(uint32_t))

/* Rust-side function: creates rapier body and sets RigidBodyHandle + RigidBody. */
extern void flecs_physics_body_register(
    ecs_world_t *world,
    ecs_entity_t entity,
    const void *desc
);

/* Observer callback: fires on PhysicsBodyDesc OnAdd. */
static void on_physics_body_added(ecs_iter_t *it) {
    for (int i = 0; i < it->count; i++) {
        ecs_entity_t entity = it->entities[i];
        const void *desc = ecs_field_at_w_size(it, PHYSICS_BODY_DESC_SIZE, 0, i);
        if (desc) {
            flecs_physics_body_register(it->world, entity, desc);
        }
    }
}

/* Create an OnAdd observer for PhysicsBodyDesc component. */
ecs_entity_t flecs_observe_physics_body(
    ecs_world_t *world,
    ecs_entity_t component_id
) {
    ecs_observer_desc_t desc = {0};
    desc.query.terms[0].id = component_id;
    desc.events[0] = EcsOnAdd;
    desc.callback = on_physics_body_added;
    ecs_entity_t result = ecs_observer_init(world, &desc);
    return result;
}
