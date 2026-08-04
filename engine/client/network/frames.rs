// Client-side frame history for handling old deltas
// Server may send delta from a frame client already processed but didn't notify server yet

use deerpunk_engine_common::network::FrameNumber;
use deerpunk_engine_common::{SimpleCircleRenderer, TransformComponent};
use hecs::Entity;

/// Maximum number of frames to keep in history (256 frames)
const MAX_FRAMES: usize = 256;

/// Entity state in a frame (simplified, only what we need for deltas)
#[derive(Clone)]
pub struct ClientPacketEntity {
    pub network_id: u16,
    pub transform: Option<TransformComponent>,
    pub circle_renderer: Option<SimpleCircleRenderer>,
}

/// Frame snapshot on client
#[derive(Clone)]
pub struct ClientFrame {
    pub frame_number: FrameNumber,
    pub entities: Vec<ClientPacketEntity>,
}

/// Client frame history for delta processing
pub struct ClientFrames {
    frames: [Option<ClientFrame>; MAX_FRAMES],
    current_frame: FrameNumber,
}

impl ClientFrames {
    pub fn new() -> Self {
        Self {
            frames: [(); MAX_FRAMES].map(|_| None),
            current_frame: 0,
        }
    }

    /// Create a new frame from current world state
    pub fn create_frame(&mut self, world: &mut hecs::World) {
        let mut entities = Vec::new();

        // Collect all entities with TransformComponent
        for (entity_id, transform) in world.query::<(Entity, &TransformComponent)>().iter() {
            let network_id = entity_id.id() as u16;

            let circle_renderer = world
                .get::<&SimpleCircleRenderer>(entity_id)
                .ok()
                .map(|r| *r);

            entities.push(ClientPacketEntity {
                network_id,
                transform: Some(*transform),
                circle_renderer,
            });
        }

        let frame = ClientFrame {
            frame_number: self.current_frame,
            entities,
        };

        // Store frame in circular buffer
        let buffer_index = self.current_frame as usize % MAX_FRAMES;
        self.frames[buffer_index] = Some(frame);

        // Increment frame number (wraps around)
        self.current_frame = self.current_frame.wrapping_add(1);
    }

    /// Get current frame number
    pub fn current_frame(&self) -> FrameNumber {
        self.current_frame.wrapping_sub(1) // Last created frame
    }

    /// Find frame by frame number
    pub fn find_frame(&self, frame_number: FrameNumber) -> Option<&ClientFrame> {
        // Check if frame is in valid range
        let frame_diff = self.current_frame.wrapping_sub(frame_number);
        if frame_diff as usize > MAX_FRAMES {
            return None; // Frame too old
        }

        // Calculate buffer index using modulo
        let buffer_index = frame_number as usize % MAX_FRAMES;

        // Check if frame exists at this index
        if let Some(frame) = &self.frames[buffer_index] {
            if frame.frame_number == frame_number {
                return Some(frame);
            }
        }

        None
    }
}

impl Default for ClientFrames {
    fn default() -> Self {
        Self::new()
    }
}
