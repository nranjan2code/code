//! Transport adapter for the greenfield Runtime API.
//!
//! This crate owns no state and never writes the filesystem.  Surfaces use
//! [`Client`] to submit Runtime commands and consume the resulting events.

pub mod cli;
mod client;
pub mod types;

pub use client::{Client, ClientBuilder, ClientError, EventStream, Result};
pub use types::*;
