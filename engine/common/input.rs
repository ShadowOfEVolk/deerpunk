pub type VirtualKey = u16;

const KEYMAP_SIZE: usize = 32;

fn key_bit(vk: VirtualKey) -> (usize, u8) {
    let byte = (vk / 8) as usize;
    let bit = vk % 8;
    (byte.min(KEYMAP_SIZE - 1), 1 << bit)
}

/// USB HID: Keyboard Grave Accent and Tilde (`~` / `` ` ``).
pub const KEY_GRAVE: VirtualKey = 53;
pub const KEY_RETURN: VirtualKey = 40;
pub const KEY_BACKSPACE: VirtualKey = 42;
pub const KEY_ESCAPE: VirtualKey = 41;

const TEXT_RING_CAP: usize = 64;

pub struct Input {
    keymap: [u8; KEYMAP_SIZE],
    mouse_dx: f32,
    mouse_dy: f32,
    mouse_x: f32,
    mouse_y: f32,
    mouse_buttons: u8,
    mouse_buttons_prev: u8,
    /// UTF-8 bytes from SDL TextInput (fixed ring, no heap).
    text_bytes: [u8; TEXT_RING_CAP],
    text_len: u8,
}

impl Default for Input {
    fn default() -> Self {
        Self {
            keymap: [0u8; KEYMAP_SIZE],
            mouse_dx: 0.0,
            mouse_dy: 0.0,
            mouse_x: 0.0,
            mouse_y: 0.0,
            mouse_buttons: 0,
            mouse_buttons_prev: 0,
            text_bytes: [0u8; TEXT_RING_CAP],
            text_len: 0,
        }
    }
}

impl Input {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_key(&mut self, vk: VirtualKey, down: bool) {
        let (byte, bit) = key_bit(vk);
        if byte < KEYMAP_SIZE {
            if down {
                self.keymap[byte] |= bit;
            } else {
                self.keymap[byte] &= !bit;
            }
        }
    }

    pub fn is_down(&self, vk: VirtualKey) -> bool {
        let (byte, bit) = key_bit(vk);
        byte < KEYMAP_SIZE && self.keymap[byte] & bit != 0
    }

    pub fn is_pressed(&self, vk: VirtualKey) -> bool {
        self.is_down(vk)
    }

    pub fn is_up(&self, vk: VirtualKey) -> bool {
        !self.is_down(vk)
    }

    pub fn add_mouse_delta(&mut self, dx: f32, dy: f32) {
        self.mouse_dx += dx;
        self.mouse_dy += dy;
    }

    pub fn set_mouse_position(&mut self, x: f32, y: f32) {
        self.mouse_x = x;
        self.mouse_y = y;
    }

    pub fn mouse_position(&self) -> (f32, f32) {
        (self.mouse_x, self.mouse_y)
    }

    pub fn set_mouse_buttons(&mut self, buttons: u8) {
        self.mouse_buttons = buttons;
    }

    /// Call once per frame after polling events (before consuming pressed/released).
    pub fn end_frame_input(&mut self) {
        self.mouse_buttons_prev = self.mouse_buttons;
    }

    pub fn mouse_button_pressed(&self, button: u8) -> bool {
        let bit = 1u8 << button;
        self.mouse_buttons & bit != 0 && self.mouse_buttons_prev & bit == 0
    }

    pub fn mouse_button_released(&self, button: u8) -> bool {
        let bit = 1u8 << button;
        self.mouse_buttons & bit == 0 && self.mouse_buttons_prev & bit != 0
    }

    pub fn mouse_dx(&self) -> f32 {
        self.mouse_dx
    }

    pub fn mouse_dy(&self) -> f32 {
        self.mouse_dy
    }

    pub fn reset_mouse_delta(&mut self) {
        self.mouse_dx = 0.0;
        self.mouse_dy = 0.0;
    }

    pub fn mouse_buttons(&self) -> u8 {
        self.mouse_buttons
    }

    pub fn mouse_left(&self) -> bool {
        self.mouse_buttons & 1 != 0
    }

    pub fn mouse_right(&self) -> bool {
        self.mouse_buttons & 2 != 0
    }

    pub fn reset(&mut self) {
        self.keymap = [0u8; KEYMAP_SIZE];
        self.mouse_dx = 0.0;
        self.mouse_dy = 0.0;
        self.mouse_x = 0.0;
        self.mouse_y = 0.0;
        self.mouse_buttons = 0;
        self.mouse_buttons_prev = 0;
        self.text_len = 0;
    }

    /// Drop keyboard state (e.g. while in-game console has focus).
    pub fn clear_keys(&mut self) {
        self.keymap = [0u8; KEYMAP_SIZE];
    }

    pub fn push_text(&mut self, text: &str) {
        for &b in text.as_bytes() {
            if (self.text_len as usize) >= TEXT_RING_CAP {
                break;
            }
            self.text_bytes[self.text_len as usize] = b;
            self.text_len += 1;
        }
    }

    /// Drain UTF-8 text queued this frame into `dst`; returns bytes written.
    pub fn take_text(&mut self, dst: &mut [u8]) -> usize {
        let n = (self.text_len as usize).min(dst.len());
        dst[..n].copy_from_slice(&self.text_bytes[..n]);
        self.text_len = 0;
        n
    }
}
