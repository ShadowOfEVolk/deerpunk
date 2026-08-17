mod game_process;
mod project;

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use iced::widget::{
    button, column, container, horizontal_rule, row, scrollable, text, text_input,
    vertical_rule,
};
use iced::{Element, Font, Length, Subscription, Task};
use iced::widget::image::Handle as ImageHandle;

use deerpunk_engine_common::frame_buffer::SharedFrameBuffer;

use parking_lot::Mutex;

use game_process::{BuildResult, GameProcess, LogLine, LogSource};
use project::ProjectConfig;

const SHM_NAME: &str = "deerpunk_editor_0";

fn main() -> iced::Result {
    iced::application("Deerpunk Editor", Editor::update, Editor::view)
        .subscription(Editor::subscription)
        .run_with(Editor::new)
}

#[derive(Debug, Clone)]
enum Message {
    ConsoleInputChanged(String),
    ConsoleSubmit(String),
    SpawnGame,
    GameKilled,
    Tick,
    CopyLog,
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
    console_scroll: scrollable::Id,
    console_log_text: String,

    entity_list_selected: usize,
    entities: Vec<String>,

    inspector_text: String,

    viewport_pixels: Vec<u8>,
    viewport_width: u32,
    viewport_height: u32,

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
            console_scroll: scrollable::Id::unique(),
            console_log_text: String::new(),
            entity_list_selected: 0,
            entities: vec![
                "Player".into(),
                "Enemy_A".into(),
                "Enemy_B".into(),
                "Light_Directional".into(),
                "Camera_Main".into(),
                "Trigger_Zone".into(),
            ],
            inspector_text: "Select an entity...".into(),
            viewport_pixels: Vec::new(),
            viewport_width: 0,
            viewport_height: 0,
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
                        if let Some(ref mut game) = self.game {
                            if !game.is_running() {
                                game_alive = false;
                            }
                            while let Some(line) = game.try_recv_log() {
                                new_lines.push(line);
                            }
                        }
                    }
                    State::Idle => {}
                }

                for line in &new_lines {
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
                        let pixels = fb.read_frame();
                        self.viewport_width = fb.width();
                        self.viewport_height = fb.height();
                        self.viewport_pixels = pixels;
                    }
                }

                Task::none()
            }

            Message::NoOp => Task::none(),
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        match &self.state {
            State::Building { .. } | State::Running => {
                iced::time::every(Duration::from_millis(50)).map(|_| Message::Tick)
            }
            State::Idle => Subscription::none(),
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let is_running = matches!(self.state, State::Running);
        let is_building = matches!(self.state, State::Building { .. });

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
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center);

        let top_row = row![
            self.view_entity_list(),
            vertical_rule(1),
            self.view_viewport(),
            vertical_rule(1),
            self.view_inspector(),
        ]
        .height(Length::Fill);

        let bottom = self.view_console();

        column![
            toolbar,
            horizontal_rule(1),
            top_row,
            horizontal_rule(1),
            bottom
        ]
        .height(Length::Fill)
        .into()
    }

    fn view_entity_list(&self) -> Element<'_, Message> {
        let title = text("Entities").size(14).width(Length::Fill);

        let mut list = column![].spacing(2);
        for (i, name) in self.entities.iter().enumerate() {
            let is_selected = i == self.entity_list_selected;
            let item = button(text(name).size(12))
                .on_press(Message::NoOp)
                .width(Length::Fill);

            let item = if is_selected {
                item.style(iced::widget::button::primary)
            } else {
                item.style(iced::widget::button::secondary)
            };

            list = list.push(item);
        }

        container(column![title, list].spacing(4).padding(8))
            .width(Length::Fixed(180.0))
            .height(Length::Fill)
            .padding(4)
            .into()
    }

    fn view_viewport(&self) -> Element<'_, Message> {
        if !self.viewport_pixels.is_empty() && self.viewport_width > 0 && self.viewport_height > 0 {
            let handle = ImageHandle::from_rgba(
                self.viewport_width,
                self.viewport_height,
                self.viewport_pixels.clone(),
            );
            container(
                image(handle)
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
