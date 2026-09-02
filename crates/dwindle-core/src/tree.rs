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
        window: WindowId,
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
    #[error("target rectangle {width}x{height} cannot be split into two positive rectangles")]
    CannotSplit { width: i32, height: i32 },
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
        if self.contains(window) {
            self.last_focused = Some(window.clone());
        }
    }

    pub fn insert(
        &mut self,
        window: WindowId,
        focused: Option<&WindowId>,
        work_area: Rect,
        config: &Config,
    ) -> Result<(), TreeError> {
        if self.contains(&window) {
            return Err(TreeError::DuplicateWindow(window));
        }

        if self.root.is_none() {
            let leaf = self.nodes.insert(Node {
                parent: None,
                kind: NodeKind::Leaf {
                    window: window.clone(),
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
                    .max_by(|(left_id, left), (right_id, right)| {
                        left.area()
                            .cmp(&right.area())
                            .then_with(|| right_id.cmp(left_id))
                    })
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
        let orientation = if split_fits(target_rect, preferred) {
            preferred
        } else if split_fits(target_rect, alternate) {
            alternate
        } else {
            return Err(TreeError::CannotSplit {
                width: target_rect.width,
                height: target_rect.height,
            });
        };

        self.split_leaf(&target, window.clone(), orientation, config.split_ratio)?;
        self.last_focused = Some(window);
        debug!(event = "TREE_INSERT", windows = self.len());
        self.debug_validate();
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
        let replace_first = if let Some(parent) = old_parent {
            match self.nodes[parent].kind {
                NodeKind::Split { first, .. } if first == old_leaf => Some(true),
                NodeKind::Split { second, .. } if second == old_leaf => Some(false),
                _ => {
                    return Err(TreeError::Corrupt(
                        "leaf parent does not reference leaf".into(),
                    ));
                }
            }
        } else {
            None
        };
        let new_leaf = self.nodes.insert(Node {
            parent: None,
            kind: NodeKind::Leaf {
                window: window.clone(),
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
            if replace_first == Some(true) {
                let NodeKind::Split { first, .. } = &mut self.nodes[parent].kind else {
                    unreachable!("parent was validated above");
                };
                *first = split;
            } else {
                let NodeKind::Split { second, .. } = &mut self.nodes[parent].kind else {
                    unreachable!("parent was validated above");
                };
                *second = split;
            }
        } else {
            self.root = Some(split);
        }
        self.windows.insert(window, new_leaf);
        Ok(())
    }

    pub fn remove(&mut self, window: &WindowId) -> Result<(), TreeError> {
        let leaf = *self
            .windows
            .get(window)
            .ok_or_else(|| TreeError::UnknownWindow(window.clone()))?;
        let parent = self.nodes[leaf].parent;

        if let Some(parent) = parent {
            let (first, second) = match self.nodes[parent].kind {
                NodeKind::Split { first, second, .. } => (first, second),
                NodeKind::Leaf { .. } => {
                    return Err(TreeError::Corrupt("leaf parent is not a split".into()));
                }
            };
            let sibling = if first == leaf {
                second
            } else if second == leaf {
                first
            } else {
                return Err(TreeError::Corrupt(
                    "leaf parent does not reference leaf".into(),
                ));
            };
            let grandparent = self.nodes[parent].parent;

            if let Some(grandparent) = grandparent {
                match &mut self.nodes[grandparent].kind {
                    NodeKind::Split { first, .. } if *first == parent => *first = sibling,
                    NodeKind::Split { second, .. } if *second == parent => *second = sibling,
                    _ => {
                        return Err(TreeError::Corrupt(
                            "split parent does not reference split".into(),
                        ));
                    }
                }
            } else {
                self.root = Some(sibling);
            }
            self.nodes[sibling].parent = grandparent;
            self.nodes.remove(leaf);
            self.nodes.remove(parent);
            debug!(event = "TREE_COLLAPSE", windows = self.len() - 1);
        } else {
            self.nodes.remove(leaf);
            self.root = None;
        }

        self.windows.remove(window);
        if self.last_focused.as_ref() == Some(window) {
            self.last_focused = self.windows.keys().min().cloned();
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
            return Err(TreeError::Corrupt(
                "different windows point to the same leaf".into(),
            ));
        }
        if !matches!(self.nodes[first_key].kind, NodeKind::Leaf { .. })
            || !matches!(self.nodes[second_key].kind, NodeKind::Leaf { .. })
        {
            return Err(TreeError::Corrupt("window lookup points to a split".into()));
        }

        self.nodes[first_key].kind = NodeKind::Leaf {
            window: second_window.clone(),
        };
        self.nodes[second_key].kind = NodeKind::Leaf {
            window: first_window.clone(),
        };
        self.windows.insert(first_window.clone(), second_key);
        self.windows.insert(second_window.clone(), first_key);
        self.debug_validate();
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
        if self.nodes.get(root).is_none() {
            return Err("root references a missing node".into());
        }
        if self.nodes[root].parent.is_some() {
            return Err("root has a parent".into());
        }

        let mut stack = vec![root];
        let mut visited = HashSet::new();
        let mut leaves = HashMap::new();
        while let Some(key) = stack.pop() {
            if !visited.insert(key) {
                return Err("node is reachable more than once or the tree contains a cycle".into());
            }
            let node = self
                .nodes
                .get(key)
                .ok_or_else(|| "tree references missing node".to_string())?;
            match &node.kind {
                NodeKind::Leaf { window } => {
                    if leaves.insert(window.clone(), key).is_some() {
                        return Err("window appears in multiple leaves".into());
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

fn split_fits(rect: Rect, orientation: Orientation) -> bool {
    match orientation {
        Orientation::Horizontal => rect.width >= 2,
        Orientation::Vertical => rect.height >= 2,
    }
}
