use std::collections::{HashMap, HashSet};

use thiserror::Error;
use tracing::{debug, info};

use crate::{
    navigation::best_candidate,
    protocol::{
        Command, Config, ContextKey, Direction, Placement, Response, Snapshot, WindowId,
        WindowSnapshot,
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
    maximized: Option<WindowId>,
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
                let context = self.add_window(window, work_area)?;
                Response::Placements {
                    placements: self.placements_for(context),
                }
            }
            Command::RemoveWindow { window_id } => {
                let context = self.remove_window(&window_id)?;
                Response::Placements {
                    placements: self.placements_for(context),
                }
            }
            Command::FocusWindow { window_id } => {
                let context = self.focus_window(&window_id)?;
                match context {
                    Some(context) => Response::Placements {
                        placements: self.placements_for(context),
                    },
                    None => Response::Ack,
                }
            }
            Command::WindowContextChanged {
                window_id,
                context,
                work_area,
            } => {
                let contexts = self.move_context(&window_id, context, work_area)?;
                Response::Placements {
                    placements: self.placements_for_contexts(&contexts),
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
                Some(target) => {
                    let context = self.windows[&window_id].context;
                    let cancelled = self.focus_window(&target)?.is_some();
                    debug!(event = "FOCUS", window_id = %target.0);
                    if cancelled {
                        Response::PlacementsAndFocus {
                            placements: self.placements_for(context),
                            window_id: target,
                        }
                    } else {
                        Response::Focus { window_id: target }
                    }
                }
                None => Response::Ack,
            },
            Command::MoveDirection {
                window_id,
                direction,
            } => {
                let context = self.swap_direction(&window_id, direction)?;
                Response::Placements {
                    placements: self.placements_for(context),
                }
            }
            Command::Resize {
                window_id,
                direction,
            } => {
                let context = self.resize(&window_id, direction)?;
                Response::Placements {
                    placements: self.placements_for(context),
                }
            }
            Command::ToggleFloating { window_id } => {
                let context = self.toggle_floating(&window_id)?;
                Response::Placements {
                    placements: self.placements_for(context),
                }
            }
            Command::ToggleMaximize { window_id } => match self.toggle_maximize(&window_id)? {
                Some(context) => Response::Placements {
                    placements: self.placements_for(context),
                },
                None => Response::Ack,
            },
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
                    maximized: None,
                },
            );
        }

        let mut rebuilt = Self::new(config);
        rebuilt.contexts = contexts;
        rebuilt.focused = snapshot
            .windows
            .iter()
            .find(|window| window.focused)
            .map(|window| window.id.clone());

        let mut windows = snapshot.windows;
        windows.sort_by(|a, b| a.id.cmp(&b.id));
        for window in windows {
            rebuilt.add_snapshot(window)?;
        }
        if let Some(focused) = rebuilt.focused.clone() {
            rebuilt.focus_window(&focused)?;
        }
        rebuilt.debug_validate()?;
        *self = rebuilt;
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
        if !floating && let Err(error) = self.insert_tiled(&window.id, window.context) {
            self.windows.remove(&window.id);
            return Err(error);
        }
        Ok(())
    }

    fn add_window(
        &mut self,
        window: WindowSnapshot,
        work_area: crate::geometry::Rect,
    ) -> Result<ContextKey, EngineError> {
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
                maximized: None,
            });
        let focused = window.focused;
        let window_id = window.id.clone();
        let context_key = window.context;
        self.add_snapshot(window)?;
        if !self.windows[&window_id].floating {
            self.contexts
                .get_mut(&context_key)
                .expect("window context exists")
                .maximized = None;
        }
        if focused {
            self.focus_window(&window_id)?;
        }
        debug!(event = "WINDOW_ADD", window_id = %window_id.0);
        Ok(context_key)
    }

    fn remove_window(&mut self, window: &WindowId) -> Result<ContextKey, EngineError> {
        let record = self
            .windows
            .get(window)
            .cloned()
            .ok_or_else(|| EngineError::UnknownWindow(window.clone()))?;
        if !record.floating {
            let context =
                self.contexts
                    .get_mut(&record.context)
                    .ok_or(EngineError::MissingContext(
                        record.context.workspace,
                        record.context.monitor,
                    ))?;
            context.tree.remove(window)?;
            if context.maximized.as_ref() == Some(window) {
                context.maximized = None;
            }
        }
        self.windows.remove(window);
        if self.focused.as_ref() == Some(window) {
            self.focused = None;
        }
        debug!(event = "WINDOW_REMOVE", window_id = %window.0);
        Ok(record.context)
    }

    fn focus_window(&mut self, window: &WindowId) -> Result<Option<ContextKey>, EngineError> {
        let record = self
            .windows
            .get(window)
            .cloned()
            .ok_or_else(|| EngineError::UnknownWindow(window.clone()))?;
        self.focused = Some(window.clone());
        if !record.floating {
            let context =
                self.contexts
                    .get_mut(&record.context)
                    .ok_or(EngineError::MissingContext(
                        record.context.workspace,
                        record.context.monitor,
                    ))?;
            context.tree.set_focused(window);
            if context
                .maximized
                .as_ref()
                .is_some_and(|maximized| maximized != window)
            {
                context.maximized = None;
                return Ok(Some(record.context));
            }
        }
        Ok(None)
    }

    fn insert_tiled(
        &mut self,
        window: &WindowId,
        context_key: ContextKey,
    ) -> Result<(), EngineError> {
        self.windows
            .get(window)
            .ok_or_else(|| EngineError::UnknownWindow(window.clone()))?;
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
    ) -> Result<Vec<ContextKey>, EngineError> {
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
                maximized: None,
            });

        if record.context == new_context {
            return Ok(vec![new_context]);
        }
        let old_context_state =
            self.contexts
                .get(&record.context)
                .cloned()
                .ok_or(EngineError::MissingContext(
                    record.context.workspace,
                    record.context.monitor,
                ))?;
        let new_context_state = self.contexts[&new_context].clone();
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
            if let Err(error) = self.insert_tiled(window, new_context) {
                self.contexts.insert(record.context, old_context_state);
                self.contexts.insert(new_context, new_context_state);
                self.windows.get_mut(window).expect("record exists").context = record.context;
                return Err(error);
            }
            let old_context = self
                .contexts
                .get_mut(&record.context)
                .expect("old window context exists");
            if old_context.maximized.as_ref() == Some(window) {
                old_context.maximized = None;
            }
            self.contexts
                .get_mut(&new_context)
                .expect("new window context exists")
                .maximized = None;
        }
        debug!(
            event = "WINDOW_MOVE_CONTEXT",
            window_id = %window.0,
            workspace = new_context.workspace,
            monitor = new_context.monitor
        );
        Ok(vec![record.context, new_context])
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
    ) -> Result<ContextKey, EngineError> {
        let context_key = self
            .windows
            .get(window)
            .ok_or_else(|| EngineError::UnknownWindow(window.clone()))?
            .context;
        let context = self
            .contexts
            .get_mut(&context_key)
            .expect("window context exists");
        if context.maximized.as_ref() == Some(window) {
            context.maximized = None;
        }
        let Some(neighbor) = self.directional_neighbor(window, direction)? else {
            return Ok(context_key);
        };
        self.contexts
            .get_mut(&context_key)
            .expect("window context exists")
            .tree
            .swap(window, &neighbor)?;
        Ok(context_key)
    }

    fn resize(
        &mut self,
        window: &WindowId,
        direction: Direction,
    ) -> Result<ContextKey, EngineError> {
        let record = self
            .windows
            .get(window)
            .cloned()
            .ok_or_else(|| EngineError::UnknownWindow(window.clone()))?;
        if !record.floating {
            let context = self
                .contexts
                .get_mut(&record.context)
                .expect("window context exists");
            if context.maximized.as_ref() == Some(window) {
                context.maximized = None;
            }
            context
                .tree
                .resize(window, direction, self.config.resize_step)?;
            debug!(event = "RESIZE", window_id = %window.0, ?direction);
        }
        Ok(record.context)
    }

    fn toggle_floating(&mut self, window: &WindowId) -> Result<ContextKey, EngineError> {
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
            if let Err(error) = self.insert_tiled(window, record.context) {
                self.windows
                    .get_mut(window)
                    .expect("record exists")
                    .floating = true;
                return Err(error);
            }
            self.contexts
                .get_mut(&record.context)
                .expect("window context exists")
                .maximized = None;
        } else {
            let context = self
                .contexts
                .get_mut(&record.context)
                .expect("window context exists");
            context.tree.remove(window)?;
            if context.maximized.as_ref() == Some(window) {
                context.maximized = None;
            }
            self.windows
                .get_mut(window)
                .expect("record exists")
                .floating = true;
        }
        debug!(event = "FLOAT", window_id = %window.0, floating = !record.floating);
        Ok(record.context)
    }

    fn toggle_maximize(&mut self, window: &WindowId) -> Result<Option<ContextKey>, EngineError> {
        let record = self
            .windows
            .get(window)
            .cloned()
            .ok_or_else(|| EngineError::UnknownWindow(window.clone()))?;
        if record.floating {
            debug!(event = "MAXIMIZE_IGNORED_FLOATING", window_id = %window.0);
            return Ok(None);
        }
        let context = self
            .contexts
            .get_mut(&record.context)
            .expect("window context exists");
        if context.maximized.as_ref() == Some(window) {
            context.maximized = None;
        } else {
            context.maximized = Some(window.clone());
            context.tree.set_focused(window);
            self.focused = Some(window.clone());
        }
        debug!(event = "MAXIMIZE", window_id = %window.0, maximized = context.maximized.is_some());
        Ok(Some(record.context))
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
        if let Some(maximized) = &context.maximized
            && let Some(placement) = placements
                .iter_mut()
                .find(|placement| &placement.window_id == maximized)
        {
            placement.rect = context.work_area;
        }
        placements.sort_by(|a, b| a.window_id.cmp(&b.window_id));
        debug!(
            event = "LAYOUT",
            workspace = context_key.workspace,
            monitor = context_key.monitor,
            windows = placements.len()
        );
        placements
    }

    fn placements_for_contexts(&self, contexts: &[ContextKey]) -> Vec<Placement> {
        let mut contexts = contexts.to_vec();
        contexts.sort();
        contexts.dedup();
        contexts
            .into_iter()
            .flat_map(|context| self.placements_for(context))
            .collect()
    }

    pub fn validate_invariants(&self) -> Result<(), String> {
        let mut tiled = HashSet::new();
        for (key, context) in &self.contexts {
            context.tree.validate_invariants()?;
            if let Some(maximized) = &context.maximized {
                if !context.tree.contains(maximized) {
                    return Err("maximized window is not tiled in its context".into());
                }
                let record = self
                    .windows
                    .get(maximized)
                    .ok_or_else(|| "maximized window has no state record".to_string())?;
                if record.floating || record.context != *key {
                    return Err("maximized window has wrong context or is floating".into());
                }
            }
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

    pub fn maximized_window(&self, context: ContextKey) -> Option<&WindowId> {
        self.contexts
            .get(&context)
            .and_then(|context| context.maximized.as_ref())
    }

    #[cfg(test)]
    pub(crate) fn tree(&self, context: ContextKey) -> Option<&LayoutTree> {
        self.contexts.get(&context).map(|context| &context.tree)
    }

    fn debug_validate(&self) -> Result<(), EngineError> {
        if cfg!(debug_assertions) {
            self.validate_invariants().map_err(EngineError::Invariant)?;
        }
        Ok(())
    }
}
