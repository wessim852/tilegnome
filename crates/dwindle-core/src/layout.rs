use std::collections::HashMap;

use crate::{
    geometry::Rect,
    protocol::{Placement, WindowId},
    tree::{LayoutTree, NodeKey, NodeKind, Orientation},
};

impl LayoutTree {
    pub fn placements(&self, work_area: Rect, outer_gap: i32, inner_gap: i32) -> Vec<Placement> {
        let Some(root) = self.root else {
            return Vec::new();
        };
        let mut placements = Vec::with_capacity(self.len());
        self.layout_node(root, work_area.inset(outer_gap), inner_gap, &mut placements);
        placements
    }

    pub fn rectangles(
        &self,
        work_area: Rect,
        outer_gap: i32,
        inner_gap: i32,
    ) -> HashMap<WindowId, Rect> {
        self.placements(work_area, outer_gap, inner_gap)
            .into_iter()
            .map(|placement| (placement.window_id, placement.rect))
            .collect()
    }

    fn layout_node(&self, key: NodeKey, rect: Rect, inner_gap: i32, output: &mut Vec<Placement>) {
        match &self.nodes[key].kind {
            NodeKind::Leaf { windows, .. } => {
                output.extend(
                    windows
                        .iter()
                        .cloned()
                        .map(|window_id| Placement { window_id, rect }),
                );
            }
            NodeKind::Split {
                orientation,
                ratio,
                first,
                second,
            } => {
                let (first_rect, second_rect) = match orientation {
                    Orientation::Horizontal => rect.split_horizontal(*ratio, inner_gap),
                    Orientation::Vertical => rect.split_vertical(*ratio, inner_gap),
                };
                self.layout_node(*first, first_rect, inner_gap, output);
                self.layout_node(*second, second_rect, inner_gap, output);
            }
        }
    }
}
