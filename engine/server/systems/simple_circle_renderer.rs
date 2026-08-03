use hecs::World;
use drifter_engine_common::SimpleCircleRenderer;

pub fn simple_circle_renderer_system(world: &mut World, _delta_time: f32) {
    for (_id, renderer) in world.query_mut::<&mut SimpleCircleRenderer>() {
        // 5% chance to change color
        if rand::random::<f32>() < 0.05 {
            // Random RGBA color
            let r = rand::random::<u8>();
            let g = rand::random::<u8>();
            let b = rand::random::<u8>();
            let a = 255u8;
            renderer.color = ((a as u32) << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32);
        }
        
        // 5% chance to change radius
        if rand::random::<f32>() < 0.05 {
            // Random radius between 5.0 and 50.0
            renderer.radius = 5.0 + rand::random::<f32>() * 45.0;
        }
    }
}



