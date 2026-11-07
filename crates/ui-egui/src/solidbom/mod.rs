//! SolidBOM's desktop review surface: the BOM workspace, its retained review editor and the
//! original window kept behind the `solidbom_legacy_window` preference.
//!
//! The session's read-only policy is enforced in the engine; `SolveApp::start` also refuses to
//! open a modelling dialog while a session is bound.

pub mod panel;
mod workspace;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
