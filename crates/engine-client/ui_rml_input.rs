//! Map SDL scancodes to RmlUi `Input::KeyIdentifier` values.

use sdl2::keyboard::Scancode;

pub fn sdl_scancode_to_rml(sc: Scancode) -> Option<i32> {
    Some(match sc {
        Scancode::Backspace => 69,
        Scancode::Tab => 70,
        Scancode::Return => 72,
        Scancode::Escape => 81,
        Scancode::PageUp => 86,
        Scancode::PageDown => 87,
        Scancode::End => 88,
        Scancode::Home => 89,
        Scancode::Left => 90,
        Scancode::Up => 91,
        Scancode::Right => 92,
        Scancode::Down => 93,
        Scancode::Delete => 99,
        Scancode::Space => 1,
        Scancode::A => 12,
        Scancode::B => 13,
        Scancode::C => 14,
        Scancode::D => 15,
        Scancode::E => 16,
        Scancode::F => 17,
        Scancode::G => 18,
        Scancode::H => 19,
        Scancode::I => 20,
        Scancode::J => 21,
        Scancode::K => 22,
        Scancode::L => 23,
        Scancode::M => 24,
        Scancode::N => 25,
        Scancode::O => 26,
        Scancode::P => 27,
        Scancode::Q => 28,
        Scancode::R => 29,
        Scancode::S => 30,
        Scancode::T => 31,
        Scancode::U => 32,
        Scancode::V => 33,
        Scancode::W => 34,
        Scancode::X => 35,
        Scancode::Y => 36,
        Scancode::Z => 37,
        Scancode::Num0 => 2,
        Scancode::Num1 => 3,
        Scancode::Num2 => 4,
        Scancode::Num3 => 5,
        Scancode::Num4 => 6,
        Scancode::Num5 => 7,
        Scancode::Num6 => 8,
        Scancode::Num7 => 9,
        Scancode::Num8 => 10,
        Scancode::Num9 => 11,
        Scancode::Minus => 41,
        Scancode::Equals => 39,
        Scancode::LeftBracket => 45,
        Scancode::RightBracket => 47,
        Scancode::Backslash => 46,
        Scancode::Semicolon => 38,
        Scancode::Apostrophe => 48,
        Scancode::Grave => 44,
        Scancode::Comma => 40,
        Scancode::Period => 42,
        Scancode::Slash => 43,
        _ => return None,
    })
}

pub fn feed_rml_text(ui: &deerpunk_ui::UiRuntime, text: &str) {
    for ch in text.chars() {
        ui.text_input(ch as u32);
    }
}

pub fn feed_rml_keys(ui: &deerpunk_ui::UiRuntime, keys: impl Iterator<Item = (i32, bool)>) {
    for (sdl, down) in keys {
        let Some(sc) = Scancode::from_i32(sdl) else {
            continue;
        };
        let Some(rml) = sdl_scancode_to_rml(sc) else {
            continue;
        };
        if down {
            ui.key_down(rml);
        } else {
            ui.key_up(rml);
        }
    }
}
