use serde::{Deserialize, Serialize};
use std::ops::{Add, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub const fn new(x: f64, y: f64) -> Self {
        Vec2 { x, y }
    }
    pub fn length(self) -> f64 {
        self.x.hypot(self.y)
    }
    pub fn normalized(self) -> Option<Vec2> {
        let l = self.length();
        (l > 0.0).then(|| Vec2::new(self.x / l, self.y / l))
    }
    /// Counter-clockwise rotation by 90°: (x, y) -> (-y, x).
    pub fn perp_ccw(self) -> Vec2 {
        Vec2::new(-self.y, self.x)
    }
    pub fn dot(self, o: Vec2) -> f64 {
        self.x * o.x + self.y * o.y
    }
    pub fn cross(self, o: Vec2) -> f64 {
        self.x * o.y - self.y * o.x
    }
    pub fn to_array(self) -> [f64; 2] {
        [self.x, self.y]
    }
    pub fn distance_to(self, o: Vec2) -> f64 {
        (o - self).length()
    }
    /// Angle of the vector in radians, in `(-pi, pi]` (`atan2` convention).
    pub fn angle(self) -> f64 {
        self.y.atan2(self.x)
    }
    /// Unit vector at `angle` radians from +X, counter-clockwise.
    pub fn from_angle(angle: f64) -> Vec2 {
        Vec2::new(angle.cos(), angle.sin())
    }
    /// Point at parameter `t` on the segment `self -> o` (`t` is not clamped).
    pub fn lerp(self, o: Vec2, t: f64) -> Vec2 {
        Vec2::new(self.x + (o.x - self.x) * t, self.y + (o.y - self.y) * t)
    }
    /// Coincidence test against a vertex tolerance.
    pub fn is_near(self, o: Vec2, tolerance: f64) -> bool {
        self.distance_to(o) <= tolerance
    }
    /// Counter-clockwise rotation by `angle` radians.
    pub fn rotated(self, angle: f64) -> Vec2 {
        let (s, c) = angle.sin_cos();
        Vec2::new(self.x * c - self.y * s, self.x * s + self.y * c)
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }
}
impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}
impl Mul<f64> for Vec2 {
    type Output = Vec2;
    fn mul(self, k: f64) -> Vec2 {
        Vec2::new(self.x * k, self.y * k)
    }
}
impl Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Vec3 { x, y, z }
    }
    pub fn from_xy(p: Vec2, z: f64) -> Self {
        Vec3::new(p.x, p.y, z)
    }
    pub fn to_array(self) -> [f64; 3] {
        [self.x, self.y, self.z]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perp_ccw_is_left_normal() {
        assert_eq!(Vec2::new(1.0, 0.0).perp_ccw(), Vec2::new(0.0, 1.0));
        assert_eq!(Vec2::new(0.0, 1.0).perp_ccw(), Vec2::new(-1.0, 0.0));
    }
}
