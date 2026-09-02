use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub fn is_valid(self) -> bool {
        self.width > 0
            && self.height > 0
            && self.right() <= i32::MAX as i64
            && self.bottom() <= i32::MAX as i64
            && self.right() >= i32::MIN as i64
            && self.bottom() >= i32::MIN as i64
    }

    pub fn right(self) -> i64 {
        i64::from(self.x) + i64::from(self.width)
    }

    pub fn bottom(self) -> i64 {
        i64::from(self.y) + i64::from(self.height)
    }

    pub fn area(self) -> i64 {
        i64::from(self.width.max(0)) * i64::from(self.height.max(0))
    }

    pub fn center(self) -> (f64, f64) {
        (
            f64::from(self.x) + f64::from(self.width) / 2.0,
            f64::from(self.y) + f64::from(self.height) / 2.0,
        )
    }

    pub fn inset(self, requested: i32) -> Self {
        let gap = requested
            .max(0)
            .min(self.width.saturating_sub(1).max(0) / 2)
            .min(self.height.saturating_sub(1).max(0) / 2);
        Self {
            x: self.x.saturating_add(gap),
            y: self.y.saturating_add(gap),
            width: self.width.saturating_sub(gap.saturating_mul(2)),
            height: self.height.saturating_sub(gap.saturating_mul(2)),
        }
    }

    pub fn contains(self, other: Self) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.right() <= self.right()
            && other.bottom() <= self.bottom()
    }

    pub(crate) fn split_horizontal(self, ratio: f64, requested_gap: i32) -> (Self, Self) {
        if self.width <= 1 {
            return (self, self);
        }
        let gap = requested_gap
            .max(0)
            .min(self.width.saturating_sub(2).max(0));
        let available = self.width.saturating_sub(gap);
        let first_width = (f64::from(available) * ratio)
            .round()
            .clamp(1.0, f64::from(available.saturating_sub(1).max(1)))
            as i32;
        let second_width = available - first_width;
        (
            Self {
                width: first_width,
                ..self
            },
            Self {
                x: self.x.saturating_add(first_width).saturating_add(gap),
                width: second_width,
                ..self
            },
        )
    }

    pub(crate) fn split_vertical(self, ratio: f64, requested_gap: i32) -> (Self, Self) {
        if self.height <= 1 {
            return (self, self);
        }
        let gap = requested_gap
            .max(0)
            .min(self.height.saturating_sub(2).max(0));
        let available = self.height.saturating_sub(gap);
        let first_height = (f64::from(available) * ratio)
            .round()
            .clamp(1.0, f64::from(available.saturating_sub(1).max(1)))
            as i32;
        let second_height = available - first_height;
        (
            Self {
                height: first_height,
                ..self
            },
            Self {
                y: self.y.saturating_add(first_height).saturating_add(gap),
                height: second_height,
                ..self
            },
        )
    }
}
