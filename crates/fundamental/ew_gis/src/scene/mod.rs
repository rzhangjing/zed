//! Scene mode and the renderable scenes.
//!
//! `Plane` is the stage-1 equirectangular 2D map; `Globe` is the stage-2
//! WGS84 ellipsoid with a quadtree LOD mesh (`globe`) built from
//! `globe_mesh` and selected by `math::lod`. `plot` is the stage-3 batched
//! vector overlay (points, polylines, polygon fills, circles, sectors), drawn
//! in both modes.

pub mod globe;
pub mod globe_mesh;
pub mod icons;
pub mod mvt_layer;
pub mod pick;
pub mod plane;
pub mod plot;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneMode {
    Plane,
    Globe,
}

impl SceneMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Plane => "2D Plane",
            Self::Globe => "3D Globe",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Plane => Self::Globe,
            Self::Globe => Self::Plane,
        }
    }
}
