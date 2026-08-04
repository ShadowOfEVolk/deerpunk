use deerpunk_engine_client::WindowRenderer;
use deerpunk_engine_core::Engine;
use std::time::{Duration, Instant};

fn main() {
    println!("Starting deerpunk engine...");

    // Initialize renderer
    let mut renderer = match WindowRenderer::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to initialize renderer: {}", e);
            return;
        }
    };

    let mut engine = Engine::new();

    // Initialize engine with gamedir
    let gamedir = "examples/deathmatch/gamedir";
    println!("Initializing engine with gamedir: {}", gamedir);
    if let Err(e) = engine.initialize(gamedir) {
        eprintln!("Failed to initialize engine: {}", e);
        return;
    }

    println!("Engine started. Close window or press Ctrl+C to exit.");

    let mut last_frame_time = Instant::now();
    const TARGET_FPS: f32 = 60.0;
    let frame_time: Duration = Duration::from_secs_f32(1.0 / TARGET_FPS);

    // Main game loop
    loop {
        let frame_start = Instant::now();
        let delta_time = (frame_start - last_frame_time).as_secs_f32();
        last_frame_time = frame_start;

        // Process frame
        if let Err(e) = engine.frame(delta_time) {
            eprintln!("Frame error: {}", e);
            break;
        }

        // Update renderer (draw entities from both worlds)
        // #[cfg(feature = "client")]
        if !renderer.update_with_worlds(engine.server_world()) {
            break; // Window closed
        }

        println!("frame");

        // Frame rate limiting
        let elapsed = frame_start.elapsed();
        if elapsed < frame_time {
            std::thread::sleep(frame_time - elapsed);
        }
    }

    println!("Shutting down engine...");
    engine.shutdown();
}
