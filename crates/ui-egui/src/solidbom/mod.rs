//! SolidBOM's desktop review surface: the BOM workspace, its retained review editor and the
//! original window kept behind the `solidbom_legacy_window` preference.
//!
//! The session's read-only policy is enforced in the engine. The window refuses every
//! interactive start, editor restoration and control entry before anything opens, and cancels
//! authoring state when the policy is bound or replaced.

pub mod panel;
mod workspace;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
