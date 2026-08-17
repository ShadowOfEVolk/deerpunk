use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU8, AtomicU32, Ordering};

const KEYMAP_SIZE: usize = 32; // 256 keys

static KEYMAP: [AtomicU8; KEYMAP_SIZE] = {
    const ZERO: AtomicU8 = AtomicU8::new(0);
    [ZERO; KEYMAP_SIZE]
};

static MOUSE_DX: AtomicI32 = AtomicI32::new(0);
static MOUSE_DY: AtomicI32 = AtomicI32::new(0);
static MOUSE_BUTTONS: AtomicU8 = AtomicU8::new(0);

pub type VirtualKey = u16;

fn key_bit(vk: VirtualKey) -> (usize, u8) {
    let byte = (vk / 8) as usize;
    let bit = vk % 8;
    (byte.min(KEYMAP_SIZE - 1), 1 << bit)
}

pub fn set_key(vk: VirtualKey, down: bool) {
    let (byte, bit) = key_bit(vk);
    if byte < KEYMAP_SIZE {
        if down {
            KEYMAP[byte].fetch_or(bit, Ordering::Relaxed);
        } else {
            KEYMAP[byte].fetch_and(!bit, Ordering::Relaxed);
        }
    }
}

pub fn apply_input_state(mouse_dx: f32, mouse_dy: f32, keys: &[u8; KEYMAP_SIZE], mouse_buttons: u8) {
    MOUSE_DX.fetch_add(mouse_dx as i32, Ordering::Relaxed);
    MOUSE_DY.fetch_add(mouse_dy as i32, Ordering::Relaxed);
    MOUSE_BUTTONS.store(mouse_buttons, Ordering::Relaxed);

    for i in 0..KEYMAP_SIZE {
        KEYMAP[i].store(keys[i], Ordering::Relaxed);
    }
}

pub fn is_down(vk: VirtualKey) -> bool {
    let (byte, bit) = key_bit(vk);
    byte < KEYMAP_SIZE && KEYMAP[byte].load(Ordering::Relaxed) & bit != 0
}

pub fn is_pressed(vk: VirtualKey) -> bool {
    is_down(vk)
}

pub fn is_up(vk: VirtualKey) -> bool {
    !is_down(vk)
}

pub fn mouse_dx() -> f32 {
    MOUSE_DX.swap(0, Ordering::Relaxed) as f32
}

pub fn mouse_dy() -> f32 {
    MOUSE_DY.swap(0, Ordering::Relaxed) as f32
}

pub fn mouse_buttons() -> u8 {
    MOUSE_BUTTONS.load(Ordering::Relaxed)
}

pub fn mouse_left() -> bool {
    mouse_buttons() & 1 != 0
}

pub fn mouse_right() -> bool {
    mouse_buttons() & 2 != 0
}

pub fn reset() {
    for i in 0..KEYMAP_SIZE {
        KEYMAP[i].store(0, Ordering::Relaxed);
    }
    MOUSE_DX.store(0, Ordering::Relaxed);
    MOUSE_DY.store(0, Ordering::Relaxed);
    MOUSE_BUTTONS.store(0, Ordering::Relaxed);
}
