use serde::{Deserialize, Serialize};

use crate::geometry::Rect;

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct WindowId(pub String);

impl WindowId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn is_valid(&self) -> bool {
        !self.0.is_empty() && self.0.len() <= 128
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct ContextKey {
    pub workspace: u32,
    pub monitor: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Left,
    Down,
    Up,
    Right,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Config {
    pub inner_gap: i32,
    pub outer_gap: i32,
    pub split_ratio: f64,
    pub resize_step: f64,
    pub smart_split: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            inner_gap: 8,
            outer_gap: 8,
            split_ratio: 0.5,
            resize_step: 0.05,
            smart_split: true,
        }
    }
}

impl Config {
    pub fn sanitized(mut self) -> Self {
        self.inner_gap = self.inner_gap.clamp(0, 512);
        self.outer_gap = self.outer_gap.clamp(0, 512);
        self.split_ratio = if self.split_ratio.is_finite() {
            self.split_ratio
                .clamp(crate::tree::MIN_RATIO, crate::tree::MAX_RATIO)
        } else {
            0.5
        };
        self.resize_step = if self.resize_step.is_finite() {
            self.resize_step.clamp(0.01, 0.25)
        } else {
            0.05
        };
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WindowSnapshot {
    pub id: WindowId,
    pub context: ContextKey,
    pub app_id: Option<String>,
    pub frame_rect: Rect,
    #[serde(default)]
    pub min_width: i32,
    #[serde(default)]
    pub min_height: i32,
    pub fullscreen: bool,
    pub window_type: String,
    pub focused: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct WorkAreaSnapshot {
    pub context: ContextKey,
    pub rect: Rect,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct Snapshot {
    pub windows: Vec<WindowSnapshot>,
    pub work_areas: Vec<WorkAreaSnapshot>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    FullSync {
        snapshot: Snapshot,
        config: Config,
    },
    Configure {
        config: Config,
    },
    AddWindow {
        window: WindowSnapshot,
        work_area: Rect,
    },
    RemoveWindow {
        window_id: WindowId,
    },
    FocusWindow {
        window_id: WindowId,
    },
    WindowContextChanged {
        window_id: WindowId,
        context: ContextKey,
        work_area: Rect,
    },
    Relayout {
        context: Option<ContextKey>,
    },
    FocusDirection {
        window_id: WindowId,
        direction: Direction,
    },
    MoveDirection {
        window_id: WindowId,
        direction: Direction,
    },
    Resize {
        window_id: WindowId,
        direction: Direction,
    },
    ToggleFloating {
        window_id: WindowId,
    },
    ToggleMaximize {
        window_id: WindowId,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Placement {
    pub window_id: WindowId,
    pub rect: Rect,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    Ack,
    Placements {
        placements: Vec<Placement>,
    },
    Focus {
        window_id: WindowId,
    },
    PlacementsAndFocus {
        placements: Vec<Placement>,
        window_id: WindowId,
    },
    Error {
        message: String,
    },
}
