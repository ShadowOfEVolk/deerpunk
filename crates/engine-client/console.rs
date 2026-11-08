//! In-game Nuklear console (GoldSrc/Source style). Cvar: `cl_console_show`.

deerpunk_engine_common::cvar_def!(
    "cl_console_show",
    "0",
    "Show in-game Nuklear console (1=visible). Toggle with ~."
);

use std::ffi::{c_void, CStr, CString};

use deerpunk_engine_common::input::{Input, KEY_GRAVE};
use deerpunk_engine_common::logger;

use deerpunk_engine_common::console_cmd::ConsoleCmdSink;
use deerpunk_engine_common::cvar::CvarSystem;

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
    /// Tab candidates for the Nuklear input callback (filled each `draw`).
    tab_cycle: Vec<String>,
    prev_up: bool,
    prev_down: bool,
    prev_tab: bool,
    edge_up: bool,
    edge_down: bool,
    edge_tab: bool,
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
            tab_cycle: Vec::new(),
            prev_up: false,
            prev_down: false,
            prev_tab: false,
            edge_up: false,
            edge_down: false,
            edge_tab: false,
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

    /// Nuklear widgets — must run between `host_ui_begin_frame` and `host_ui_end_frame`.
    pub fn tick_draw(
        &mut self,
        cvar: &CvarSystem,
        cmd: &impl ConsoleCmdSink,
        cvar_sys: &CvarSystem,
    ) {
        if !Self::visible(cvar) {
            return;
        }
        self.refresh_completion_names(cmd, cvar_sys);
        self.draw(cmd, cvar_sys);
    }

    pub fn tick(
        &mut self,
        cvar: &CvarSystem,
        cmd: &impl ConsoleCmdSink,
        cvar_sys: &CvarSystem,
        input: &mut Input,
    ) {
        self.tick_input(cvar, input);
        self.tick_draw(cvar, cmd, cvar_sys);
    }

    fn refresh_completion_names(&mut self, cmd: &impl ConsoleCmdSink, cvar_sys: &CvarSystem) {
        self.completion_names.clear();
        self.completion_names.extend(cmd.cmd_names());
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

    fn submit(&mut self, cmd: &impl ConsoleCmdSink) {
        let line = self.input_str().trim().to_string();
        if line.is_empty() {
            return;
        }
        log::info!(target: "console", "{}", line);
        self.push_history(&line);
        cmd.cmd_add(line);
        cmd.cmd_process();
        self.set_input_str("");
        self.history_nav = self.history.len();
        self.completion_pick = 0;
        self.log_follow_tail = true;
    }

    fn autocomplete_candidates(&self, cmd: &impl ConsoleCmdSink) -> Vec<String> {
        let line = self.input_str();
        if line.trim().is_empty() {
            return Vec::new();
        }
        cmd.cmd_autocomplete(line, &self.completion_names)
    }

    fn apply_completion(&mut self, candidate: &str) {
        self.set_input_str(candidate);
    }

    pub fn capture_keys(&mut self, input: &deerpunk_engine_common::input::Input) {
        let up = input.is_down(82);
        let down = input.is_down(81);
        let tab = input.is_down(43);
        self.edge_up = up && !self.prev_up;
        self.edge_down = down && !self.prev_down;
        self.edge_tab = tab && !self.prev_tab;
        self.prev_up = up;
        self.prev_down = down;
        self.prev_tab = tab;
    }

    fn draw(&mut self, cmd: &impl ConsoleCmdSink, _cvar_sys: &CvarSystem) {
        let suggestions = self.autocomplete_candidates(cmd);
        if self.edge_tab && !suggestions.is_empty() {
            let n = suggestions.len();
            if self.completion_pick >= n {
                self.completion_pick = 0;
            }
            self.completion_pick = (self.completion_pick + 1) % n;
            let pick = suggestions[self.completion_pick].clone();
            self.apply_completion(&pick);
        } else if self.edge_up {
            if self.history_nav > 0 {
                self.history_nav -= 1;
                let line = self.history[self.history_nav].clone();
                self.set_input_str(&line);
            }
        } else if self.edge_down {
            if self.history_nav + 1 < self.history.len() {
                self.history_nav += 1;
                let line = self.history[self.history_nav].clone();
                self.set_input_str(&line);
            } else {
                self.history_nav = self.history.len();
                self.set_input_str("");
            }
        }
        self.edge_up = false;
        self.edge_down = false;
        self.edge_tab = false;

        let [sw, sh] = deerpunk_engine_renderer::host_ui::screen();
        if !deerpunk_engine_renderer::host_ui::begin_console(sw, sh * 0.5) {
            return;
        }
        let log_h = (sh * 0.5 - 28.0 - suggestions.len().min(6) as f32 * 18.0).max(40.0);
        if deerpunk_engine_renderer::host_ui::group_begin("console-log", log_h) {
            logger::for_each_line(|bytes| {
                let n = bytes.len().min(logger::LOG_LINE_CAP);
                self.line_z[..n].copy_from_slice(&bytes[..n]);
                self.line_z[n] = 0;
                let text = String::from_utf8_lossy(&self.line_z[..n]).to_string();
                let col = if let Some(p) = logger::parse_plain_log_line(&text) {
                    logger::level_ui_color(p.level)
                } else {
                    logger::console_legacy_ui_color()
                };
                deerpunk_engine_renderer::host_ui::label_colored(&text, col);
            });
            deerpunk_engine_renderer::host_ui::group_end("console-log", self.log_follow_tail);
        }
        for (i, s) in suggestions.iter().enumerate().take(6) {
            if deerpunk_engine_renderer::host_ui::selectable(s, i == self.completion_pick) {
                self.apply_completion(s);
                self.completion_pick = i;
            }
        }
        if self.focus_input_next {
            deerpunk_engine_renderer::host_ui::edit_focus();
            self.focus_input_next = false;
        }
        deerpunk_engine_renderer::host_ui::edit_focus();
        if deerpunk_engine_renderer::host_ui::edit_line(&mut self.input_buf) {
            self.submit(cmd);
        }
        deerpunk_engine_renderer::host_ui::end_window();
    }
}

impl Default for Console {
    fn default() -> Self {
        Self::new()
    }
}

