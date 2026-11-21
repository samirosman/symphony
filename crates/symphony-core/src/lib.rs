//! Symphony core: the scene model, kept free of any UI or GPU code so the
//! front end can change without touching it.
//!
//! - [`xsheet`]: columns of exposed drawings per frame (frames are 1-based).
//! - [`graph`]: the node graph (Drawing, Peg, Composite, Display, Write).
//! - [`anim`]: animatable values (constant or keyframed).
//! - [`math`]: 2D affine transforms.

pub mod anim;
pub mod graph;
pub mod math;
pub mod xsheet;
