use std::collections::HashMap;

use crate::{
    geometry::Rect,
    protocol::{Direction, WindowId},
};

pub fn best_candidate(
    current: &WindowId,
    direction: Direction,
    rects: &HashMap<WindowId, Rect>,
) -> Option<WindowId> {
    let source = *rects.get(current)?;
    let (source_x, source_y) = source.center();

    rects
        .iter()
        .filter(|(id, _)| *id != current)
        .filter_map(|(id, rect)| {
            let (x, y) = rect.center();
            let in_direction = match direction {
                Direction::Left => x < source_x,
                Direction::Right => x > source_x,
                Direction::Up => y < source_y,
                Direction::Down => y > source_y,
            };
            if !in_direction {
                return None;
            }

            let (overlap, primary, perpendicular) = match direction {
                Direction::Left => (
                    ranges_overlap(source.y, source.height, rect.y, rect.height),
                    (i64::from(source.x) - rect.right()).max(0) as f64,
                    (source_y - y).abs(),
                ),
                Direction::Right => (
                    ranges_overlap(source.y, source.height, rect.y, rect.height),
                    (i64::from(rect.x) - source.right()).max(0) as f64,
                    (source_y - y).abs(),
                ),
                Direction::Up => (
                    ranges_overlap(source.x, source.width, rect.x, rect.width),
                    (i64::from(source.y) - rect.bottom()).max(0) as f64,
                    (source_x - x).abs(),
                ),
                Direction::Down => (
                    ranges_overlap(source.x, source.width, rect.x, rect.width),
                    (i64::from(rect.y) - source.bottom()).max(0) as f64,
                    (source_x - x).abs(),
                ),
            };
            Some((!overlap, primary, perpendicular, id))
        })
        .min_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1.total_cmp(&b.1))
                .then_with(|| a.2.total_cmp(&b.2))
                .then_with(|| a.3.cmp(b.3))
        })
        .map(|(_, _, _, id)| id.clone())
}

fn ranges_overlap(a_start: i32, a_size: i32, b_start: i32, b_size: i32) -> bool {
    i64::from(a_start) < i64::from(b_start) + i64::from(b_size)
        && i64::from(b_start) < i64::from(a_start) + i64::from(a_size)
}
