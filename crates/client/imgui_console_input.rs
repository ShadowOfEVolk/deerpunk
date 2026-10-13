//! Feed SDL keyboard/text into ImGui IO (in-game console).

use std::ffi::{CStr, CString};
use std::sync::Mutex;

use deerpunk_engine_common::input::Input;
use imgui::sys::{
    ImGuiIO, ImGuiIO_AddInputCharactersUTF8, ImGuiIO_AddKeyEvent, ImGuiKey, ImGuiKey_A,
    ImGuiKey_Backspace, ImGuiKey_Delete, ImGuiKey_DownArrow, ImGuiKey_End, ImGuiKey_Enter,
    ImGuiKey_Escape, ImGuiKey_Home, ImGuiKey_LeftArrow, ImGuiKey_LeftAlt, ImGuiKey_LeftCtrl,
    ImGuiKey_LeftShift, ImGuiKey_RightAlt, ImGuiKey_RightCtrl, ImGuiKey_RightShift,
    ImGuiKey_RightArrow, ImGuiKey_Tab, ImGuiKey_UpArrow, ImGuiKey_0,
};

use crate::app::App;

static CLIPBOARD_GET_BUF: Mutex<Vec<u8>> = Mutex::new(Vec::new());

/// Call once per frame before `igNewFrame` while the console is open.
pub unsafe fn feed(input: &mut Input, app: &App) {
    let io = imgui::sys::igGetIO();
    if io.is_null() {
        return;
    }
    install_clipboard(io, app);
    feed_keys(io, input);
    feed_text(io, input);
}

unsafe fn install_clipboard(io: *mut ImGuiIO, app: &App) {
    let io = &mut *io;
    io.SetClipboardTextFn = Some(imgui_set_clipboard);
    io.GetClipboardTextFn = Some(imgui_get_clipboard);
    io.ClipboardUserData = (app as *const App) as *mut std::ffi::c_void;
}

unsafe extern "C" fn imgui_set_clipboard(user_data: *mut std::ffi::c_void, text: *const i8) {
    if user_data.is_null() || text.is_null() {
        return;
    }
    let app = &*(user_data as *const App);
    let s = CStr::from_ptr(text).to_string_lossy();
    app.set_clipboard_text(&s);
}

unsafe extern "C" fn imgui_get_clipboard(user_data: *mut std::ffi::c_void) -> *const i8 {
    if user_data.is_null() {
        return std::ptr::null();
    }
    let app = &*(user_data as *const App);
    let text = app.clipboard_text().unwrap_or_default();
    let mut buf = CLIPBOARD_GET_BUF.lock().unwrap_or_else(|e| e.into_inner());
    buf.clear();
    buf.extend_from_slice(text.as_bytes());
    buf.push(0);
    buf.as_ptr() as *const i8
}

unsafe fn feed_keys(io: *mut ImGuiIO, input: &Input) {
    for sc in 0u16..256 {
        if let Some(key) = scancode_to_imgui(sc) {
            ImGuiIO_AddKeyEvent(io, key, input.is_down(sc));
        }
    }
    ImGuiIO_AddKeyEvent(io, ImGuiKey_LeftCtrl, input.is_down(224));
    ImGuiIO_AddKeyEvent(io, ImGuiKey_RightCtrl, input.is_down(228));
    ImGuiIO_AddKeyEvent(io, ImGuiKey_LeftShift, input.is_down(225));
    ImGuiIO_AddKeyEvent(io, ImGuiKey_RightShift, input.is_down(229));
    ImGuiIO_AddKeyEvent(io, ImGuiKey_LeftAlt, input.is_down(226));
    ImGuiIO_AddKeyEvent(io, ImGuiKey_RightAlt, input.is_down(230));
}

unsafe fn feed_text(io: *mut ImGuiIO, input: &mut Input) {
    let mut buf = [0u8; 512];
    let n = input.take_text(&mut buf);
    if n == 0 {
        return;
    }
    if let Ok(s) = CString::new(&buf[..n]) {
        ImGuiIO_AddInputCharactersUTF8(io, s.as_ptr());
    }
}

fn scancode_to_imgui(sc: u16) -> Option<ImGuiKey> {
    match sc {
        40 => Some(ImGuiKey_Enter),
        41 => Some(ImGuiKey_Escape),
        42 => Some(ImGuiKey_Backspace),
        43 => Some(ImGuiKey_Tab),
        79 => Some(ImGuiKey_RightArrow),
        80 => Some(ImGuiKey_LeftArrow),
        81 => Some(ImGuiKey_DownArrow),
        82 => Some(ImGuiKey_UpArrow),
        74 => Some(ImGuiKey_Home),
        77 => Some(ImGuiKey_End),
        76 => Some(ImGuiKey_Delete),
        4..=29 => Some(ImGuiKey_A + (sc - 4) as u32),
        30..=39 => Some(ImGuiKey_0 + (sc - 30) as u32),
        _ => None,
    }
}
