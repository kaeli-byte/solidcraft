//! The read-only policy a session carries while it is bound to a reviewed SolidBOM source.
//!
//! The policy is bound by the trusted project-import path, never by a command. It denies every
//! command that could change the source geometry or hierarchy, and every other document change
//! except renaming a body or component. View, selection, inspection, measurement, those renames,
//! the SolidBOM business commands and local export stay available. A name is not geometry: the
//! reviewed mapping is keyed by occurrence ids and child-index paths, never by names, so it stays
//! the same. The allowlist is a default-deny policy: a command is available only when it is named
//! here or belongs to a read-only family (the registry already refuses unknown ids).
//!
//! A local import also records the reviewed original's path. Save and export commands stay
//! available, but never onto that original, directly or through an equivalent path: a reviewed
//! session may keep its own working copy, not replace the file it was given.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;
use solidbom_contract::{AnalysisKey, SourceRef};

/// The reason a bound session reports for an authoring action that has no single command id (for
/// example reopening a feature editor from the timeline).
pub const READ_ONLY_REASON: &str = "the reviewed source is read-only while it is reviewed";

/// Commands that only read, view, measure, name or export.
const ALLOWED: &[&str] = &[
    "assembly.bom",
    // A name is not geometry or hierarchy: the review mapping never reads it (issue #50).
    "body.rename",
    // Display-only: open/closed browser rows are not saved in the design and are not undo steps.
    "browser.collapse_all",
    "browser.expand_all",
    "component.activate",
    "component.copies",
    "component.links",
    "component.list",
    "component.metadata",
    "component.properties",
    "component.rename",
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
    "select.clear",
    "select.filter",
    "select.set",
    // SolidBOM's business seam: reviewed inputs, saved projects and cost results live outside
    // the source geometry. Import and reopen are deliberately absent: they replace the source.
    "solidbom.calculate",
    "solidbom.calculation",
    "solidbom.inputs",
    "solidbom.inputs.batch",
    "solidbom.parts",
    "solidbom.retry",
    "solidbom.review",
    "solidbom.save",
    // Shared company records live outside the reviewed source; an Administrator may maintain
    // them while a project is open (the service still enforces the permission set).
    "solidbom.shared",
    "solidbom.shared.app_config.version",
    "solidbom.shared.component.version",
    "solidbom.shared.cost_center.version",
    "solidbom.shared.material.version",
    "solidbom.state",
];

/// Families that only change the view or inspect the model. `view.save`, `view.rename` and
/// `view.delete` are not view-only: they change the document's named views (and push an undo
/// step), so they stay denied.
const ALLOWED_PREFIXES: &[&str] = &["view.", "inspect."];

/// Document-changing commands that would otherwise be admitted by a read-only family.
const DENIED: &[&str] = &["view.delete", "view.rename", "view.save"];

/// Allowed commands that write the file their `path` parameter names. They are available in a
/// reviewed session, but never onto the reviewed original.
const PATH_WRITING: &[&str] = &[
    "file.export",
    "file.export_3mf",
    "file.export_obj",
    "file.export_step",
    "file.export_stl",
    "file.export_urdf",
    "file.save_as",
    "file.save_mesh",
];

/// What a reviewed session is bound to, and what it may still run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewPolicy {
    source: SourceRef,
    analysis: AnalysisKey,
    /// The reviewed original on disk, when this session imported it from a local file.
    source_path: Option<PathBuf>,
    allowed: BTreeSet<String>,
}

impl ReviewPolicy {
    pub fn new(source: SourceRef, analysis: AnalysisKey) -> Self {
        Self { source, analysis, source_path: None, allowed: ALLOWED.iter().map(|id| (*id).to_string()).collect() }
    }

    /// Bind the reviewed original's path, so save and export cannot write over it. The import
    /// path passes the file it read; a reopened source has no local original.
    pub fn with_source_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.source_path = Some(path.into());
        self
    }

    pub fn source(&self) -> &SourceRef {
        &self.source
    }

    pub fn analysis(&self) -> &AnalysisKey {
        &self.analysis
    }

    /// The reviewed original on disk, when this session imported one.
    pub fn source_path(&self) -> Option<&Path> {
        self.source_path.as_deref()
    }

    /// Whether `id` may run in this session. Panel/layout commands are allowed: they change the
    /// application layout, not the design.
    pub fn allows(&self, id: &str) -> bool {
        if DENIED.contains(&id) {
            return false;
        }
        self.allowed.contains(id) || ALLOWED_PREFIXES.iter().any(|prefix| id.starts_with(prefix)) || crate::panels::commands::is_command(id)
    }

    /// Why `id` with `params` may not run; `None` when it may (or the session is unbound).
    pub fn refusal_for(&self, id: &str, params: &Value) -> Option<String> {
        if !self.allows(id) {
            return Some(self.refusal(id));
        }
        if PATH_WRITING.contains(&id) && self.writes_source(params) {
            return Some(self.source_refusal(id));
        }
        None
    }

    /// The message a refused command reports.
    pub fn refusal(&self, id: &str) -> String {
        format!("`{id}` is not allowed: the imported source is read-only while it is reviewed")
    }

    /// The message a save or export onto the reviewed original reports.
    pub fn source_refusal(&self, id: &str) -> String {
        format!("`{id}` would overwrite the reviewed source")
    }

    /// Whether the command's `path` parameter names the reviewed original, directly or through
    /// an equivalent path (relative, `..`, a link, a hard link or another case of the same name).
    fn writes_source(&self, params: &Value) -> bool {
        let Some(source) = self.source_path.as_deref() else { return false };
        let Some(path) = params.get("path").and_then(Value::as_str) else { return false };
        same_file(Path::new(path), source)
    }
}

/// Whether two paths name the same file, without requiring the written path to exist yet.
fn same_file(a: &Path, b: &Path) -> bool {
    // Two existing paths: compare what they resolve to. This also covers a target that differs
    // only in case on a case-insensitive file system, and a link pointing at the original.
    if let (Ok(canonical_a), Ok(canonical_b)) = (a.canonicalize(), b.canonicalize())
        && canonical_a == canonical_b
    {
        return true;
    }
    // Hard links keep different names but one file.
    if let (Ok(meta_a), Ok(meta_b)) = (std::fs::metadata(a), std::fs::metadata(b)) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta_a.dev() == meta_b.dev() && meta_a.ino() == meta_b.ino() {
                return true;
            }
        }
        #[cfg(not(unix))]
        let _ = (meta_a, meta_b);
    }
    // The target does not exist yet, or resolving it failed: compare the paths as written, made
    // absolute and with `.` and `..` resolved lexically. A new file cannot overwrite the
    // original, but `dir/../original.step` must not look different from `original.step`.
    match (lexical(a), lexical(b)) {
        (Some(a), Some(b)) => lexically_same(&a, &b),
        _ => false,
    }
}

/// Whether normalized paths name the same spelling. A case-insensitive file system can resolve
/// two spellings to one file even when that name does not exist yet (a deleted source, a
/// re-created export target), so those hosts compare the spelling without case.
fn lexically_same(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    #[cfg(any(target_os = "macos", windows))]
    {
        a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy())
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        false
    }
}

/// `path` made absolute and lexically normalized, resolving `.` and `..` without touching disk.
fn lexical(path: &Path) -> Option<PathBuf> {
    let absolute = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().ok()?.join(path) };
    let mut out = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    Some(out)
}
