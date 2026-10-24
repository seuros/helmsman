//! Helmsman core library: adaptive instruction templates.
//!
//! `engine`/`models`/`config`/`skills`/`environment` have no CLI-only deps and
//! are always available. `anthropic`/`goals`/`registry`/`remote`/`server`/
//! `tokenizer` need the `cli` feature (on by default for the `helmsman` binary).
pub mod config;
pub mod engine;
pub mod environment;
pub mod models;
pub mod skills;

#[cfg(feature = "cli")]
pub mod anthropic;
#[cfg(feature = "cli")]
pub mod goals;
#[cfg(feature = "cli")]
pub mod registry;
#[cfg(feature = "cli")]
pub mod remote;
#[cfg(feature = "cli")]
pub mod server;
#[cfg(feature = "cli")]
pub mod tokenizer;
