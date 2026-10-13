//! In-game ImGui console (GoldSrc/Source style). Cvar: `cl_console_show`.

use std::ffi::{c_void, CStr, CString};

use deerpunk_engine_common::input::{Input, KEY_GRAVE};
use deerpunk_engine_common::logger;

use crate::cmd::CmdSystem;
use crate::cvar::CvarSystem;

const INPUT_CAP: usize = 512;
const HISTORY_CAP: usize = 128;
const LINE_TMP: usize = 64;
const MAX_SUGGESTIONS: usize = 12;
/// Visible suggestion rows above the input; extra matches scroll inside the panel.
const SUGGEST_VISIBLE_ROWS: usize = 6;

pub struct Console {
    input_buf: [u8; INPUT_CAP],
    grave_down: bool,
    focus_input_next: bool,
    history: Vec<String>,
    /// Index into `history` for Up/Down navigation (`history.len()` = draft line).
    history_nav: usize,
    completion_names: Vec<String>,
    /// Tab-completion cycle index into current `autocomplete_candidates()`.
    completion_pick: usize,
    /// Auto-scroll log to bottom until the user scrolls up.
    log_follow_tail: bool,
    line_z: [u8; logger::LOG_LINE_CAP + 1],
    /// Set for the duration of `draw` (Tab completion in input callback).
    draw_cmd: *const CmdSystem,
}

impl Console {
    pub fn new() -> Self {
        Self {
            input_buf: [0; INPUT_CAP],
            grave_down: false,
            focus_input_next: false,
            history: Vec::new(),
            history_nav: 0,
            completion_names: Vec::new(),
            completion_pick: 0,
            log_follow_tail: true,
            line_z: [0; logger::LOG_LINE_CAP + 1],
            draw_cmd: std::ptr::null(),
        }
    }

    pub fn visible(cvar: &CvarSystem) -> bool {
        cvar.get_bool_or("cl_console_show", false)
    }

    pub fn set_visible(cvar: &CvarSystem, on: bool) {
        let _ = cvar.set("cl_console_show", if on { "1" } else { "0" });
    }

    /// Toggle / focus. Call before gamelib frame. Does not draw.
    pub fn tick_input(&mut self, cvar: &CvarSystem, input: &mut Input) {
        let grave = input.is_down(KEY_GRAVE);
        if grave && !self.grave_down {
            let on = !Self::visible(cvar);
            Self::set_visible(cvar, on);
            self.input_buf = [0; INPUT_CAP];
            self.history_nav = self.history.len();
            self.focus_input_next = on;
            if on {
                self.log_follow_tail = true;
            }
            self.completion_pick = 0;
            let mut dump = [0u8; LINE_TMP];
            let _ = input.take_text(&mut dump);
        }
        self.grave_down = grave;
    }

    /// ImGui widgets — must run between `imgui_begin_frame` / end.
    pub fn tick_draw(&mut self, cvar: &CvarSystem, cmd: &CmdSystem, cvar_sys: &CvarSystem) {
        if !Self::visible(cvar) {
            return;
        }
        self.refresh_completion_names(cmd, cvar_sys);
        self.draw(cmd, cvar_sys);
    }

    pub fn tick(&mut self, cvar: &CvarSystem, cmd: &CmdSystem, cvar_sys: &CvarSystem, input: &mut Input) {
        self.tick_input(cvar, input);
        self.tick_draw(cvar, cmd, cvar_sys);
    }

    fn refresh_completion_names(&mut self, cmd: &CmdSystem, cvar_sys: &CvarSystem) {
        self.completion_names.clear();
        self.completion_names.extend(cmd.command_names());
        for (name, _) in cvar_sys.dump() {
            if !self.completion_names.iter().any(|n| n == &name) {
                self.completion_names.push(name);
            }
        }
        self.completion_names.sort();
    }

    fn input_str(&self) -> &str {
        let end = self
            .input_buf
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(INPUT_CAP);
        std::str::from_utf8(&self.input_buf[..end]).unwrap_or("")
    }

    fn set_input_str(&mut self, s: &str) {
        let bytes = s.as_bytes();
        let n = bytes.len().min(INPUT_CAP - 1);
        self.input_buf[..n].copy_from_slice(&bytes[..n]);
        self.input_buf[n] = 0;
        for i in n + 1..INPUT_CAP {
            self.input_buf[i] = 0;
        }
    }

    fn push_history(&mut self, line: &str) {
        if line.is_empty() {
            return;
        }
        if self.history.last().map(|s| s.as_str()) == Some(line) {
            return;
        }
        self.history.push(line.to_string());
        if self.history.len() > HISTORY_CAP {
            self.history.remove(0);
        }
        self.history_nav = self.history.len();
    }

    fn submit(&mut self, cmd: &CmdSystem) {
        let line = self.input_str().trim().to_string();
        if line.is_empty() {
            return;
        }
        log::info!(target: "console", "{}", line);
        self.push_history(&line);
        cmd.add_command(line);
        cmd.process_buffer();
        self.set_input_str("");
        self.history_nav = self.history.len();
        self.completion_pick = 0;
        self.log_follow_tail = true;
    }

    fn autocomplete_candidates(&self, cmd: &CmdSystem) -> Vec<String> {
        let line = self.input_str();
        if line.trim().is_empty() {
            return Vec::new();
        }
        cmd.autocomplete(line, &self.completion_names)
    }

    fn apply_completion(&mut self, candidate: &str) {
        self.set_input_str(candidate);
    }

    fn draw(&mut self, cmd: &CmdSystem, _cvar_sys: &CvarSystem) {
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
            push_console_theme();
            self.draw_cmd = cmd as *const CmdSystem;

            let suggestions = self.autocomplete_candidates(cmd);
            if self.completion_pick >= suggestions.len() {
                self.completion_pick = 0;
            }

            let suggest_row_h = imgui::sys::igGetTextLineHeightWithSpacing();
            let suggest_h = if suggestions.is_empty() {
                0.0
            } else {
                let rows = suggestions.len().min(SUGGEST_VISIBLE_ROWS) as f32;
                suggest_row_h * rows + 8.0
            };
            let input_row_h = imgui::sys::igGetFrameHeightWithSpacing() + 6.0;
            let footer_h = input_row_h + suggest_h + 6.0;

            imgui::sys::igBeginChild_Str(
                b"##cl_console_log\0".as_ptr() as *const i8,
                imgui::sys::ImVec2 {
                    x: 0.0,
                    y: -footer_h,
                },
                true,
                (imgui::sys::ImGuiWindowFlags_HorizontalScrollbar
                    | imgui::sys::ImGuiWindowFlags_AlwaysVerticalScrollbar) as i32,
            );

            logger::for_each_line(|bytes| {
                let n = bytes.len().min(logger::LOG_LINE_CAP);
                self.line_z[..n].copy_from_slice(&bytes[..n]);
                self.line_z[n] = 0;
                let text = CStr::from_ptr(self.line_z.as_ptr() as *const i8)
                    .to_string_lossy();
                draw_log_line(&text);
            });

            let scroll_y = imgui::sys::igGetScrollY();
            let scroll_max = imgui::sys::igGetScrollMaxY();
            if scroll_max > 1.0 && scroll_y < scroll_max - 2.0 {
                self.log_follow_tail = false;
            } else if scroll_max > 0.0 && scroll_y >= scroll_max - 2.0 {
                self.log_follow_tail = true;
            }
            if self.log_follow_tail {
                imgui::sys::igSetScrollHereY(1.0);
            }
            imgui::sys::igEndChild();

            if !suggestions.is_empty() {
                imgui::sys::igBeginChild_Str(
                    b"##cl_console_suggest\0".as_ptr() as *const i8,
                    imgui::sys::ImVec2 {
                        x: 0.0,
                        y: suggest_h,
                    },
                    true,
                    imgui::sys::ImGuiWindowFlags_AlwaysVerticalScrollbar as i32,
                );
                for (i, s) in suggestions.iter().enumerate() {
                    let label = CString::new(format!("{s}##suggest{i}")).unwrap_or_default();
                    let selected = i == self.completion_pick;
                    let clicked = imgui::sys::igSelectable_Bool(
                        label.as_ptr(),
                        selected,
                        0,
                        imgui::sys::ImVec2 { x: 0.0, y: 0.0 },
                    );
                    if selected {
                        imgui::sys::igSetScrollHereY(0.5);
                    }
                    if clicked {
                        self.apply_completion(s);
                        self.completion_pick = i;
                    }
                }
                imgui::sys::igEndChild();
            }

            if self.focus_input_next {
                imgui::sys::igSetKeyboardFocusHere(0);
                self.focus_input_next = false;
            }

            imgui::sys::igSetNextItemWidth(-1.0);

            let input_flags = (imgui::sys::ImGuiInputTextFlags_EnterReturnsTrue
                | imgui::sys::ImGuiInputTextFlags_CallbackHistory
                | imgui::sys::ImGuiInputTextFlags_CallbackCompletion
                | imgui::sys::ImGuiInputTextFlags_CallbackEdit
                | imgui::sys::ImGuiInputTextFlags_EscapeClearsAll) as i32;

            let user_data = self as *mut Console as *mut c_void;
            let mut submitted = false;
            if imgui::sys::igInputTextWithHint(
                b"##cl_console_cmd\0".as_ptr() as *const i8,
                b"command - Tab complete, Up/Down history\0".as_ptr() as *const i8,
                self.input_buf.as_mut_ptr() as *mut i8,
                INPUT_CAP,
                input_flags,
                Some(input_callback),
                user_data,
            ) {
                submitted = true;
            }
            if submitted {
                self.submit(cmd);
            }

            pop_console_theme();
            self.draw_cmd = std::ptr::null();
            imgui::sys::igEnd();
        }
    }
}

impl Default for Console {
    fn default() -> Self {
        Self::new()
    }
}

const CONSOLE_THEME_COLORS: usize = 16;

unsafe fn push_console_theme() {
    use imgui::sys::ImVec4;
    let c = |r: f32, g: f32, b: f32, a: f32| ImVec4 { x: r, y: g, z: b, w: a };
    let push = |idx: imgui::sys::ImGuiCol_, v: ImVec4| {
        imgui::sys::igPushStyleColor_Vec4(idx as i32, v);
    };
    push(imgui::sys::ImGuiCol_WindowBg, c(0.07, 0.07, 0.08, 0.94));
    push(imgui::sys::ImGuiCol_ChildBg, c(0.05, 0.05, 0.06, 1.0));
    push(imgui::sys::ImGuiCol_FrameBg, c(0.13, 0.13, 0.15, 1.0));
    push(imgui::sys::ImGuiCol_FrameBgHovered, c(0.17, 0.17, 0.19, 1.0));
    push(imgui::sys::ImGuiCol_FrameBgActive, c(0.20, 0.20, 0.22, 1.0));
    push(imgui::sys::ImGuiCol_Text, c(0.90, 0.90, 0.90, 1.0));
    push(imgui::sys::ImGuiCol_TextDisabled, c(0.42, 0.42, 0.45, 1.0));
    push(imgui::sys::ImGuiCol_Border, c(0.24, 0.24, 0.26, 0.6));
    push(imgui::sys::ImGuiCol_NavHighlight, c(0.35, 0.35, 0.38, 0.45));
    push(imgui::sys::ImGuiCol_TextSelectedBg, c(0.35, 0.35, 0.38, 0.55));
    push(imgui::sys::ImGuiCol_Header, c(0.22, 0.22, 0.24, 1.0));
    push(imgui::sys::ImGuiCol_HeaderHovered, c(0.28, 0.28, 0.30, 1.0));
    push(imgui::sys::ImGuiCol_HeaderActive, c(0.32, 0.32, 0.34, 1.0));
    push(imgui::sys::ImGuiCol_Button, c(0.18, 0.18, 0.20, 1.0));
    push(imgui::sys::ImGuiCol_ButtonHovered, c(0.24, 0.24, 0.26, 1.0));
    push(imgui::sys::ImGuiCol_ButtonActive, c(0.28, 0.28, 0.30, 1.0));
}

unsafe fn pop_console_theme() {
    imgui::sys::igPopStyleColor(CONSOLE_THEME_COLORS as i32);
}

unsafe fn draw_log_line(text: &str) {
    if let Some(p) = logger::parse_plain_log_line(text) {
        let ts_col = logger::console_timestamp_imgui_color();
        let lvl_col = logger::level_imgui_color(p.level);
        let scope_col = logger::console_scope_imgui_color();
        let msg_col = logger::console_message_imgui_color();
        text_colored(&ts_col, "[");
        imgui::sys::igSameLine(0.0, 0.0);
        text_colored(&ts_col, &p.timestamp);
        imgui::sys::igSameLine(0.0, 0.0);
        text_colored(&lvl_col, &format!(" {}", p.level.as_str()));
        imgui::sys::igSameLine(0.0, 0.0);
        text_colored(&ts_col, "] ");
        imgui::sys::igSameLine(0.0, 0.0);
        text_colored(&scope_col, &format!("{}:", p.scope));
        imgui::sys::igSameLine(0.0, 0.0);
        text_colored(&msg_col, &p.message);
    } else {
        let col = logger::console_legacy_imgui_color();
        text_colored(&col, text);
    }
}

unsafe fn text_colored(color: &[f32; 4], text: &str) {
    let c = CString::new(text).unwrap_or_default();
    let col = imgui::sys::ImVec4 {
        x: color[0],
        y: color[1],
        z: color[2],
        w: color[3],
    };
    imgui::sys::igPushStyleColor_Vec4(imgui::sys::ImGuiCol_Text as i32, col);
    imgui::sys::igTextUnformatted(c.as_ptr(), std::ptr::null());
    imgui::sys::igPopStyleColor(1);
}

unsafe extern "C" fn input_callback(data: *mut imgui::sys::ImGuiInputTextCallbackData) -> i32 {
    if data.is_null() {
        return 0;
    }
    let data = &mut *data;
    let console = data.UserData as *mut Console;
    if console.is_null() {
        return 0;
    }
    let console = &mut *console;

    if data.EventFlag as u32 == imgui::sys::ImGuiInputTextFlags_CallbackHistory {
        if data.EventKey == imgui::sys::ImGuiKey_UpArrow {
            if console.history_nav > 0 {
                console.history_nav -= 1;
                copy_buf(data, console.history[console.history_nav].as_str());
            }
        } else if data.EventKey == imgui::sys::ImGuiKey_DownArrow {
            if console.history_nav + 1 < console.history.len() {
                console.history_nav += 1;
                copy_buf(data, console.history[console.history_nav].as_str());
            } else if console.history_nav < console.history.len() {
                console.history_nav = console.history.len();
                copy_buf(data, "");
            }
        }
        return 0;
    }

    if data.EventFlag as u32 == imgui::sys::ImGuiInputTextFlags_CallbackEdit {
        console.completion_pick = 0;
        return 0;
    }

    if data.EventFlag as u32 == imgui::sys::ImGuiInputTextFlags_CallbackCompletion {
        if data.EventKey == imgui::sys::ImGuiKey_Tab {
            if console.draw_cmd.is_null() {
                return 0;
            }
            let cmd = &*console.draw_cmd;
            let cands = console.autocomplete_candidates(cmd);
            if cands.is_empty() {
                return 0;
            }
            let io = imgui::sys::igGetIO();
            let shift = !io.is_null() && (*io).KeyShift;
            if shift {
                console.completion_pick = if console.completion_pick == 0 {
                    cands.len() - 1
                } else {
                    console.completion_pick - 1
                };
            } else {
                console.completion_pick = (console.completion_pick + 1) % cands.len();
            }
            console.apply_completion(&cands[console.completion_pick]);
            copy_buf(data, console.input_str());
        }
        return 0;
    }

    0
}

unsafe fn copy_buf(data: &mut imgui::sys::ImGuiInputTextCallbackData, s: &str) {
    let buf = data.Buf;
    if buf.is_null() || data.BufSize <= 0 {
        return;
    }
    let max = (data.BufSize - 1) as usize;
    let bytes = s.as_bytes();
    let n = bytes.len().min(max);
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, n);
    buf.add(n).write(0);
    data.BufTextLen = n as i32;
    data.BufDirty = true;
    data.CursorPos = n as i32;
    data.SelectionStart = data.CursorPos;
    data.SelectionEnd = data.CursorPos;
}
