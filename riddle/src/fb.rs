//! Geometry helpers. Drawing lives in surface.rs.

#[cfg(not(feature = "rm2"))]
pub const SCREEN_W: usize = 1620;
#[cfg(not(feature = "rm2"))]
pub const SCREEN_H: usize = 2160;

#[cfg(feature = "rm2")]
pub const SCREEN_W: usize = 1404;
#[cfg(feature = "rm2")]
pub const SCREEN_H: usize = 1872;

/// Grow-only pixel bounding box, used to build update/dissolve regions.
#[derive(Clone, Copy, Debug)]
pub struct BBox {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl BBox {
    pub fn empty() -> Self {
        Self { x0: i32::MAX, y0: i32::MAX, x1: i32::MIN, y1: i32::MIN }
    }
    pub fn is_empty(&self) -> bool {
        self.x0 > self.x1
    }
    pub fn add(&mut self, x: i32, y: i32, margin: i32) {
        self.x0 = self.x0.min(x - margin).max(0);
        self.y0 = self.y0.min(y - margin).max(0);
        self.x1 = self.x1.max(x + margin).min(SCREEN_W as i32 - 1);
        self.y1 = self.y1.max(y + margin).min(SCREEN_H as i32 - 1);
    }
    pub fn rect(&self) -> (i32, i32, i32, i32) {
        (self.x0, self.y0, self.x1 - self.x0 + 1, self.y1 - self.y0 + 1)
    }
}
