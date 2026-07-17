#![deny(missing_docs)]
//! Agent Runtime Protocol wire types.
//!
//! The Agent Runtime Protocol is the outer protocol used to communicate with
//! an agent runtime. It carries Agent Client Protocol (ACP) messages alongside
//! runtime control messages without interpreting the wrapped ACP payloads.
//!
//! The normative wire specification is maintained in `SPEC.md` at the crate
//! root.

/// Role-oriented connections over a logical Agent Runtime Protocol stream.
pub mod connection;
/// Versioned protocol message types.
pub mod schema;
/// Physical transports for logical Agent Runtime Protocol messages.
pub mod transport;

/// Utilities for asserting direction-labelled JSON messages at the wire boundary.
#[cfg(any(test, feature = "test-utils"))]
pub mod testing;
