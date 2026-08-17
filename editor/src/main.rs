mod game_process;
mod project;
mod shared_viewport;

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use iced::widget::{
    button, column, container, row, rule, scrollable, space, text, text_input,
};
use iced::{Element, Font, Length, Subscription, Task};
use iced::widget::image::Handle as ImageHandle;

use deerpunk_engine_common::frame_buffer::SharedFrameBuffer;
use deerpunk_engine_common::ipc::IpcMessage;

use parking_lot::Mutex;

use iced::Event;
use iced::keyboard::key::Named;

use game_process::{BuildResult, GameProcess, LogLine, LogSource};
use project::ProjectConfig;

const SHM_NAME: &str = "deerpunk_editor_0";

fn iced_key_to_vk(key: iced::keyboard::Key) -> Option<u16> {
    match key {
        iced::keyboard::Key::Character(c) => {
            let s = c.as_str();
            match s {
                "w" | "W" => Some(0x57),
                "a" | "A" => Some(0x41),
                "s" | "S" => Some(0x53),
                "d" | "D" => Some(0x44),
                "e" | "E" => Some(0x45),
                "q" | "Q" => Some(0x51),
                "r" | "R" => Some(0x52),
                "f" | "F" => Some(0x46),
                "g" | "G" => Some(0x47),
                "c" | "C" => Some(0x43),
                "z" | "Z" => Some(0x5A),
                "x" | "X" => Some(0x58),
                "v" | "V" => Some(0x56),
                "b" | "B" => Some(0x42),
                "1" => Some(0x31),
                "2" => Some(0x32),
                "3" => Some(0x33),
                "4" => Some(0x34),
                "5" => Some(0x35),
                "6" => Some(0x36),
                "7" => Some(0x37),
                "8" => Some(0x38),
                "9" => Some(0x39),
                "0" => Some(0x30),
                _ => None,
            }
        }
        iced::keyboard::Key::Named(n) => match n {
            Named::Space => Some(0x20),
            Named::Shift => Some(0xA0),
            Named::Control => Some(0xA2),
            Named::Alt => Some(0xA4),
            Named::Enter => Some(0x0D),
            Named::Escape => Some(0x1B),
            Named::Tab => Some(0x09),
            Named::Backspace => Some(0x08),
            Named::Delete => Some(0x2E),
            Named::ArrowUp => Some(0x26),
            Named::ArrowDown => Some(0x28),
            Named::ArrowLeft => Some(0x25),
            Named::ArrowRight => Some(0x27),
            Named::Home => Some(0x24),
            Named::End => Some(0x23),
            Named::PageUp => Some(0x21),
            Named::PageDown => Some(0x22),
            Named::F1 => Some(0x70),
            Named::F2 => Some(0x71),
            Named::F3 => Some(0x72),
            Named::F4 => Some(0x73),
            Named::F5 => Some(0x74),
            Named::F6 => Some(0x75),
            Named::F7 => Some(0x76),
            Named::F8 => Some(0x77),
            Named::F9 => Some(0x78),
            Named::F10 => Some(0x79),
            Named::F11 => Some(0x7A),
            Named::F12 => Some(0x7B),
            _ => None,
        },
        _ => None,
    }
}

fn main() -> iced::Result {
    iced::application(Editor::new, Editor::update, Editor::view)
        .title("Deerpunk Editor")
        .subscription(Editor::subscription)
        .run()
}

#[derive(Debug, Clone)]
enum Message {
    ConsoleInputChanged(String),
    ConsoleSubmit(String),
    SpawnGame,
    GameKilled,
    Tick,
    IcedEvent(Event),
    CopyLog,
    EntitySelected(usize),
    NoOp,
}

enum State {
    Idle,
    Building {
        build_rx: mpsc::Receiver<LogLine>,
        result_rx: mpsc::Receiver<BuildResult>,
    },
    Running,
}

struct ConsoleEntry {
    source: LogSource,
    text: String,
}

struct Editor {
    project: Option<ProjectConfig>,
    game: Option<GameProcess>,
    frame_buffer: Option<Mutex<SharedFrameBuffer>>,
    state: State,

    console_entries: Vec<ConsoleEntry>,
    console_input: String,
    console_scroll: iced::widget::Id,
    console_log_text: String,

    entity_list_selected: usize,
    entities: Vec<String>,

    inspector_text: String,

    viewport_pixels: Vec<u8>,
    viewport_image: Option<iced::widget::image::Handle>,
    viewport_width: u32,
    viewport_height: u32,

    shared_texture_handle: Option<(i64, u32, u32)>,

    // Viewport input state
    console_input_focused: bool,
    key_bitmap: [u8; 32],
    mouse_dx: f32,
    mouse_dy: f32,
    last_cursor_pos: Option<iced::Point>,
    ipc_connected: bool,

    status: String,
}

impl Editor {
    fn new() -> (Self, Task<Message>) {
        let project_dir = std::env::args()
            .nth(1)
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        let project = ProjectConfig::load(&project_dir).ok();

        let mut editor = Self {
            project,
            game: None,
            frame_buffer: None,
            state: State::Idle,
            console_entries: Vec::new(),
            console_input: String::new(),
            console_scroll: iced::widget::Id::unique(),
            console_log_text: String::new(),
            entity_list_selected: 0,
            entities: Vec::new(),
            inspector_text: "Select an entity...".into(),
            viewport_pixels: Vec::new(),
            viewport_image: None,
            viewport_width: 0,
            viewport_height: 0,
            shared_texture_handle: None,
            console_input_focused: false,
            key_bitmap: [0u8; 32],
            mouse_dx: 0.0,
            mouse_dy: 0.0,
            last_cursor_pos: None,
            ipc_connected: false,
            status: String::new(),
        };

        if let Some(ref p) = editor.project {
            editor.status = format!("Project: {}", p.name);
            editor.push_console_line(
                LogSource::Editor,
                &format!("Loaded project: {} ({})", p.name, p.root.display()),
            );
        } else {
            editor.status = "No project found. Pass project path as argument.".into();
            editor.push_console_line(
                LogSource::Editor,
                "No deerpunk.toml found. Usage: deerpunk-editor [project_path]",
            );
        }

        (editor, Task::none())
    }

fn push_console_line(&mut self, source: LogSource, text: &str) {
    let prefix = match source {
        LogSource::Stdout => "[game]",
        LogSource::Stderr => "[err] ",
        LogSource::Editor => "[ed]  ",
    };
    self.console_log_text
        .push_str(&format!("{prefix} {text}\n"));
    self.console_entries.push(ConsoleEntry {
        source,
        text: text.to_string(),
    });
    const MAX_LINES: usize = 5000;
    if self.console_entries.len() > MAX_LINES {
        let excess = self.console_entries.len() - MAX_LINES;
        self.console_entries.drain(..excess);
        if let Some(pos) = self.console_log_text.match_indices('\n').nth(excess) {
            self.console_log_text.drain(..=pos.0);
        }
    }
}

    fn spawn_game_with_binary(&mut self, _bin_path: PathBuf) {
        let project_name = self.project.as_ref().map(|p| p.name.clone());
        let gamedir = self.project.as_ref().map(|p| p.gamedir.clone());
        let commands = self
            .project
            .as_ref()
            .map(|p| p.commands.clone())
            .unwrap_or_default();
        let project_root = self.project.as_ref().map(|p| p.root.clone());

        if let (Some(name), Some(root)) = (&project_name, &project_root) {
            match SharedFrameBuffer::create(SHM_NAME) {
                Ok(fb) => {
                    self.frame_buffer = Some(Mutex::new(fb));
                    self.push_console_line(
                        LogSource::Editor,
                        &format!("Created shared memory: {SHM_NAME}"),
                    );
                }
                Err(e) => {
                    self.push_console_line(
                        LogSource::Editor,
                        &format!("Failed to create shared memory: {e}"),
                    );
                    return;
                }
            }

            let project = ProjectConfig {
                root: root.clone(),
                name: name.clone(),
                engine: self.project.as_ref().unwrap().engine.clone(),
                gamedir: gamedir.unwrap_or_default(),
                commands,
            };

            match GameProcess::spawn(&project, SHM_NAME) {
                Ok(game) => {
                    self.push_console_line(
                        LogSource::Editor,
                        &format!("Game started: {}", _bin_path.display()),
                    );
                    self.status = format!("Running: {name}");
                    self.game = Some(game);
                    self.state = State::Running;
                }
                Err(e) => {
                    self.push_console_line(
                        LogSource::Editor,
                        &format!("Failed to spawn game: {e}"),
                    );
                    self.state = State::Idle;
                }
            }
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SpawnGame => {
                if let Some(ref project) = self.project {
                    let name = project.name.clone();
                    let project_clone = project.clone();
                    self.status = format!("Building: {name}...");
                    self.console_entries.clear();
                    self.console_log_text.clear();
                    self.entities.clear();
                    self.ipc_connected = false;

                    let (build_tx, build_rx) = mpsc::channel();
                    let (result_tx, result_rx) = mpsc::channel();

                    std::thread::spawn(move || {
                        let result = GameProcess::build(&project_clone, build_tx.clone());
                        let _ = result_tx.send(result);
                        drop(build_tx);
                    });

                    self.state = State::Building {
                        build_rx,
                        result_rx,
                    };
                } else {
                    self.push_console_line(LogSource::Editor, "No project loaded.");
                }
                Task::none()
            }

            Message::GameKilled => {
                if let Some(ref mut game) = self.game {
                    game.kill();
                }
                self.game = None;
                self.frame_buffer = None;
                self.state = State::Idle;
                self.push_console_line(LogSource::Editor, "Game process killed.");
                self.status = "Game stopped.".into();
                Task::none()
            }

            Message::ConsoleInputChanged(s) => {
                self.console_input = s;
                Task::none()
            }

            Message::ConsoleSubmit(cmd) => {
                let trimmed = cmd.trim().to_string();
                if !trimmed.is_empty() {
                    if let Some(ref game) = self.game {
                        game.send_command(&trimmed);
                        self.push_console_line(LogSource::Editor, &format!("> {trimmed}"));
                    } else {
                        self.push_console_line(
                            LogSource::Editor,
                            "No game running. Start game first.",
                        );
                    }
                }
                self.console_input.clear();
                Task::none()
            }

            Message::CopyLog => {
                let text = self.console_log_text.clone();
                return iced::clipboard::write(text);
            }

            Message::EntitySelected(i) => {
                self.entity_list_selected = i;
                return Task::none();
            }

            Message::Tick => {
                let mut new_lines: Vec<LogLine> = Vec::new();
                let mut build_done: Option<BuildResult> = None;
                let mut game_alive = true;

                match &self.state {
                    State::Building {
                        build_rx,
                        result_rx,
                    } => {
                        while let Ok(line) = build_rx.try_recv() {
                            new_lines.push(line);
                        }
                        if let Ok(result) = result_rx.try_recv() {
                            build_done = Some(result);
                        }
                    }
                    State::Running => {
                        let mut ipc_messages = Vec::new();
                        if let Some(ref mut game) = self.game {
                            if !game.is_running() {
                                game_alive = false;
                            }
                            while let Some(line) = game.try_recv_log() {
                                new_lines.push(line);
                            }
                            while let Some(msg) = game.try_recv_ipc() {
                                ipc_messages.push(msg);
                            }
                        }
                        for msg in ipc_messages {
                            match msg {
                                IpcMessage::EntityList { entities } => {
                                    self.entities = entities.iter().map(|e| {
                                        if e.name.is_empty() {
                                            format!("#{}", e.id)
                                        } else {
                                            format!("#{} {} {}", e.id, e.name, e.kind)
                                        }
                                    }).collect();
                                }
                                IpcMessage::Pong => {}
                                IpcMessage::SharedTextureHandle { handle, width, height } => {
                                    self.shared_texture_handle = Some((handle, width, height));
                                    self.viewport_width = width;
                                    self.viewport_height = height;
                                    self.push_console_line(LogSource::Editor, &format!("Shared texture: handle={handle} {width}x{height}"));
                                }
                                _ => {}
                            }
                        }
                    }
                    State::Idle => {}
                }

                for line in &new_lines {
                    if line.text.contains("IPC connected") {
                        self.ipc_connected = true;
                    }
                    if line.text.contains("editor disconnected") || line.text.contains("broken pipe") {
                        self.ipc_connected = false;
                    }
                    self.push_console_line(line.source.clone(), &line.text);
                }

                if !game_alive {
                    self.push_console_line(LogSource::Editor, "Game process exited.");
                    self.game = None;
                    self.frame_buffer = None;
                    self.state = State::Idle;
                    self.status = "Game exited.".into();
                } else if let Some(result) = build_done {
                    match result {
                        BuildResult::Success(bin_path) => {
                            self.console_entries.clear();
                            self.console_log_text.clear();
                            self.push_console_line(
                                LogSource::Editor,
                                "Build complete, launching game...",
                            );
                            self.spawn_game_with_binary(bin_path);
                        }
                        BuildResult::Failed(e) => {
                            self.push_console_line(
                                LogSource::Editor,
                                &format!("Build failed: {e}"),
                            );
                            self.status = "Build failed.".into();
                            self.state = State::Idle;
                        }
                    }
                }

                if let Some(ref fb) = self.frame_buffer {
                    let mut fb = fb.lock();
                    if fb.dirty() {
                        self.viewport_pixels = fb.read_frame();
                        self.viewport_width = fb.width();
                        self.viewport_height = fb.height();
                        self.viewport_image = Some(ImageHandle::from_rgba(
                            self.viewport_width,
                            self.viewport_height,
                            self.viewport_pixels.clone(),
                        ));
                        fb.clear_dirty();
                    }
                }

                // Send input to game via IPC — ALWAYS, even when idle
                // (game needs to know keys were released)
                if !self.console_input_focused {
                    let dx = self.mouse_dx;
                    let dy = self.mouse_dy;
                    self.mouse_dx = 0.0;
                    self.mouse_dy = 0.0;

                    if let Some(ref game) = self.game {
                        game.send_ipc(&IpcMessage::InputState {
                            mouse_dx: dx,
                            mouse_dy: dy,
                            mouse_buttons: 0,
                            keys: self.key_bitmap,
                        });
                    }
                }

                Task::none()
            }

            Message::NoOp => Task::none(),

            Message::IcedEvent(event) => {
                if !matches!(self.state, State::Running) {
                    return Task::none();
                }
                if self.console_input_focused {
                    return Task::none();
                }
                match event {
                    Event::Keyboard(iced::keyboard::Event::KeyPressed {
                        key, ..
                    }) => {
                        if let Some(vk) = iced_key_to_vk(key) {
                            self.key_bitmap[(vk / 8) as usize] |= 1 << (vk % 8);
                        }
                    }
                    Event::Keyboard(iced::keyboard::Event::KeyReleased {
                        key, ..
                    }) => {
                        if let Some(vk) = iced_key_to_vk(key) {
                            self.key_bitmap[(vk / 8) as usize] &= !(1 << (vk % 8));
                        }
                    }
                    Event::Mouse(iced::mouse::Event::CursorMoved { position }) => {
                        if let Some(prev) = self.last_cursor_pos {
                            self.mouse_dx += position.x - prev.x;
                            self.mouse_dy += position.y - prev.y;
                        }
                        self.last_cursor_pos = Some(position);
                    }
                    _ => {}
                }
                Task::none()
            }
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let tick_sub = match &self.state {
            State::Building { .. } | State::Running => {
                iced::time::every(Duration::from_millis(16)).map(|_| Message::Tick)
            }
            State::Idle => Subscription::none(),
        };

        let event_sub = iced::event::listen().map(Message::IcedEvent);

        Subscription::batch([tick_sub, event_sub])
    }

    fn view(&self) -> Element<'_, Message> {
        let is_running = matches!(self.state, State::Running);
        let is_building = matches!(self.state, State::Building { .. });

        let ipc_status = if is_running {
            if self.ipc_connected {
                text("IPC: connected").size(12).font(Font::MONOSPACE)
            } else {
                text("IPC: waiting...").size(12).font(Font::MONOSPACE)
            }
        } else {
            text("IPC: off").size(12).font(Font::MONOSPACE)
        };

        let toolbar = row![
            button(text("Start Game").size(12)).on_press_maybe(if is_running || is_building {
                None
            } else {
                Some(Message::SpawnGame)
            }),
            button(text("Stop Game").size(12)).on_press_maybe(if is_running {
                Some(Message::GameKilled)
            } else {
                None
            }),
            text(&self.status).size(12),
            space::horizontal(),
            ipc_status,
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

        let top_row = row![
            self.view_entity_list(),
            rule::vertical(1),
            self.view_viewport(),
            rule::vertical(1),
            self.view_inspector(),
        ]
        .height(Length::Fill);

        let bottom = self.view_console();

        column![
            toolbar,
            rule::horizontal(1),
            top_row,
            rule::horizontal(1),
            bottom
        ]
        .height(Length::Fill)
        .into()
    }

    fn view_entity_list(&self) -> Element<'_, Message> {
        let title = text("Entities").size(14).width(Length::Fill);

        let content = if self.entities.is_empty() {
            let content: Element<'_, Message> = match &self.state {
                State::Running => container(text("Waiting for entity list...").size(11)).padding(8).into(),
                _ => container(text("Game not running").size(11)).padding(8).into(),
            };
            content
        } else {
            let count = text(format!("{} entities", self.entities.len())).size(11);
            let mut list = column![].spacing(1);
            for (i, name) in self.entities.iter().enumerate() {
                let is_selected = i == self.entity_list_selected;
                let item = button(text(name).size(11).font(iced::Font::MONOSPACE))
                    .on_press(Message::EntitySelected(i))
                    .width(Length::Fill)
                    .padding(iced::Padding::from([2, 4]));

                let item = if is_selected {
                    item.style(iced::widget::button::primary)
                } else {
                    item.style(iced::widget::button::text)
                };

                list = list.push(item);
            }
            column![count, scrollable(list).height(Length::Fill).anchor_bottom()]
                .spacing(4)
                .into()
        };

        container(column![title, content].spacing(4).padding(8))
            .width(Length::Fixed(200.0))
            .height(Length::Fill)
            .padding(4)
            .into()
    }

    fn view_viewport(&self) -> Element<'_, Message> {
        if let Some((handle, width, height)) = self.shared_texture_handle {
            let program = shared_viewport::SharedViewportProgram { handle, width, height };
            iced::widget::shader::Shader::new(program)
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        } else if let Some(ref handle) = self.viewport_image {
            container(
                iced::widget::image(handle.clone())
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .content_fit(iced::ContentFit::Contain),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(|_theme: &iced::Theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgb(0.05, 0.05, 0.05))),
                ..Default::default()
            })
            .into()
        } else {
            let info = text("Waiting for frame...").size(11);
            container(info)
                .width(Length::Fill)
                .height(Length::Fill)
                .center_x(Length::Fill)
                .center_y(Length::Fill)
                .style(|_theme: &iced::Theme| iced::widget::container::Style {
                    background: Some(iced::Background::Color(iced::Color::from_rgb(
                        0.1, 0.1, 0.12,
                    ))),
                    text_color: Some(iced::Color::from_rgb(0.5, 0.5, 0.5)),
                    ..Default::default()
                })
                .into()
        }
    }

    fn view_inspector(&self) -> Element<'_, Message> {
        let title = text("Inspector").size(14).width(Length::Fill);
        let content = text(&self.inspector_text).size(12);

        container(column![title, content].spacing(4).padding(8))
            .width(Length::Fixed(220.0))
            .height(Length::Fill)
            .padding(4)
            .into()
    }

    fn view_console(&self) -> Element<'_, Message> {
        let title = text("Console").size(14).width(Length::Fill);

        let mut log_lines = column![].spacing(1);
        for entry in &self.console_entries {
            let (color, prefix) = match entry.source {
                LogSource::Stdout => {
                    (iced::Color::from_rgb(0.4, 0.8, 1.0), "[game]")
                }
                LogSource::Stderr => {
                    (iced::Color::from_rgb(1.0, 0.35, 0.35), "[err] ")
                }
                LogSource::Editor => {
                    (iced::Color::from_rgb(0.4, 1.0, 0.5), "[ed]  ")
                }
            };
            let line_text = format!("{prefix} {}", entry.text);
            log_lines = log_lines.push(
                text(line_text)
                    .font(Font::MONOSPACE)
                    .size(12)
                    .color(color),
            );
        }

        let log_display = scrollable(log_lines)
            .id(self.console_scroll.clone())
            .height(Length::Fill)
            .width(Length::Fill);

        let input = text_input("Type command...", &self.console_input)
            .on_input(Message::ConsoleInputChanged)
            .on_submit(Message::ConsoleSubmit(self.console_input.clone()))
            .size(12)
            .width(Length::Fill);

        let header = row![
            title,
            button(text("Copy Log").size(11)).on_press(Message::CopyLog),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

        container(column![header, log_display, input].spacing(4).padding(8))
            .height(Length::Fixed(250.0))
            .padding(4)
            .into()
    }
}
