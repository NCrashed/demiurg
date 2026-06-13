//! Orbit camera for the model viewport.
//!
//! roxlap's [`Camera`] is a position plus an orthonormal `right/down/
//! forward` basis in the voxlap z-down world. We expose an orbit around
//! a fixed look-at point (the model centre) parameterised by yaw, pitch,
//! and distance, and convert to that basis with the exact yaw/pitch
//! formula the roxlap sprite oracle uses — so the basis is guaranteed
//! consistent with the projection the renderer applies.
//!
//! Ported from `monada-render`'s `OrbitCamera` (the M1 top-down camera),
//! generalised so the framing distance is chosen per model size.

use glam::DVec3;
use roxlap_core::Camera;

/// Orbit camera: looks at `center` from `dist` away, at `yaw`/`pitch`.
#[derive(Clone, Copy, Debug)]
pub struct OrbitCamera {
    pub center: DVec3,
    /// Rotation about the world z axis (radians).
    pub yaw: f64,
    /// Tilt below the horizon (radians); `pi/2` looks straight down.
    pub pitch: f64,
    /// Eye distance from `center`, in world voxels.
    pub dist: f64,
}

impl OrbitCamera {
    // Pitch spans nearly straight-up to nearly straight-down so the
    // camera can drop below the model to edit its underside. The basis
    // in `to_roxlap` stays orthonormal across this range (no gimbal
    // lock), so only the poles are excluded.
    const PITCH_MIN: f64 = -1.5;
    const PITCH_MAX: f64 = 1.5;
    const DIST_MIN: f64 = 8.0;
    const DIST_MAX: f64 = 4000.0;

    /// A three-quarter view framing a model: looking "north-and-down"
    /// from `dist` away. Callers pick `dist` from the model's size (see
    /// [`crate::ModelView::framing_camera`]).
    #[must_use]
    pub fn framing(center: DVec3, dist: f64) -> OrbitCamera {
        OrbitCamera {
            center,
            yaw: 0.0,
            pitch: 0.9,
            dist: dist.clamp(Self::DIST_MIN, Self::DIST_MAX),
        }
    }

    /// Nudge the orbit; pitch and distance are clamped to sane ranges.
    pub fn orbit(&mut self, dyaw: f64, dpitch: f64, ddist: f64) {
        self.yaw += dyaw;
        self.pitch = (self.pitch + dpitch).clamp(Self::PITCH_MIN, Self::PITCH_MAX);
        self.dist = (self.dist + ddist).clamp(Self::DIST_MIN, Self::DIST_MAX);
    }

    /// Convert to roxlap's `pos` + `right/down/forward` basis.
    ///
    /// `forward` is the view direction; the eye sits `dist` *behind* the
    /// look-at along it. The basis is **right-handed** (`right × down =
    /// forward`), matching voxlap's `setcamera`: the sprite frustum cull
    /// derives its inward edge normals from the corner winding, so a
    /// left-handed basis makes the cull reject every sprite. At yaw =
    /// pitch = 0 this yields `forward = +x`, `right = +y`, `down = +z` —
    /// the oracle pose.
    #[must_use]
    pub fn to_roxlap(&self) -> Camera {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();

        let forward = [cy * cp, sy * cp, sp];
        let right = [-sy, cy, 0.0];
        let down = [-sp * cy, -sp * sy, cp];

        let fwd = DVec3::from_array(forward);
        let eye = self.center - fwd * self.dist;

        Camera {
            pos: eye.to_array(),
            right,
            down,
            forward,
        }
    }
}
