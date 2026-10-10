//! In-game ImGui console (GoldSrc/Source style). Cvar: `cl_console_show`.

use deerpunk_engine_common::input::{Input, KEY_BACKSPACE, KEY_GRAVE, KEY_RETURN};
use deerpunk_engine_common::logger;

use crate::cmd::CmdSystem;
use crate::cvar::CvarSystem;

const INPUT_CAP: usize = 256;
const TEXT_TMP: usize = 64;

pub struct Console {
    input: [u8; INPUT_CAP],
    input_len: usize,
    grave_down: bool,
    enter_down: bool,
    backspace_down: bool,
    line_z: [u8; logger::LOG_LINE_CAP + 1],
    prompt_z: [u8; INPUT_CAP + 4],
}

impl Console {
    pub fn new() -> Self {
        Self {
            input: [0; INPUT_CAP],
            input_len: 0,
            grave_down: false,
            enter_down: false,
            backspace_down: false,
            line_z: [0; logger::LOG_LINE_CAP + 1],
            prompt_z: [0; INPUT_CAP + 4],
        }
    }

    pub fn visible(cvar: &CvarSystem) -> bool {
        cvar.get_bool_or("cl_console_show", false)
    }

    pub fn set_visible(cvar: &CvarSystem, on: bool) {
        let _ = cvar.set("cl_console_show", if on { "1" } else { "0" });
    }

    /// Call during ImGui frame, **before** gamelib `server_frame`.
    pub fn tick(&mut self, cvar: &CvarSystem, cmd: &CmdSystem, input: &mut Input) {
        let grave = input.is_down(KEY_GRAVE);
        if grave && !self.grave_down {
            let on = !Self::visible(cvar);
            Self::set_visible(cvar, on);
            self.input_len = 0;
            let mut dump = [0u8; TEXT_TMP];
            let _ = input.take_text(&mut dump);
        }
        self.grave_down = grave;

        if !Self::visible(cvar) {
            return;
        }

        let enter = input.is_down(KEY_RETURN);
        let backspace = input.is_down(KEY_BACKSPACE);
        if enter && !self.enter_down {
            self.submit(cmd);
        }
        if backspace && !self.backspace_down {
            self.backspace_char();
        }
        self.enter_down = enter;
        self.backspace_down = backspace;

        let mut tmp = [0u8; TEXT_TMP];
        let n = input.take_text(&mut tmp);
        for &b in &tmp[..n] {
            if b == b'`' || b == b'~' {
                continue;
            }
            if b == b'\n' || b == b'\r' {
                self.submit(cmd);
                continue;
            }
            if self.input_len + 1 < INPUT_CAP {
                self.input[self.input_len] = b;
                self.input_len += 1;
            }
        }

        // Eat gameplay keys while open (keep grave for edge tracking).
        input.clear_keys();
        if grave {
            input.set_key(KEY_GRAVE, true);
        }

        self.draw();
    }

    fn backspace_char(&mut self) {
        if self.input_len == 0 {
            return;
        }
        let mut len = self.input_len - 1;
        while len > 0 && self.input[len] & 0xC0 == 0x80 {
            len -= 1;
        }
        self.input_len = len;
    }

    fn submit(&mut self, cmd: &CmdSystem) {
        if self.input_len == 0 {
            return;
        }
        let line = std::str::from_utf8(&self.input[..self.input_len]).unwrap_or("");
        log::info!("] {line}");
        cmd.add_command(line.to_string());
        cmd.process_buffer();
        self.input_len = 0;
    }

    fn draw(&mut self) {
        unsafe {
            let io = imgui::sys::igGetIO();
            if io.is_null() {
                return;
            }
            let w = (*io).DisplaySize.x;
            let h = (*io).DisplaySize.y * 0.5;

            imgui::sys::igSetNextWindowPos(
                imgui::sys::ImVec2 { x: 0.0, y: 0.0 },
                imgui::sys::ImGuiCond_Always as i32,
                imgui::sys::ImVec2 { x: 0.0, y: 0.0 },
            );
            imgui::sys::igSetNextWindowSize(
                imgui::sys::ImVec2 { x: w, y: h },
                imgui::sys::ImGuiCond_Always as i32,
            );

            let flags = imgui::sys::ImGuiWindowFlags_NoTitleBar
                | imgui::sys::ImGuiWindowFlags_NoResize
                | imgui::sys::ImGuiWindowFlags_NoMove
                | imgui::sys::ImGuiWindowFlags_NoCollapse
                | imgui::sys::ImGuiWindowFlags_NoSavedSettings;

            let mut open = true;
            if !imgui::sys::igBegin(
                b"##cl_console\0".as_ptr() as *const i8,
                &mut open,
                flags as i32,
            ) {
                imgui::sys::igEnd();
                return;
            }

            let line_h = imgui::sys::igGetTextLineHeightWithSpacing();
            imgui::sys::igBeginChild_Str(
                b"##cl_console_log\0".as_ptr() as *const i8,
                imgui::sys::ImVec2 {
                    x: 0.0,
                    y: -line_h - 4.0,
                },
                true,
                imgui::sys::ImGuiWindowFlags_HorizontalScrollbar as i32,
            );

            logger::for_each_line(|bytes| {
                let n = bytes.len().min(logger::LOG_LINE_CAP);
                self.line_z[..n].copy_from_slice(&bytes[..n]);
                self.line_z[n] = 0;
                imgui::sys::igTextUnformatted(
                    self.line_z.as_ptr() as *const i8,
                    self.line_z.as_ptr().add(n) as *const i8,
                );
            });
            imgui::sys::igSetScrollHereY(1.0);
            imgui::sys::igEndChild();

            self.prompt_z[0] = b']';
            self.prompt_z[1] = b' ';
            let n = self.input_len.min(INPUT_CAP - 1);
            self.prompt_z[2..2 + n].copy_from_slice(&self.input[..n]);
            self.prompt_z[2 + n] = b'_';
            self.prompt_z[3 + n] = 0;
            imgui::sys::igTextUnformatted(self.prompt_z.as_ptr() as *const i8, std::ptr::null());

            imgui::sys::igEnd();
        }
    }
}

impl Default for Console {
    fn default() -> Self {
        Self::new()
    }
}
