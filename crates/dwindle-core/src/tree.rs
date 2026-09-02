use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use slotmap::{SlotMap, new_key_type};
use thiserror::Error;
use tracing::debug;

use crate::{
    geometry::Rect,
    protocol::{Config, Direction, WindowId},
};

pub const MIN_RATIO: f64 = 0.1;
pub const MAX_RATIO: f64 = 0.9;

new_key_type! { pub(crate) struct NodeKey; }

/// Horizontal splits arrange children left/right and divide width.
/// Vertical splits arrange children top/bottom and divide height.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Orientation {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug)]
pub(crate) enum NodeKind {
    Leaf {
        windows: Vec<WindowId>,
        active: usize,
    },
    Split {
        orientation: Orientation,
        ratio: f64,
        first: NodeKey,
        second: NodeKey,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct Node {
    pub(crate) parent: Option<NodeKey>,
    pub(crate) kind: NodeKind,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum TreeError {
    #[error("window already exists: {0:?}")]
    DuplicateWindow(WindowId),
    #[error("window is not tiled: {0:?}")]
    UnknownWindow(WindowId),
    #[error("layout tree is corrupt: {0}")]
    Corrupt(String),
}

#[derive(Clone, Debug, Default)]
pub struct LayoutTree {
    pub(crate) root: Option<NodeKey>,
    pub(crate) nodes: SlotMap<NodeKey, Node>,
    pub(crate) windows: HashMap<WindowId, NodeKey>,
    last_focused: Option<WindowId>,
}

impl LayoutTree {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    pub fn contains(&self, window: &WindowId) -> bool {
        self.windows.contains_key(window)
    }

    pub fn set_focused(&mut self, window: &WindowId) {
        let Some(&leaf) = self.windows.get(window) else {
            return;
        };
        if let NodeKind::Leaf { windows, active } = &mut self.nodes[leaf].kind {
            *active = windows
                .iter()
                .position(|candidate| candidate == window)
                .unwrap_or(0);
        }
        self.last_focused = Some(window.clone());
    }

    pub fn insert(
        &mut self,
        window: WindowId,
        focused: Option<&WindowId>,
        work_area: Rect,
        config: &Config,
        min_size: (i32, i32),
    ) -> Result<(), TreeError> {
        if self.contains(&window) {
            return Err(TreeError::DuplicateWindow(window));
        }

        if self.root.is_none() {
            let leaf = self.nodes.insert(Node {
                parent: None,
                kind: NodeKind::Leaf {
                    windows: vec![window.clone()],
                    active: 0,
                },
            });
            self.root = Some(leaf);
            self.windows.insert(window.clone(), leaf);
            self.last_focused = Some(window);
            debug!(event = "TREE_INSERT", windows = self.len());
            self.debug_validate();
            return Ok(());
        }

        let rects = self.rectangles(work_area, config.outer_gap, config.inner_gap);
        let target = focused
            .filter(|id| self.contains(id))
            .cloned()
            .or_else(|| self.last_focused.clone().filter(|id| self.contains(id)))
            .or_else(|| {
                rects
                    .iter()
                    .max_by_key(|(_, rect)| rect.area())
                    .map(|(id, _)| id.clone())
            })
            .ok_or_else(|| TreeError::Corrupt("non-empty tree has no leaf".into()))?;
        let target_rect = rects
            .get(&target)
            .copied()
            .ok_or_else(|| TreeError::Corrupt("target leaf has no rectangle".into()))?;
        let preferred = if config.smart_split && target_rect.height > target_rect.width {
            Orientation::Vertical
        } else {
            Orientation::Horizontal
        };
        let alternate = match preferred {
            Orientation::Horizontal => Orientation::Vertical,
            Orientation::Vertical => Orientation::Horizontal,
        };
        let orientation = if split_fits(target_rect, preferred, config, min_size) {
            preferred
        } else if split_fits(target_rect, alternate, config, min_size) {
            alternate
        } else {
            let root_rect = work_area.inset(config.outer_gap);
            let root_preferred = if config.smart_split && root_rect.height > root_rect.width {
                Orientation::Vertical
            } else {
                Orientation::Horizontal
            };
            let root_alternate = match root_preferred {
                Orientation::Horizontal => Orientation::Vertical,
                Orientation::Vertical => Orientation::Horizontal,
            };
            // ponytail: this guarantees the new client's hints; track constraints per
            // tree leaf if constrained existing clients later need global reflow.
            let root_orientation = [root_preferred, root_alternate]
                .into_iter()
                .find(|orientation| split_fits(root_rect, *orientation, config, min_size));
            if let Some(root_orientation) = root_orientation {
                self.wrap_root(window.clone(), root_orientation, config.split_ratio)?;
                self.last_focused = Some(window);
                debug!(event = "TREE_INSERT_ROOT_FALLBACK", windows = self.len());
                self.debug_validate();
                return Ok(());
            }
            preferred
        };
        self.split_leaf(&target, window.clone(), orientation, config.split_ratio)?;
        self.last_focused = Some(window);
        debug!(event = "TREE_INSERT", windows = self.len());
        self.debug_validate();
        Ok(())
    }

    fn wrap_root(
        &mut self,
        window: WindowId,
        orientation: Orientation,
        ratio: f64,
    ) -> Result<(), TreeError> {
        let old_root = self
            .root
            .ok_or_else(|| TreeError::Corrupt("non-empty tree has no root".into()))?;
        if let NodeKind::Split {
            orientation: old_orientation,
            ..
        } = &mut self.nodes[old_root].kind
            && *old_orientation == orientation
        {
            *old_orientation = match orientation {
                Orientation::Horizontal => Orientation::Vertical,
                Orientation::Vertical => Orientation::Horizontal,
            };
        }
        let new_leaf = self.nodes.insert(Node {
            parent: None,
            kind: NodeKind::Leaf {
                windows: vec![window.clone()],
                active: 0,
            },
        });
        let new_root = self.nodes.insert(Node {
            parent: None,
            kind: NodeKind::Split {
                orientation,
                ratio: ratio.clamp(MIN_RATIO, MAX_RATIO),
                first: old_root,
                second: new_leaf,
            },
        });
        self.nodes[old_root].parent = Some(new_root);
        self.nodes[new_leaf].parent = Some(new_root);
        self.root = Some(new_root);
        self.windows.insert(window, new_leaf);
        Ok(())
    }

    fn split_leaf(
        &mut self,
        target: &WindowId,
        window: WindowId,
        orientation: Orientation,
        ratio: f64,
    ) -> Result<(), TreeError> {
        let old_leaf = *self
            .windows
            .get(target)
            .ok_or_else(|| TreeError::UnknownWindow(target.clone()))?;
        let old_parent = self.nodes[old_leaf].parent;
        let new_leaf = self.nodes.insert(Node {
            parent: None,
            kind: NodeKind::Leaf {
                windows: vec![window.clone()],
                active: 0,
            },
        });
        let split = self.nodes.insert(Node {
            parent: old_parent,
            kind: NodeKind::Split {
                orientation,
                ratio: ratio.clamp(MIN_RATIO, MAX_RATIO),
                first: old_leaf,
                second: new_leaf,
            },
        });
        self.nodes[old_leaf].parent = Some(split);
        self.nodes[new_leaf].parent = Some(split);

        if let Some(parent) = old_parent {
            match &mut self.nodes[parent].kind {
                NodeKind::Split { first, second, .. } if *first == old_leaf => *first = split,
                NodeKind::Split { first, second, .. } if *second == old_leaf => *second = split,
                _ => {
                    return Err(TreeError::Corrupt(
                        "leaf parent does not reference leaf".into(),
                    ));
                }
            }
        } else {
            self.root = Some(split);
        }
        self.windows.insert(window, new_leaf);
        Ok(())
    }

    pub fn remove(&mut self, window: &WindowId) -> Result<(), TreeError> {
        let leaf = self
            .windows
            .remove(window)
            .ok_or_else(|| TreeError::UnknownWindow(window.clone()))?;
        if let NodeKind::Leaf { windows, active } = &mut self.nodes[leaf].kind
            && windows.len() > 1
        {
            let removed = windows
                .iter()
                .position(|candidate| candidate == window)
                .ok_or_else(|| TreeError::Corrupt("window lookup points to wrong group".into()))?;
            windows.remove(removed);
            if removed < *active {
                *active -= 1;
            } else if *active >= windows.len() {
                *active = windows.len() - 1;
            }
            if self.last_focused.as_ref() == Some(window) {
                self.last_focused = windows.get(*active).cloned();
            }
            self.debug_validate();
            return Ok(());
        }
        let parent = self.nodes[leaf].parent;

        if let Some(parent) = parent {
            let (first, second) = match self.nodes[parent].kind {
                NodeKind::Split { first, second, .. } => (first, second),
                NodeKind::Leaf { .. } => {
                    return Err(TreeError::Corrupt("leaf parent is not a split".into()));
                }
            };
            let sibling = if first == leaf { second } else { first };
            let grandparent = self.nodes[parent].parent;
            self.nodes[sibling].parent = grandparent;

            if let Some(grandparent) = grandparent {
                match &mut self.nodes[grandparent].kind {
                    NodeKind::Split { first, second, .. } if *first == parent => *first = sibling,
                    NodeKind::Split { first, second, .. } if *second == parent => *second = sibling,
                    _ => {
                        return Err(TreeError::Corrupt(
                            "split parent does not reference split".into(),
                        ));
                    }
                }
            } else {
                self.root = Some(sibling);
            }
            self.nodes.remove(leaf);
            self.nodes.remove(parent);
            debug!(event = "TREE_COLLAPSE", windows = self.len());
        } else {
            self.nodes.remove(leaf);
            self.root = None;
        }

        if self.last_focused.as_ref() == Some(window) {
            self.last_focused = self.windows.keys().next().cloned();
        }
        debug!(event = "TREE_REMOVE", windows = self.len());
        self.debug_validate();
        Ok(())
    }

    pub fn swap(
        &mut self,
        first_window: &WindowId,
        second_window: &WindowId,
    ) -> Result<(), TreeError> {
        if first_window == second_window {
            return Ok(());
        }
        let first_key = *self
            .windows
            .get(first_window)
            .ok_or_else(|| TreeError::UnknownWindow(first_window.clone()))?;
        let second_key = *self
            .windows
            .get(second_window)
            .ok_or_else(|| TreeError::UnknownWindow(second_window.clone()))?;
        if first_key == second_key {
            return Ok(());
        }

        let first_kind = self.nodes[first_key].kind.clone();
        let second_kind = self.nodes[second_key].kind.clone();
        if !matches!(first_kind, NodeKind::Leaf { .. })
            || !matches!(second_kind, NodeKind::Leaf { .. })
        {
            return Err(TreeError::Corrupt("window lookup points to a split".into()));
        }
        self.nodes[first_key].kind = second_kind;
        self.nodes[second_key].kind = first_kind;
        self.remap_leaf(first_key)?;
        self.remap_leaf(second_key)?;
        self.debug_validate();
        Ok(())
    }

    pub fn toggle_group(
        &mut self,
        window: &WindowId,
        work_area: Rect,
        config: &Config,
    ) -> Result<bool, TreeError> {
        let leaf = *self
            .windows
            .get(window)
            .ok_or_else(|| TreeError::UnknownWindow(window.clone()))?;
        let grouped = matches!(
            &self.nodes[leaf].kind,
            NodeKind::Leaf { windows, .. } if windows.len() > 1
        );
        if grouped {
            self.ungroup_leaf(leaf, window, work_area, config)?;
        } else if !self.group_parent(leaf, window)? {
            return Ok(false);
        }
        self.debug_validate();
        Ok(true)
    }

    pub fn cycle_group(
        &mut self,
        window: &WindowId,
        forward: bool,
    ) -> Result<Option<WindowId>, TreeError> {
        let leaf = *self
            .windows
            .get(window)
            .ok_or_else(|| TreeError::UnknownWindow(window.clone()))?;
        let NodeKind::Leaf { windows, active } = &mut self.nodes[leaf].kind else {
            return Err(TreeError::Corrupt("window lookup points to a split".into()));
        };
        if windows.len() < 2 {
            return Ok(None);
        }
        *active = if forward {
            (*active + 1) % windows.len()
        } else {
            (*active + windows.len() - 1) % windows.len()
        };
        let target = windows[*active].clone();
        self.last_focused = Some(target.clone());
        Ok(Some(target))
    }

    fn group_parent(&mut self, leaf: NodeKey, focused: &WindowId) -> Result<bool, TreeError> {
        let Some(parent) = self.nodes[leaf].parent else {
            return Ok(false);
        };
        let mut windows = Vec::new();
        self.collect_windows(parent, &mut windows)?;
        let active = windows
            .iter()
            .position(|window| window == focused)
            .ok_or_else(|| TreeError::Corrupt("focused window missing from group".into()))?;
        let mut descendants = Vec::new();
        self.collect_descendants(parent, &mut descendants)?;
        for key in descendants {
            self.nodes.remove(key);
        }
        self.nodes[parent].kind = NodeKind::Leaf {
            windows: windows.clone(),
            active,
        };
        for window in windows {
            self.windows.insert(window, parent);
        }
        self.last_focused = Some(focused.clone());
        Ok(true)
    }

    fn ungroup_leaf(
        &mut self,
        leaf: NodeKey,
        focused: &WindowId,
        work_area: Rect,
        config: &Config,
    ) -> Result<(), TreeError> {
        let NodeKind::Leaf { windows, .. } = &self.nodes[leaf].kind else {
            return Err(TreeError::Corrupt("window lookup points to a split".into()));
        };
        let members = windows.clone();
        self.nodes[leaf].kind = NodeKind::Leaf {
            windows: vec![members[0].clone()],
            active: 0,
        };
        for member in &members[1..] {
            self.windows.remove(member);
        }
        self.windows.insert(members[0].clone(), leaf);
        let mut target = members[0].clone();
        for member in members.into_iter().skip(1) {
            self.insert(member.clone(), Some(&target), work_area, config, (0, 0))?;
            target = member;
        }
        self.set_focused(focused);
        Ok(())
    }

    fn collect_windows(&self, key: NodeKey, output: &mut Vec<WindowId>) -> Result<(), TreeError> {
        match &self.nodes[key].kind {
            NodeKind::Leaf { windows, .. } => output.extend(windows.iter().cloned()),
            NodeKind::Split { first, second, .. } => {
                self.collect_windows(*first, output)?;
                self.collect_windows(*second, output)?;
            }
        }
        Ok(())
    }

    fn collect_descendants(
        &self,
        key: NodeKey,
        output: &mut Vec<NodeKey>,
    ) -> Result<(), TreeError> {
        if let NodeKind::Split { first, second, .. } = self.nodes[key].kind {
            self.collect_descendants(first, output)?;
            self.collect_descendants(second, output)?;
            output.push(first);
            output.push(second);
        }
        Ok(())
    }

    fn remap_leaf(&mut self, key: NodeKey) -> Result<(), TreeError> {
        let NodeKind::Leaf { windows, .. } = &self.nodes[key].kind else {
            return Err(TreeError::Corrupt("expected leaf".into()));
        };
        for window in windows.clone() {
            self.windows.insert(window, key);
        }
        Ok(())
    }

    pub fn resize(
        &mut self,
        window: &WindowId,
        direction: Direction,
        step: f64,
    ) -> Result<bool, TreeError> {
        let mut child = *self
            .windows
            .get(window)
            .ok_or_else(|| TreeError::UnknownWindow(window.clone()))?;

        while let Some(parent) = self.nodes[child].parent {
            let (orientation, first, second) = match self.nodes[parent].kind {
                NodeKind::Split {
                    orientation,
                    first,
                    second,
                    ..
                } => (orientation, first, second),
                NodeKind::Leaf { .. } => {
                    return Err(TreeError::Corrupt("ancestor is not a split".into()));
                }
            };
            let is_first = child == first;
            let same_axis = matches!(
                (direction, orientation),
                (Direction::Left | Direction::Right, Orientation::Horizontal)
                    | (Direction::Up | Direction::Down, Orientation::Vertical)
            );
            if same_axis {
                if let NodeKind::Split { ratio, .. } = &mut self.nodes[parent].kind {
                    let grow = matches!(direction, Direction::Right | Direction::Down);
                    let delta = if grow == is_first { step } else { -step };
                    *ratio = (*ratio + delta).clamp(MIN_RATIO, MAX_RATIO);
                }
                self.debug_validate();
                return Ok(true);
            }
            debug_assert!(child == first || child == second);
            child = parent;
        }
        Ok(false)
    }

    pub fn validate_invariants(&self) -> Result<(), String> {
        if self.root.is_none() {
            if self.nodes.is_empty() && self.windows.is_empty() {
                return Ok(());
            }
            return Err("empty root has nodes or window lookup entries".into());
        }

        let root = self.root.expect("checked above");
        if self.nodes[root].parent.is_some() {
            return Err("root has a parent".into());
        }

        let mut stack = vec![root];
        let mut visited = HashSet::new();
        let mut leaves = HashMap::new();
        while let Some(key) = stack.pop() {
            if !visited.insert(key) {
                return Err("node is reachable more than once".into());
            }
            let node = self
                .nodes
                .get(key)
                .ok_or_else(|| "tree references missing node".to_string())?;
            match &node.kind {
                NodeKind::Leaf { windows, active } => {
                    if windows.is_empty() || *active >= windows.len() {
                        return Err("group leaf is empty or has an invalid active index".into());
                    }
                    for window in windows {
                        if leaves.insert(window.clone(), key).is_some() {
                            return Err("window appears in multiple leaves".into());
                        }
                    }
                }
                NodeKind::Split {
                    ratio,
                    first,
                    second,
                    ..
                } => {
                    if first == second {
                        return Err("split children are identical".into());
                    }
                    if !ratio.is_finite() || !(MIN_RATIO..=MAX_RATIO).contains(ratio) {
                        return Err("split ratio is outside safe bounds".into());
                    }
                    for child in [first, second] {
                        if self.nodes.get(*child).and_then(|node| node.parent) != Some(key) {
                            return Err("child has incorrect parent".into());
                        }
                        stack.push(*child);
                    }
                }
            }
        }

        if visited.len() != self.nodes.len() {
            return Err("arena contains unreachable nodes".into());
        }
        if leaves != self.windows {
            return Err("window lookup differs from leaves".into());
        }
        Ok(())
    }

    fn debug_validate(&self) {
        debug_assert!(
            self.validate_invariants().is_ok(),
            "layout tree invariant failed: {:?}",
            self.validate_invariants()
        );
    }
}

fn split_fits(
    rect: Rect,
    orientation: Orientation,
    config: &Config,
    (min_width, min_height): (i32, i32),
) -> bool {
    let (_, second) = match orientation {
        Orientation::Horizontal => rect.split_horizontal(config.split_ratio, config.inner_gap),
        Orientation::Vertical => rect.split_vertical(config.split_ratio, config.inner_gap),
    };
    second.width >= min_width.max(0) && second.height >= min_height.max(0)
}
