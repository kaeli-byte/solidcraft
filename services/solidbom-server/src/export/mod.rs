//! Internal Excel and PDF exports (package SB-21, additive to API version 2).
//!
//! The service exports one explicitly selected saved result: an accepted baseline (SB-19) or the
//! project's current draft calculation snapshot (SB-16). [`resolve`] builds one
//! snapshot-resolved [`model::ExportModel`] from the stored records the selected result pins, and
//! the two renderers draw it:
//!
//! - [`xlsx`] writes the detailed internal workbook (metadata, BOM, routings, feasibility, cost
//!   details and commercial scenarios);
//! - [`pdf`] writes the internal summary (costs, annual prices/EBIT, tooling/setup charges and the
//!   unresolved issues).
//!
//! Both renderers are pure functions of the model. They do no business arithmetic: the model's
//! values are copied from the stored snapshot output, and the renderers only format, wrap and
//! paginate them. The portable wire vocabulary is `solidbom_contract::exports`.
//!
//! The model and the renderers are service-internal because only the service renders an export; the
//! desktop command receives the rendered bytes and saves them.

pub mod model;
mod pdf;
mod resolve;
mod xlsx;
mod zip;

use model::ExportModel;
use solidbom_contract::ExportFormat;

/// Render one resolved export model in the requested format.
pub fn render(model: &ExportModel, format: ExportFormat) -> Vec<u8> {
    match format {
        ExportFormat::Xlsx => xlsx::render(model),
        ExportFormat::Pdf => pdf::render(model),
    }
}
