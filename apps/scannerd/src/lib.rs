//! The capture half of the paper pipeline, as a library.
//!
//! Split from the binary so the parts worth testing can be tested: the page
//! detector against synthetic frames, the spool against a real directory, and
//! the uploader against a server on loopback. None of that needs a camera, and
//! a daemon whose logic can only be exercised on the device it runs on is one
//! that gets changed by guessing.

pub mod button;
pub mod camera;
pub mod detect;
pub mod display;
#[cfg(target_os = "linux")]
pub mod epaper;
pub mod folders;
pub mod hub;
pub mod lcd;
pub mod locate;
pub mod paint;
pub mod pdf;
pub mod picture;
pub mod quality;
pub mod read;
pub mod run;
pub mod settings;
pub mod spool;
pub mod straighten;
pub mod touch;
pub mod upload;
pub mod web;
