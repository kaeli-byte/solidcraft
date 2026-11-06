//! SolidBOM's desktop review surface.
//!
//! The session's read-only policy is enforced in the engine; `SolveApp::start` also refuses to
//! open a modelling dialog while a session is bound. The panel and controller arrive in later
//! tasks.

pub mod panel;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
