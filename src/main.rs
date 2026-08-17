use deerpunk_engine_common::cvar::cvar;
use deerpunk_engine_common::cmd::{cmd, exec_plus_args};
use deerpunk_engine_common::frame_buffer::SharedFrameBuffer;
use deerpunk_engine_core::Engine;
use std::time::{Duration, Instant};

fn main() {
    println!("Starting deerpunk engine...");

    // Initialize cvars FIRST so CLI +args can actually set values
    cvar().initialize();
    cmd().initialize();

    // Process +cvar CLI args (e.g. +r_texturehandle deerpunk_editor_0)
    exec_plus_args(&[]);

    let mut engine = Engine::new();
    let gamedir = cvar().get_string("gamedir").unwrap_or_else(|| "examples/deathmatch/gamedir".to_string());
    println!("Initializing engine with gamedir: {}", gamedir);
    if let Err(e) = engine.initialize(gamedir) {
        eprintln!("Failed to initialize engine: {}", e);
        return;
    }

    // Check if editor requested shared memory frame output
    let shm_name = cvar().get_string("r_texturehandle").filter(|s| !s.is_empty());
    let mut shared_frame = None;
    if let Some(ref name) = shm_name {
        match SharedFrameBuffer::open(name) {
            Ok(buf) => {
                println!("[game] Connected to shared frame buffer: {} ({}x{})",
                    name, buf.width(), buf.height());
                shared_frame = Some(buf);
            }
            Err(e) => {
                eprintln!("[game] Failed to open shared frame buffer: {e}");
            }
        }
    }

    println!("Engine started. Press Ctrl+C to exit.");

    let mut last_frame_time = Instant::now();
    const TARGET_FPS: f32 = 60.0;
    let frame_time: Duration = Duration::from_secs_f32(1.0 / TARGET_FPS);

    let mut frame_count: u32 = 0;

    // Main game loop
    loop {
        let frame_start = Instant::now();
        let delta_time = (frame_start - last_frame_time).as_secs_f32();
        last_frame_time = frame_start;

        // Process engine frame
        if let Err(e) = engine.frame(delta_time) {
            eprintln!("Frame error: {}", e);
            break;
        }

        // If connected to editor, write a test frame to shared memory
        if let Some(ref mut buf) = shared_frame {
            let w = buf.width() as usize;
            let h = buf.height() as usize;
            let mut pixels = vec![0u8; w * h * 4];

            // Draw a simple test pattern: gradient + frame counter marker
            for y in 0..h {
                for x in 0..w {
                    let idx = (y * w + x) * 4;
                    let t = (y as f32 / h as f32 * 255.0) as u8;
                    let s = ((x as f32 / w as f32 * 128.0) + (frame_count as f32 * 2.0) % 128.0) as u8;
                    pixels[idx] = s;       // R
                    pixels[idx + 1] = t;   // G
                    pixels[idx + 2] = 64;  // B
                    pixels[idx + 3] = 255; // A
                }
            }

            buf.write_frame(&pixels);
            frame_count = frame_count.wrapping_add(1);
        }

        // Flush stdout every frame so logs reach the editor when piped
        use std::io::Write;
        let _ = std::io::stdout().flush();

        // Frame rate limiting
        let elapsed = frame_start.elapsed();
        if elapsed < frame_time {
            std::thread::sleep(frame_time - elapsed);
        }
    }

    println!("Shutting down engine...");
    engine.shutdown();
}
