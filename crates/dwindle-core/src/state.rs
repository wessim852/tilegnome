use std::collections::{HashMap, HashSet};

use thiserror::Error;
use tracing::{debug, info};

use crate::{
    navigation::best_candidate,
    protocol::{
        Command, Config, ContextKey, CycleDirection, Direction, Placement, Response, Snapshot,
        WindowId, WindowSnapshot,
    },
    tree::{LayoutTree, TreeError},
};

#[derive(Clone, Debug)]
struct WindowRecord {
    context: ContextKey,
    floating: bool,
}

#[derive(Clone, Debug)]
struct TilingContext {
    work_area: crate::geometry::Rect,
    tree: LayoutTree,
}

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("invalid window id")]
    InvalidWindowId,
    #[error("invalid work area for workspace {0} monitor {1}")]
    InvalidWorkArea(u32, u32),
    #[error("missing work area for workspace {0} monitor {1}")]
    MissingContext(u32, u32),
    #[error("unknown window: {0:?}")]
    UnknownWindow(WindowId),
    #[error("duplicate window: {0:?}")]
    DuplicateWindow(WindowId),
    #[error(transparent)]
    Tree(#[from] TreeError),
    #[error("state invariant failed: {0}")]
    Invariant(String),
}

#[derive(Clone, Debug, Default)]
pub struct EngineState {
    config: Config,
    contexts: HashMap<ContextKey, TilingContext>,
    windows: HashMap<WindowId, WindowRecord>,
    focused: Option<WindowId>,
}

impl EngineState {
    pub fn new(config: Config) -> Self {
        Self {
            config: config.sanitized(),
            ..Self::default()
        }
    }

    pub fn handle(&mut self, command: Command) -> Response {
        match self.apply(command) {
            Ok(response) => response,
            Err(error) => {
                tracing::error!(event = "ERROR", error = %error);
                Response::Error {
                    message: error.to_string(),
                }
            }
        }
    }

    pub fn apply(&mut self, command: Command) -> Result<Response, EngineError> {
        let response = match command {
            Command::FullSync { snapshot, config } => {
                self.full_sync(snapshot, config)?;
                Response::Placements {
                    placements: self.placements(),
                }
            }
            Command::Configure { config } => {
                self.config = config.sanitized();
                Response::Placements {
                    placements: self.placements(),
                }
            }
            Command::AddWindow { window, work_area } => {
                self.add_window(window, work_area)?;
                Response::Placements {
                    placements: self.placements(),
                }
            }
            Command::RemoveWindow { window_id } => {
                self.remove_window(&window_id)?;
                Response::Placements {
                    placements: self.placements(),
                }
            }
            Command::FocusWindow { window_id } => {
                self.focus_window(&window_id)?;
                Response::Ack
            }
            Command::WindowContextChanged {
                window_id,
                context,
                work_area,
            } => {
                self.move_context(&window_id, context, work_area)?;
                Response::Placements {
                    placements: self.placements(),
                }
            }
            Command::Relayout { context } => Response::Placements {
                placements: match context {
                    Some(context) => self.placements_for(context),
                    None => self.placements(),
                },
            },
            Command::FocusDirection {
                window_id,
                direction,
            } => match self.directional_neighbor(&window_id, direction)? {
                Some(window_id) => {
                    debug!(event = "FOCUS", window_id = %window_id.0);
                    Response::Focus { window_id }
                }
                None => Response::Ack,
            },
            Command::MoveDirection {
                window_id,
                direction,
            } => {
                self.swap_direction(&window_id, direction)?;
                Response::Placements {
                    placements: self.placements(),
                }
            }
            Command::Resize {
                window_id,
                direction,
            } => {
                self.resize(&window_id, direction)?;
                Response::Placements {
                    placements: self.placements(),
                }
            }
            Command::ToggleFloating { window_id } => {
                self.toggle_floating(&window_id)?;
                Response::Placements {
                    placements: self.placements(),
                }
            }
            Command::ToggleGroup { window_id } => {
                self.toggle_group(&window_id)?;
                Response::Placements {
                    placements: self.placements(),
                }
            }
            Command::CycleGroup { window_id, cycle } => {
                match self.cycle_group(&window_id, cycle)? {
                    Some(window_id) => Response::Focus { window_id },
                    None => Response::Ack,
                }
            }
        };
        self.debug_validate()?;
        Ok(response)
    }

    pub fn full_sync(&mut self, snapshot: Snapshot, config: Config) -> Result<(), EngineError> {
        let mut contexts = HashMap::new();
        for area in snapshot.work_areas {
            if !area.rect.is_valid() {
                return Err(EngineError::InvalidWorkArea(
                    area.context.workspace,
                    area.context.monitor,
                ));
            }
            contexts.insert(
                area.context,
                TilingContext {
                    work_area: area.rect,
                    tree: LayoutTree::new(),
                },
            );
        }

        self.config = config.sanitized();
        self.contexts = contexts;
        self.windows.clear();
        self.focused = snapshot
            .windows
            .iter()
            .find(|window| window.focused)
            .map(|window| window.id.clone());

        let mut windows = snapshot.windows;
        windows.sort_by(|a, b| a.id.cmp(&b.id));
        for window in windows {
            self.add_snapshot(window)?;
        }
        if let Some(focused) = self.focused.clone() {
            self.focus_window(&focused)?;
        }
        info!(
            event = "FULL_SYNC",
            windows = self.windows.len(),
            contexts = self.contexts.len()
        );
        self.debug_validate()?;
        Ok(())
    }

    fn add_snapshot(&mut self, window: WindowSnapshot) -> Result<(), EngineError> {
        if !window.id.is_valid() {
            return Err(EngineError::InvalidWindowId);
        }
        if self.windows.contains_key(&window.id) {
            return Err(EngineError::DuplicateWindow(window.id));
        }
        if !self.contexts.contains_key(&window.context) {
            return Err(EngineError::MissingContext(
                window.context.workspace,
                window.context.monitor,
            ));
        }

        let floating = window.fullscreen || window.window_type != "normal";
        self.windows.insert(
            window.id.clone(),
            WindowRecord {
                context: window.context,
                floating,
            },
        );
        if !floating {
            self.insert_tiled(&window.id, window.context)?;
        }
        Ok(())
    }

    fn add_window(
        &mut self,
        window: WindowSnapshot,
        work_area: crate::geometry::Rect,
    ) -> Result<(), EngineError> {
        if !work_area.is_valid() {
            return Err(EngineError::InvalidWorkArea(
                window.context.workspace,
                window.context.monitor,
            ));
        }
        self.contexts
            .entry(window.context)
            .and_modify(|context| context.work_area = work_area)
            .or_insert_with(|| TilingContext {
                work_area,
                tree: LayoutTree::new(),
            });
        self.add_snapshot(window.clone())?;
        debug!(event = "WINDOW_ADD", window_id = %window.id.0);
        Ok(())
    }

    fn remove_window(&mut self, window: &WindowId) -> Result<(), EngineError> {
        let record = self
            .windows
            .remove(window)
            .ok_or_else(|| EngineError::UnknownWindow(window.clone()))?;
        if !record.floating {
            self.contexts
                .get_mut(&record.context)
                .ok_or(EngineError::MissingContext(
                    record.context.workspace,
                    record.context.monitor,
                ))?
                .tree
                .remove(window)?;
        }
        if self.focused.as_ref() == Some(window) {
            self.focused = None;
        }
        debug!(event = "WINDOW_REMOVE", window_id = %window.0);
        Ok(())
    }

    fn focus_window(&mut self, window: &WindowId) -> Result<(), EngineError> {
        let record = self
            .windows
            .get(window)
            .ok_or_else(|| EngineError::UnknownWindow(window.clone()))?;
        self.focused = Some(window.clone());
        if !record.floating {
            self.contexts
                .get_mut(&record.context)
                .ok_or(EngineError::MissingContext(
                    record.context.workspace,
                    record.context.monitor,
                ))?
                .tree
                .set_focused(window);
        }
        Ok(())
    }

    fn insert_tiled(
        &mut self,
        window: &WindowId,
        context_key: ContextKey,
    ) -> Result<(), EngineError> {
        let focused = self.focused.as_ref().filter(|focused| {
            self.windows
                .get(*focused)
                .is_some_and(|record| record.context == context_key && !record.floating)
        });
        let context = self
            .contexts
            .get_mut(&context_key)
            .ok_or(EngineError::MissingContext(
                context_key.workspace,
                context_key.monitor,
            ))?;
        context
            .tree
            .insert(window.clone(), focused, context.work_area, &self.config)?;
        Ok(())
    }

    fn move_context(
        &mut self,
        window: &WindowId,
        new_context: ContextKey,
        work_area: crate::geometry::Rect,
    ) -> Result<(), EngineError> {
        if !work_area.is_valid() {
            return Err(EngineError::InvalidWorkArea(
                new_context.workspace,
                new_context.monitor,
            ));
        }
        let record = self
            .windows
            .get(window)
            .cloned()
            .ok_or_else(|| EngineError::UnknownWindow(window.clone()))?;
        self.contexts
            .entry(new_context)
            .and_modify(|context| context.work_area = work_area)
            .or_insert_with(|| TilingContext {
                work_area,
                tree: LayoutTree::new(),
            });

        if record.context == new_context {
            return Ok(());
        }
        if !record.floating {
            self.contexts
                .get_mut(&record.context)
                .ok_or(EngineError::MissingContext(
                    record.context.workspace,
                    record.context.monitor,
                ))?
                .tree
                .remove(window)?;
        }
        self.windows.get_mut(window).expect("record exists").context = new_context;
        if !record.floating {
            self.insert_tiled(window, new_context)?;
        }
        debug!(
            event = "WINDOW_MOVE_CONTEXT",
            window_id = %window.0,
            workspace = new_context.workspace,
            monitor = new_context.monitor
        );
        Ok(())
    }

    fn directional_neighbor(
        &self,
        window: &WindowId,
        direction: Direction,
    ) -> Result<Option<WindowId>, EngineError> {
        let record = self
            .windows
            .get(window)
            .ok_or_else(|| EngineError::UnknownWindow(window.clone()))?;
        if record.floating {
            return Ok(None);
        }
        let context = self
            .contexts
            .get(&record.context)
            .ok_or(EngineError::MissingContext(
                record.context.workspace,
                record.context.monitor,
            ))?;
        let rects = context.tree.rectangles(
            context.work_area,
            self.config.outer_gap,
            self.config.inner_gap,
        );
        Ok(best_candidate(window, direction, &rects))
    }

    fn swap_direction(
        &mut self,
        window: &WindowId,
        direction: Direction,
    ) -> Result<(), EngineError> {
        let Some(neighbor) = self.directional_neighbor(window, direction)? else {
            return Ok(());
        };
        let context_key = self.windows[window].context;
        self.contexts
            .get_mut(&context_key)
            .expect("window context exists")
            .tree
            .swap(window, &neighbor)?;
        Ok(())
    }

    fn resize(&mut self, window: &WindowId, direction: Direction) -> Result<(), EngineError> {
        let record = self
            .windows
            .get(window)
            .ok_or_else(|| EngineError::UnknownWindow(window.clone()))?;
        if !record.floating {
            self.contexts
                .get_mut(&record.context)
                .expect("window context exists")
                .tree
                .resize(window, direction, self.config.resize_step)?;
            debug!(event = "RESIZE", window_id = %window.0, ?direction);
        }
        Ok(())
    }

    fn toggle_floating(&mut self, window: &WindowId) -> Result<(), EngineError> {
        let record = self
            .windows
            .get(window)
            .cloned()
            .ok_or_else(|| EngineError::UnknownWindow(window.clone()))?;
        if record.floating {
            self.windows
                .get_mut(window)
                .expect("record exists")
                .floating = false;
            self.insert_tiled(window, record.context)?;
        } else {
            self.contexts
                .get_mut(&record.context)
                .expect("window context exists")
                .tree
                .remove(window)?;
            self.windows
                .get_mut(window)
                .expect("record exists")
                .floating = true;
        }
        debug!(event = "FLOAT", window_id = %window.0, floating = !record.floating);
        Ok(())
    }

    fn toggle_group(&mut self, window: &WindowId) -> Result<(), EngineError> {
        let record = self
            .windows
            .get(window)
            .cloned()
            .ok_or_else(|| EngineError::UnknownWindow(window.clone()))?;
        if record.floating {
            return Ok(());
        }
        let context = self
            .contexts
            .get_mut(&record.context)
            .expect("window context exists");
        context
            .tree
            .toggle_group(window, context.work_area, &self.config)?;
        debug!(event = "GROUP_TOGGLE", window_id = %window.0);
        Ok(())
    }

    fn cycle_group(
        &mut self,
        window: &WindowId,
        cycle: CycleDirection,
    ) -> Result<Option<WindowId>, EngineError> {
        let record = self
            .windows
            .get(window)
            .cloned()
            .ok_or_else(|| EngineError::UnknownWindow(window.clone()))?;
        if record.floating {
            return Ok(None);
        }
        let target = self
            .contexts
            .get_mut(&record.context)
            .expect("window context exists")
            .tree
            .cycle_group(window, cycle == CycleDirection::Next)?;
        if let Some(target) = &target {
            self.focused = Some(target.clone());
            debug!(event = "GROUP_CYCLE", window_id = %target.0);
        }
        Ok(target)
    }

    pub fn placements(&self) -> Vec<Placement> {
        let mut contexts: Vec<_> = self.contexts.keys().copied().collect();
        contexts.sort();
        contexts
            .into_iter()
            .flat_map(|context| self.placements_for(context))
            .collect()
    }

    pub fn placements_for(&self, context_key: ContextKey) -> Vec<Placement> {
        let Some(context) = self.contexts.get(&context_key) else {
            return Vec::new();
        };
        let mut placements = context.tree.placements(
            context.work_area,
            self.config.outer_gap,
            self.config.inner_gap,
        );
        placements.sort_by(|a, b| a.window_id.cmp(&b.window_id));
        debug!(
            event = "LAYOUT",
            workspace = context_key.workspace,
            monitor = context_key.monitor,
            windows = placements.len()
        );
        placements
    }

    pub fn validate_invariants(&self) -> Result<(), String> {
        let mut tiled = HashSet::new();
        for (key, context) in &self.contexts {
            context.tree.validate_invariants()?;
            for window in context.tree.windows.keys() {
                if !tiled.insert(window.clone()) {
                    return Err("tiled window exists in multiple contexts".into());
                }
                let record = self
                    .windows
                    .get(window)
                    .ok_or_else(|| "tree window has no state record".to_string())?;
                if record.floating || record.context != *key {
                    return Err("tree window record has wrong context or floating state".into());
                }
            }
        }
        let expected: HashSet<_> = self
            .windows
            .iter()
            .filter(|(_, record)| !record.floating)
            .map(|(window, _)| window.clone())
            .collect();
        if tiled != expected {
            return Err("tiled records differ from context trees".into());
        }
        Ok(())
    }

    pub fn context_len(&self, context: ContextKey) -> usize {
        self.contexts
            .get(&context)
            .map_or(0, |context| context.tree.len())
    }

    pub fn window_context(&self, window: &WindowId) -> Option<ContextKey> {
        self.windows.get(window).map(|record| record.context)
    }

    pub fn is_floating(&self, window: &WindowId) -> Option<bool> {
        self.windows.get(window).map(|record| record.floating)
    }

    fn debug_validate(&self) -> Result<(), EngineError> {
        if cfg!(debug_assertions) {
            self.validate_invariants().map_err(EngineError::Invariant)?;
        }
        Ok(())
    }
}
