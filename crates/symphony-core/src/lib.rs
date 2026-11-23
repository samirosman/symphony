//! Symphony core: the document model, kept free of UI, GPU and file IO so
//! it can be tested and scripted headless (ENGINEERING E1.2).
//!
//! - [`id`]: stable IDs (E2.2).
//! - [`doc`]: the scene document: layers, xsheet, graph, drawings, palette.
//! - [`history`]: undoable edits (E3).
//! - [`xsheet`], [`graph`], [`stroke`], [`palette`], [`anim`], [`math`]: parts.
//! - [`sound`]: sound tracks that play with the picture.
//! - [`bitmap`]: imported images kept as the original file.
//! - [`deform`]: bone chains that bend drawings at their joints.

pub mod anim;
pub mod b64;
pub mod bitmap;
pub mod deform;
pub mod doc;
pub mod graph;
pub mod history;
pub mod id;
pub mod math;
pub mod palette;
pub mod sound;
pub mod stroke;
pub mod xsheet;
