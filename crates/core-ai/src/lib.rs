//! The local model, and how not to fool yourself about it.
//!
//! Brief §3.3 and plan §4 agree on the risk, and it is not that a model fails.
//! It is never finding out whether it beat a `HashMap` of sender to folder, and
//! paying its latency for nothing. So this crate is built to be measured before
//! it is trusted:
//!
//! - **Two ways of asking**, because the plan says to spike both before
//!   committing. [`PromptClassifier`] asks a chat model for a word;
//!   [`Neighbours`] embeds a message and takes the nearest of the user's own
//!   past filings, which needs no prompt tuning and improves as they file.
//! - **Nothing here decides what the list shows.** A model verdict is recorded
//!   beside the rules' and compared with it; `core-store` keeps model verdicts
//!   out of the list on purpose, so a model can be wrong for a month without
//!   anyone's mail moving.
//! - **The sender's words are fenced.** A prompt carries mail, and mail is
//!   written by whoever sent it. The worst an injected instruction can do here
//!   is change a verdict nobody is shown — but the fence is still built so that
//!   a subject cannot close it.
//!
//! The model server is Ollama, spoken to over its HTTP API directly. It is a
//! local process on a known port, and a client library for two endpoints would
//! be more code to trust than the endpoints themselves.

mod neighbours;
mod ollama;
mod prompt;

pub use neighbours::{cosine, embedding_input, Hybrid, Nearest, Neighbours};
pub use ollama::{AiError, ChatReply, Embedded, Ollama, TRANSCRIBE};
pub use prompt::{ModelVerdict, PromptClassifier};
