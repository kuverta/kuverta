//! Outgoing mail: composing RFC 5322 messages and submitting them over SMTP.
//!
//! This is stage 1 of the write capability described in
//! docs/implementation-plan.md section 1a. It is deliberately additive: nothing
//! here mutates a mailbox. Submission talks to the provider's SMTP endpoint,
//! and the copy filed in Sent goes via IMAP `APPEND`, which creates a message
//! rather than changing one. The read-only `EXAMINE` property of `core-proto`
//! survives untouched.
//!
//! The two halves are separate on purpose: [`compose`] is pure and synchronous,
//! so every rule about quoting, threading and header safety is testable without
//! a server, and [`submit`] does nothing but move bytes.

pub mod compose;
pub mod submit;

pub use compose::{BuiltMessage, ComposeError, Draft, Mailbox, ReplyMode, ReplySource};
pub use submit::{submit, SubmitError};
