use std::fs;
use std::path::PathBuf;

use iced::widget::pane_grid::{self, Node};
use serde::{Deserialize, Serialize};

use crate::Panel;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum LayoutNode {
    Split {
        axis: String,
        ratio: f32,
        a: Box<LayoutNode>,
        b: Box<LayoutNode>,
    },
    Pane {
        panel: String,
    },
}

impl LayoutNode {
    fn from_node(node: &Node, panes: &std::collections::BTreeMap<pane_grid::Pane, Panel>) -> Self {
        match node {
            Node::Split { axis, ratio, a, b, .. } => LayoutNode::Split {
                axis: match axis {
                    pane_grid::Axis::Horizontal => "h".into(),
                    pane_grid::Axis::Vertical => "v".into(),
                },
                ratio: *ratio,
                a: Box::new(Self::from_node(a, panes)),
                b: Box::new(Self::from_node(b, panes)),
            },
            Node::Pane(pane) => {
                let panel = panes.get(pane).copied().unwrap_or(Panel::Console);
                LayoutNode::Pane {
                    panel: panel_name(panel),
                }
            }
        }
    }

    pub fn to_iced(&self) -> pane_grid::Configuration<Panel> {
        match self {
            LayoutNode::Split { axis, ratio, a, b } => pane_grid::Configuration::Split {
                axis: match axis.as_str() {
                    "h" => pane_grid::Axis::Horizontal,
                    _ => pane_grid::Axis::Vertical,
                },
                ratio: *ratio,
                a: Box::new(a.to_iced()),
                b: Box::new(b.to_iced()),
            },
            LayoutNode::Pane { panel } => {
                pane_grid::Configuration::Pane(parse_panel(panel))
            }
        }
    }
}

fn panel_name(p: Panel) -> String {
    match p {
        Panel::EntityList => "entities".into(),
        Panel::Viewport => "viewport".into(),
        Panel::Inspector => "inspector".into(),
        Panel::Console => "console".into(),
    }
}

fn parse_panel(s: &str) -> Panel {
    match s {
        "entities" => Panel::EntityList,
        "viewport" => Panel::Viewport,
        "inspector" => Panel::Inspector,
        _ => Panel::Console,
    }
}

fn config_dir() -> PathBuf {
    let base = dirs_or_temp();
    base.join("deerpunk").join("editor")
}

fn layout_path() -> PathBuf {
    config_dir().join("layout.toml")
}

fn dirs_or_temp() -> PathBuf {
    if let Some(data) = std::env::var_os("APPDATA") {
        PathBuf::from(data)
    } else if let Some(data) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(data)
    } else {
        std::env::temp_dir()
    }
}

pub fn save_layout(state: &pane_grid::State<Panel>) {
    let node = state.layout();
    let layout = LayoutNode::from_node(node, &state.panes);
    let toml_str = match toml::to_string_pretty(&layout) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[editor] failed to serialize layout: {e}");
            return;
        }
    };
    let dir = config_dir();
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("[editor] failed to create config dir {}: {e}", dir.display());
        return;
    }
    let path = layout_path();
    if let Err(e) = fs::write(&path, &toml_str) {
        eprintln!("[editor] failed to save layout to {}: {e}", path.display());
    }
}

pub fn load_layout() -> Option<pane_grid::Configuration<Panel>> {
    let path = layout_path();
    let data = fs::read_to_string(&path).ok()?;
    let node: LayoutNode = toml::from_str(&data).ok()?;
    Some(node.to_iced())
}
