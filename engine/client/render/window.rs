use drifter_engine_common::{SimpleCircleRenderer, TransformComponent};
use hecs::World;
use minifb::{Window, WindowOptions};

const WIDTH: usize = 800;
const HEIGHT: usize = 600;

pub struct WindowRenderer {
    window: Window,
    buffer: Vec<u32>,
}

pub struct Canvas<'a> {
    buffer: &'a mut Vec<u32>,
    width: usize,
    height: usize,
}

impl<'a> Canvas<'a> {
    fn new(buffer: &'a mut Vec<u32>, width: usize, height: usize) -> Self {
        Self {
            buffer,
            width,
            height,
        }
    }

    /// Clear canvas with color (ARGB format: 0xAARRGGBB)
    pub fn clear(&mut self, color: u32) {
        self.buffer.fill(color);
    }

    /// Draw a filled circle
    pub fn fill_circle(&mut self, x: i32, y: i32, radius: i32, color: u32) {
        let min_x = (x - radius).max(0) as usize;
        let max_x = (x + radius).min(self.width as i32) as usize;
        let min_y = (y - radius).max(0) as usize;
        let max_y = (y + radius).min(self.height as i32) as usize;

        for py in min_y..max_y {
            for px in min_x..max_x {
                let dx = px as i32 - x;
                let dy = py as i32 - y;
                let distance_squared = dx * dx + dy * dy;

                if distance_squared <= radius * radius {
                    self.buffer[py * self.width + px] = color;
                }
            }
        }
    }

    /// Draw a filled rectangle
    pub fn fill_rect(&mut self, x: i32, y: i32, width: i32, height: i32, color: u32) {
        let min_x = x.max(0) as usize;
        let max_x = (x + width).min(self.width as i32) as usize;
        let min_y = y.max(0) as usize;
        let max_y = (y + height).min(self.height as i32) as usize;

        for py in min_y..max_y {
            for px in min_x..max_x {
                self.buffer[py * self.width + px] = color;
            }
        }
    }

    /// Set pixel color
    pub fn set_pixel(&mut self, x: usize, y: usize, color: u32) {
        if x < self.width && y < self.height {
            self.buffer[y * self.width + x] = color;
        }
    }
}

impl WindowRenderer {
    pub fn new() -> Result<Self, String> {
        let mut window = Window::new("Drifter Engine", WIDTH, HEIGHT, WindowOptions::default())
            .map_err(|e| format!("Failed to create window: {}", e))?;

        window.limit_update_rate(Some(std::time::Duration::from_micros(16600))); // ~60 FPS

        Ok(Self {
            window,
            buffer: vec![0; WIDTH * HEIGHT],
        })
    }

    /// Get canvas for drawing
    pub fn canvas(&mut self) -> Canvas {
        Canvas::new(&mut self.buffer, WIDTH, HEIGHT)
    }

    /// Update renderer and draw entities from both server and client worlds
    pub fn update_with_worlds(
        &mut self,
        server_world: &World,
        client_world: &World,
    ) -> bool {
        // Get canvas and draw
        let mut canvas = self.canvas();
        canvas.clear(0x00000000); // Black background

        // Draw server entity (red, semi-transparent)
        // Use query_mut for iteration (hecs allows read-only access via &T in query_mut)
        for (id, circle_renderer) in server_world.query::<&SimpleCircleRenderer>().iter() {
            let transform = server_world.get::<TransformComponent>(id).unwrap();
            let screen_x = transform.position.x as i32;
            let screen_y = (HEIGHT as f32 - transform.position.y) as i32; // Flip Y axis

            // Red semi-transparent: 0x80FF0000 (50% opacity)
            canvas.fill_circle(
                screen_x, screen_y,
                circle_renderer.radius as i32,
                circle_renderer.color,
            );
        }

        // Draw client entity (blue, semi-transparent)
        for (id, circle_renderer) in client_world.query::<&SimpleCircleRenderer>().iter() {
            let transform = client_world.get::<TransformComponent>(id).unwrap();
            let screen_x = transform.position.x as i32;
            let screen_y = (HEIGHT as f32 - transform.position.y) as i32; // Flip Y axis

            // Blue semi-transparent: 0x800000FF (50% opacity)
            canvas.fill_circle(
                screen_x, screen_y,
                circle_renderer.radius as i32 - 2,
                0x100000FF, // Blue semi-transparent (ARGB)
            );
        }

        self.window
            .update_with_buffer(&self.buffer, WIDTH, HEIGHT)
            .unwrap();

        true
    }

    pub fn is_open(&self) -> bool {
        self.window.is_open()
    }
}
