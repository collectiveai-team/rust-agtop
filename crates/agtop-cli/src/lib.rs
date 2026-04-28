//! Library facade for integration tests and external consumers.
//! The binary entry point is `main.rs`.
//!
//! This lib target exposes the `tui` module for integration tests.

// Suppress dead-code warnings for items that are only used from the binary.
#![allow(dead_code)]

// These modules are referenced by tui/ internally.
pub mod fmt;
pub mod tui;
pub mod version;
