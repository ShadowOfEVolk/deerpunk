use hecs::World;
use glam::Quat;
use drifter_engine_common::{SimpleEntity, TransformComponent};

const SCREEN_WIDTH: f32 = 800.0;
const SCREEN_HEIGHT: f32 = 600.0;
const ENTITY_RADIUS: f32 = 20.0;

pub fn simple_entity_system(world: &mut World, delta_time: f32) {
    for (_id, (transform, entity)) in world.query_mut::<(&mut TransformComponent, &mut SimpleEntity)>() {
        // Update position
        let mut new_x = transform.position.x + entity.velocity_x * delta_time;
        let mut new_y = transform.position.y + entity.velocity_y * delta_time;
        
        // Bounce off screen edges
        if new_x - ENTITY_RADIUS < 0.0 || new_x + ENTITY_RADIUS > SCREEN_WIDTH {
            entity.velocity_x = -entity.velocity_x;
            new_x = new_x.max(ENTITY_RADIUS).min(SCREEN_WIDTH - ENTITY_RADIUS);
        }
        
        if new_y - ENTITY_RADIUS < 0.0 || new_y + ENTITY_RADIUS > SCREEN_HEIGHT {
            entity.velocity_y = -entity.velocity_y;
            new_y = new_y.max(ENTITY_RADIUS).min(SCREEN_HEIGHT - ENTITY_RADIUS);
        }
        
        transform.position.x = new_x;
        transform.position.y = new_y;
        
        // 10% chance to change rotation
        if rand::random::<f32>() < 0.1 {
            // Random rotation around Z axis (for 2D)
            let angle = rand::random::<f32>() * std::f32::consts::TAU;
            transform.rotation = Quat::from_rotation_z(angle);
        }
    }
}

