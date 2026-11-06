//! The read-only policy a session carries while it is bound to a reviewed SolidBOM source.
//!
//! The policy is bound by the trusted project-import path, never by a command. It denies every
//! command that could change the source geometry, hierarchy or metadata; view, selection,
//! inspection, measurement and local export stay available. The check is exact-id or a named
//! read-only family; unknown commands are already refused by the registry.

use std::collections::BTreeSet;

use solidbom_contract::{AnalysisKey, SourceRef};

/// Commands that only read, view, measure or export.
const ALLOWED: &[&str] = &[
    "assembly.bom",
    "component.activate",
    "component.copies",
    "component.hide_history",
    "component.links",
    "component.list",
    "component.metadata",
    "component.properties",
    "component.show_history",
    "document.inspect",
    "document.metadata",
    "document.revision",
    "engine.commands",
    "engine.describe",
    "file.export",
    "file.export_3mf",
    "file.export_obj",
    "file.export_step",
    "file.export_stl",
    "file.export_urdf",
    "file.save_as",
    "file.save_mesh",
    "file.slicers",
    "model.edges",
    "model.faces",
    "model.threads",
    "occurrence.properties",
];

/// Families that only change the view or inspect the model.
const ALLOWED_PREFIXES: &[&str] = &["view.", "inspect."];

/// What a reviewed session is bound to, and what it may still run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewPolicy {
    source: SourceRef,
    analysis: AnalysisKey,
    allowed: BTreeSet<String>,
}

impl ReviewPolicy {
    pub fn new(source: SourceRef, analysis: AnalysisKey) -> Self {
        Self { source, analysis, allowed: ALLOWED.iter().map(|id| (*id).to_string()).collect() }
    }

    pub fn source(&self) -> &SourceRef {
        &self.source
    }

    pub fn analysis(&self) -> &AnalysisKey {
        &self.analysis
    }

    /// Whether `id` may run in this session. Panel/layout commands are allowed: they change the
    /// application layout, not the design.
    pub fn allows(&self, id: &str) -> bool {
        self.allowed.contains(id) || ALLOWED_PREFIXES.iter().any(|prefix| id.starts_with(prefix)) || crate::panels::commands::is_command(id)
    }

    /// The message a refused command reports.
    pub fn refusal(&self, id: &str) -> String {
        format!("`{id}` is not allowed: the imported source is read-only while it is reviewed")
    }
}
