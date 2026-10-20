//! Views as elements: state, camera, display, visibility, plan and section graphics, document views, browser, 3D picking.
//!
//! Every module keeps its crate-root path too (`crate::views` is `crate::viewing::views`): see the re-exports in `lib.rs`.

pub mod views;
pub mod view_camera;
pub mod view_display;
pub mod view_dup;
pub mod view_names;
pub mod view_visibility;
pub mod plan_graphics;
pub mod section_graphics;
pub mod docviews;
pub mod browser;
pub mod pick3d;
