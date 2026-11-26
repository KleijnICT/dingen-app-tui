//! Storage, sync and logic for native Dingen clients.
//!
//! Built against the Dingen specification (`spec/` in the main repository):
//! the sync protocol, version 4, and the conformance test set. Free of any
//! user interface code.

pub mod actions;
pub mod api;
pub mod dates;
pub mod entities;
pub mod hlc;
pub mod ids;
pub mod misc;
pub mod model;
pub mod order;
pub mod record;
pub mod repeat;
pub mod replica;
pub mod rest;
pub mod review;
pub mod search;
pub mod tag_tree;
pub mod today;
pub mod views;

/// The sync protocol this client speaks.
pub const SYNC_PROTOCOL: u32 = 4;

/// The version of the conformance test set this crate passes
/// (`spec/conformance/manifest.json`).
pub const CONFORMANCE_VERSION: u32 = 2;
