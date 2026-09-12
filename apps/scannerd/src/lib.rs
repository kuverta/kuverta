//! The capture half of the paper pipeline, as a library.
//!
//! Split from the binary so the parts worth testing can be tested: the page
//! detector against synthetic frames, the spool against a real directory, and
//! the uploader against a server on loopback. None of that needs a camera, and
//! a daemon whose logic can only be exercised on the device it runs on is one
//! that gets changed by guessing.

pub mod camera;
pub mod detect;
pub mod spool;
pub mod upload;
