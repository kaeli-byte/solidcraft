//! The SolidBOM review window: engineering review, manufacturing inputs and cost explanation.
//!
//! The panel is thin: it reads the session's review state and acts through the SolidBOM commands.
//! It never talks to the service and never does cost arithmetic.

use std::collections::BTreeMap;

use egui::RichText;
use serde_json::{Value, json};
use solidbom_contract::{CalculationSnapshot, DraftProject, Issue, Sourcing, Unit};
use solidcraft_engine::solidbom::state::{Phase, ResultState, SaveState, State};
use uuid::Uuid;

use crate::SolveApp;
use crate::theme::Tokens;
use crate::ui_kit;

#[derive(Debug, Clone)]
pub struct Panel {
    pub(super) part: Option<Uuid>,
    fields: Fields,
    baseline: Fields,
    /// The loaded part, the reviewed-input generation and the reference data it resolved its
    /// material and rate choices against. New reference data reloads the part unless it has edits.
    loaded: Option<(Uuid, u64, (usize, usize))>,
    drafts: BTreeMap<Uuid, (Fields, Fields, u64)>,
    pub(super) error: Option<String>,
    pub(super) query: String,
    pub(super) filter: String,
    pub(super) sort: usize,
    pub(super) descending: bool,
    pub(super) section: String,
    pub(super) table: bool,
    pub(super) inspector: bool,
    pub(super) table_fraction: f32,
    pub(super) layout_generation: u64,
    pub(super) reveal_part: bool,
    focus_cavities: bool,
    overhead: String,
    overhead_baseline: String,
    pub(super) project_id: Option<String>,
    observed_selection: Vec<solidcraft_engine::Sel>,
    pub(super) action: Option<String>,
    pub(super) projection: Option<(String, Vec<Value>)>,
    /// The calculation whose explanations the cost section shows; `None` shows the current one.
    pub(super) history: Option<Uuid>,
    window_open: bool,
}

impl Default for Panel {
    fn default() -> Self {
        Self {
            part: None,
            fields: Fields::default(),
            baseline: Fields::default(),
            loaded: None,
            drafts: BTreeMap::new(),
            error: None,
            query: String::new(),
            filter: "all".into(),
            sort: 0,
            descending: false,
            section: "engineering".into(),
            table: true,
            inspector: true,
            table_fraction: 0.35,
            layout_generation: 0,
            reveal_part: false,
            focus_cavities: false,
            overhead: String::new(),
            overhead_baseline: String::new(),
            project_id: None,
            observed_selection: Vec::new(),
            action: None,
            projection: None,
            history: None,
            window_open: true,
        }
    }
}

impl Panel {
    pub fn has_pending(&self) -> bool {
        self.fields != self.baseline
            || self.overhead != self.overhead_baseline
            || self.drafts.values().any(|(fields, baseline, _)| fields != baseline)
    }

    fn stash(&mut self) {
        if let Some((part, generation, _)) = self.loaded {
            self.drafts.insert(part, (self.fields.clone(), self.baseline.clone(), generation));
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
struct Fields {
    classification: String,
    confirmed: bool,
    manufactured: bool,
    volume: String,
    manual_reason: String,
    volume_confirmed: bool,
    material: usize,
    allowance: String,
    cycle: String,
    cavities: String,
    production_rate: usize,
    setup_hours: String,
    setup_rate: usize,
}

/// The selected part's reviewed identity: one definition shared by its occurrences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PartContext {
    pub part_number: Option<String>,
    pub revision: Option<String>,
    pub definition_id: String,
    pub occurrence_ids: Vec<String>,
}

impl PartContext {
    /// The expanded quantity: the occurrences that share this one reviewed record.
    pub(super) fn quantity(&self) -> usize {
        self.occurrence_ids.len()
    }
}

/// One row of the calculation history the cost section lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct HistoryEntry {
    pub calculation_id: Uuid,
    pub saved_version: u64,
    /// Whether its full snapshot is in memory, so its explanations and own inputs can be shown.
    pub loaded: bool,
}

/// A backend decimal amount shown with two places, rounded from the exact value. A missing value
/// stays `None`: it is never shown as zero.
pub(super) fn amount_two_places(value: Option<&str>) -> Option<String> {
    value.map(str::trim).filter(|value| !value.is_empty()).and_then(round_two_places)
}

/// An amount ready for a label; a missing one reads "Unavailable".
pub(super) fn amount_text(value: Option<&str>) -> String {
    amount_two_places(value).unwrap_or_else(|| crate::i18n::tr("solidbom.workspace.unavailable").to_string())
}

/// Like [`amount_text`], for the contract's exact decimal values.
pub(super) fn amount_text_of(value: Option<impl std::fmt::Display>) -> String {
    let text = value.map(|value| value.to_string());
    amount_text(text.as_deref())
}

/// Round a plain decimal string to two places, half up. The rounding happens once, on the exact
/// backend value, so an assembly total is never built from an already-displayed per-part amount.
fn round_two_places(text: &str) -> Option<String> {
    let (negative, rest) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let (integer, fraction) = match rest.split_once('.') {
        Some((integer, fraction)) => (integer, fraction),
        None => (rest, ""),
    };
    if integer.is_empty() && fraction.is_empty() {
        return None;
    }
    if !integer.bytes().all(|byte| byte.is_ascii_digit()) || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let digit = |index: usize| fraction.as_bytes().get(index).map_or(0_u32, |byte| u32::from(byte - b'0'));
    let mut kept = digit(0) * 10 + digit(1);
    let mut integer_digits: Vec<u8> = integer.bytes().collect();
    if integer_digits.is_empty() {
        integer_digits.push(b'0');
    }
    if digit(2) >= 5 {
        kept += 1;
        if kept == 100 {
            kept = 0;
            let mut carry = 1_u8;
            for index in (0..integer_digits.len()).rev() {
                let sum = integer_digits[index] - b'0' + carry;
                integer_digits[index] = b'0' + sum % 10;
                carry = sum / 10;
                if carry == 0 {
                    break;
                }
            }
            if carry == 1 {
                integer_digits.insert(0, b'1');
            }
        }
    }
    let first = integer_digits.iter().position(|digit| *digit != b'0').unwrap_or(integer_digits.len() - 1);
    let integer_text: String = integer_digits[first..].iter().map(|digit| char::from(*digit)).collect();
    let zero = integer_text == "0" && kept == 0;
    let sign = if negative && !zero { "-" } else { "" };
    Some(format!("{sign}{integer_text}.{kept:02}"))
}

/// The reviewed definition and occurrences behind one selected part, or `None` when the part is
/// no longer in the draft.
pub(super) fn part_context(draft: &DraftProject, part_version_id: Uuid) -> Option<PartContext> {
    let part = draft.parts.iter().find(|part| part.part_version_id == part_version_id)?;
    let definition = draft.analysis.definitions.iter().find(|definition| definition.definition_id == part.definition_id);
    let occurrence_ids = draft
        .analysis
        .occurrences
        .iter()
        .filter(|occurrence| occurrence.definition_id == part.definition_id)
        .map(|occurrence| occurrence.occurrence_id.to_string())
        .collect();
    Some(PartContext {
        part_number: definition.and_then(|definition| definition.customer_part_number.clone()),
        revision: definition.and_then(|definition| definition.customer_revision.clone()),
        definition_id: part.definition_id.to_string(),
        occurrence_ids,
    })
}

/// Source/analysis issues as review-required lines. A named issue is a reason to review, not a
/// value to display as zero.
pub(super) fn analysis_issue_lines(draft: &DraftProject) -> Vec<String> {
    draft.analysis.issues.iter().filter(|issue| !resolved_by_manual_volume(draft, issue)).map(issue_line).collect()
}

/// An extraction issue that asked for a volume is resolved once the engineer entered, confirmed a
/// manual volume for that definition: the source fact stays recorded, the review requirement goes.
fn resolved_by_manual_volume(draft: &DraftProject, issue: &Issue) -> bool {
    let Some(definition_id) = issue.definition_id else { return false };
    if !matches!(issue.code.as_str(), "missing_volume" | "open_body" | "missing_body" | "invalid_volume" | "measure_failed" | "volume_overflow") {
        return false;
    }
    draft.parts.iter().any(|part| {
        part.definition_id == definition_id
            && part.volume_mm3.source == solidbom_contract::MeasurementSource::Manual
            && part.volume_mm3.review == solidbom_contract::ReviewState::Confirmed
            && part.volume_mm3.value.is_some()
    })
}

fn issue_line(issue: &Issue) -> String {
    let text: &str = match issue.code.as_str() {
        "unresolved_grouping" => crate::i18n::tr("solidbom.issue.unresolved_grouping"),
        "open_body" => crate::i18n::tr("solidbom.issue.open_body"),
        "missing_volume" => crate::i18n::tr("solidbom.issue.missing_volume"),
        "missing_body" => crate::i18n::tr("solidbom.issue.missing_body"),
        "invalid_volume" => crate::i18n::tr("solidbom.issue.invalid_volume"),
        "measure_failed" => crate::i18n::tr("solidbom.issue.measure_failed"),
        "volume_overflow" => crate::i18n::tr("solidbom.issue.volume_overflow"),
        _ => issue.message.as_str(),
    };
    format!("{text} ({})", issue.code)
}

/// The message of a failed project save, if the project failed to save.
pub(super) fn save_failure_message(save: &SaveState) -> Option<&str> {
    match save {
        SaveState::SaveFailed { message, .. } => Some(message),
        _ => None,
    }
}

/// Every calculation the app knows about: this session's snapshots and the summaries a reopen
/// returned, newest saved version first.
pub(super) fn history_entries(state: &State) -> Vec<HistoryEntry> {
    let mut entries: BTreeMap<Uuid, HistoryEntry> = BTreeMap::new();
    for summary in &state.summaries {
        entries.insert(
            summary.calculation_id,
            HistoryEntry { calculation_id: summary.calculation_id, saved_version: summary.saved_version, loaded: false },
        );
    }
    for snapshot in &state.calculations {
        entries.insert(
            snapshot.calculation_id,
            HistoryEntry { calculation_id: snapshot.calculation_id, saved_version: snapshot.saved_version, loaded: true },
        );
    }
    let mut rows: Vec<HistoryEntry> = entries.into_values().collect();
    rows.sort_by(|left, right| right.saved_version.cmp(&left.saved_version).then_with(|| left.calculation_id.cmp(&right.calculation_id)));
    rows
}

/// The full snapshot the cost section shows: the one the person selected, or the current one.
pub(super) fn selected_snapshot(state: &State, history: Option<Uuid>) -> Option<&CalculationSnapshot> {
    match history {
        Some(id) => state.calculations.iter().find(|snapshot| snapshot.calculation_id == id),
        None => state.current.as_ref(),
    }
}

/// A saved result the app only has as a compact summary: it can be listed, but not explained.
pub(super) fn selected_summary(state: &State, history: Option<Uuid>) -> Option<&solidbom_contract::CalculationSummary> {
    let id = history?;
    if state.calculations.iter().any(|snapshot| snapshot.calculation_id == id) {
        return None;
    }
    state.summaries.iter().find(|summary| summary.calculation_id == id)
}

/// Whether `snapshot` is the result the current review state points at.
pub(super) fn is_current_snapshot(state: &State, snapshot: &CalculationSnapshot) -> bool {
    state.current.as_ref().map(|current| current.calculation_id) == Some(snapshot.calculation_id)
}

/// The one-line state of a shown calculation: current, stale, incomplete or an earlier result.
pub(super) fn snapshot_state_line(state: &State, snapshot: &CalculationSnapshot) -> String {
    let version = snapshot.saved_version.to_string();
    let id = if !is_current_snapshot(state, snapshot) {
        "solidbom.workspace.history_earlier"
    } else {
        match state.result {
            ResultState::Stale => "solidbom.workspace.history_stale",
            ResultState::Incomplete => "solidbom.workspace.history_incomplete",
            _ => "solidbom.workspace.history_current",
        }
    };
    crate::i18n::format(id, &[("version", &version)])
}

/// A contract value as text, or `unavailable` when it is missing.
fn display_value(value: Option<impl std::fmt::Display>, unavailable: &str) -> String {
    value.map(|value| value.to_string()).unwrap_or_else(|| unavailable.to_string())
}

/// The exact backend decimal as text, never rounded for display.
pub(super) fn exact_text_of(value: Option<impl std::fmt::Display>) -> String {
    display_value(value, crate::i18n::tr("solidbom.workspace.unavailable"))
}

/// The exact inputs the shown snapshot was calculated from. A reopened summary without its full
/// snapshot has none here: the UI never invents them from the current draft.
pub(super) fn snapshot_inputs_line(snapshot: &CalculationSnapshot, part_version_id: Uuid) -> Option<String> {
    let part = snapshot.inputs.parts.iter().find(|part| part.part_version_id == part_version_id)?;
    let unavailable = crate::i18n::tr("solidbom.workspace.unavailable");
    let quantity = part.quantity.to_string();
    let volume = display_value(part.volume_mm3, unavailable);
    let density = display_value(part.density_g_cm3, unavailable);
    let material_price = display_value(part.material_price_per_kg, unavailable);
    let allowance = display_value(part.allowance_rate, unavailable);
    let cycle = display_value(part.cycle_seconds, unavailable);
    let cavities = display_value(part.cavities, unavailable);
    let production = display_value(part.production_rate_per_hour, unavailable);
    let setup = display_value(part.setup_hours, unavailable);
    let setup_rate = display_value(part.setup_rate_per_hour, unavailable);
    let overhead = display_value(snapshot.inputs.material_overhead_rate, unavailable);
    let currency = snapshot.inputs.currency.as_str();
    Some(crate::i18n::format(
        "solidbom.workspace.history_inputs",
        &[
            ("quantity", &quantity),
            ("volume", &volume),
            ("density", &density),
            ("material_price", &material_price),
            ("allowance", &allowance),
            ("cycle", &cycle),
            ("cavities", &cavities),
            ("production", &production),
            ("setup", &setup),
            ("setup_rate", &setup_rate),
            ("overhead", &overhead),
            ("currency", currency),
        ],
    ))
}

/// One command button that follows the command's real availability, as the toolbar does.
fn command_button(app: &SolveApp, ui: &mut egui::Ui, id: &str, label: &str) -> bool {
    let Some(command) = solidcraft_engine::find_command(id) else { return false };
    let info = crate::toolbar::interactive_command_info(app, command);
    let response = ui_kit::button(ui, label, ui_kit::Button::Enabled(info.enabled));
    let response = if info.enabled {
        response
    } else {
        match info.disabled_reason {
            Some(reason) => response.on_disabled_hover_text(reason),
            None => response,
        }
    };
    response.clicked()
}

/// Open the window.
pub fn open(app: &mut SolveApp) {
    app.solidbom.get_or_insert_with(Panel::default).window_open = true;
    if !app.preferences.solidbom_legacy_window
        && let Err(error) = crate::workspace::enter(app, crate::workspace::WorkspaceKind::Bom)
    {
        app.set_status(error, true);
    }
}

/// Draw the window; it stays open until the user closes it.
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    super::workspace::project_dialog(app, ctx);
    if !app.preferences.solidbom_legacy_window {
        return;
    }
    let Some(mut panel) = app.solidbom.take() else { return };
    if !panel.window_open {
        app.solidbom = Some(panel);
        return;
    }
    let title = crate::i18n::tr("solidbom.title").to_string();
    let closed = ui_kit::Window::new(egui::Id::unique("sc_solidbom"), "bom", &title).cards(3.0).show_body(ctx, |ui| {
        draw(app, &mut panel, ui);
    });
    let action = panel.action.take();
    app.solidbom = Some(panel);
    if let Some(action) = action {
        app.start(&action);
    }
    if closed && let Some(panel) = app.solidbom.as_mut() {
        panel.window_open = false;
    }
}

pub(super) fn draw(app: &mut SolveApp, panel: &mut Panel, ui: &mut egui::Ui) {
    let _language = crate::i18n::scope(&app.preferences.language);
    sync_selection(app, panel);

    let (phase, save, result, source, project, version, definitions, occurrences, issues, source_issues, needs_service, error, notice) = {
        let state = &app.session.solidbom;
        (
            state.phase,
            state.save.clone(),
            state.result.clone(),
            state.draft.as_ref().map(|draft| draft.source.original_name.clone()),
            state.draft.as_ref().map(|draft| draft.project_id.to_string()),
            state.version(),
            state.draft.as_ref().map_or(0, |draft| draft.analysis.definitions.len()),
            state.draft.as_ref().map_or(0, |draft| draft.analysis.occurrences.len()),
            state.issue_codes(),
            state.draft.as_ref().map(analysis_issue_lines).unwrap_or_default(),
            state.needs_service,
            state.error.clone(),
            state.notice.clone(),
        )
    };

    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(source.as_deref().unwrap_or("SolidBOM")).strong());
        ui.label(format!(
            "{} · {} · {} · {} · {}",
            phase_label(phase),
            save_label(&save),
            result_label(&result),
            format_args!("{definitions} {}", crate::i18n::tr("solidbom.definitions")),
            format_args!("{occurrences} {}", crate::i18n::tr("solidbom.occurrences"))
        ));
        if let Some(project) = &project {
            ui.label(format!("{project} · v{version}"));
        }
    });
    if panel.has_pending() {
        ui_kit::status_line(ui, crate::i18n::tr("solidbom.workspace.pending_edits"), ui_kit::StatusTone::Warning);
    }
    if needs_service {
        ui_kit::status_line(ui, crate::i18n::tr("solidbom.no_service"), ui_kit::StatusTone::Warning);
    }
    let save_failure = save_failure_message(&save);
    if let Some(message) = save_failure {
        ui_kit::status_line(ui, &crate::i18n::format("solidbom.workspace.save_failed_line", &[("message", message)]), ui_kit::StatusTone::Error);
    }
    if let Some(error) = &error {
        ui_kit::status_line(ui, error, ui_kit::StatusTone::Error);
    }
    if let Some(notice) = &notice
        && save_failure.is_none()
    {
        ui_kit::status_line(ui, notice, ui_kit::StatusTone::Dim);
    }
    if !issues.is_empty() {
        ui.label(RichText::new(format!("{}: {}", crate::i18n::tr("solidbom.issues"), issues.join(", "))).color(Tokens::get().warning));
    }
    // A source or analysis finding is a review requirement: it is shown before any cost so an
    // unresolved grouping or an unmeasured surface is never taken for a complete review.
    if !source_issues.is_empty() {
        ui.label(RichText::new(crate::i18n::tr("solidbom.workspace.review_required")).strong().color(Tokens::get().warning));
        for issue in &source_issues {
            ui.label(RichText::new(issue).color(Tokens::get().warning));
        }
    }

    let Some(part_id) = panel.part else {
        ui_kit::status_line(ui, crate::i18n::tr("solidbom.select_part"), ui_kit::StatusTone::Dim);
        return;
    };
    let generation = app.session.solidbom.generation;
    let reference = reference_fingerprint(app);
    if panel.loaded != Some((part_id, generation, reference)) && panel.fields == panel.baseline {
        load(app, panel, part_id);
    }
    let Some(draft) = app.session.solidbom.draft.clone() else { return };
    let Some(part) = draft.parts.iter().find(|part| part.part_version_id == part_id) else { return };

    // The reviewed identity behind the selected part: one shared definition with its expanded
    // occurrences, not the runtime row that happened to be clicked.
    if let Some(context) = part_context(&draft, part_id) {
        if let Some(part_number) = &context.part_number {
            let identity = match &context.revision {
                Some(revision) => crate::i18n::format("solidbom.workspace.part_identity", &[("part", part_number), ("revision", revision)]),
                None => crate::i18n::format("solidbom.workspace.part_identity_no_revision", &[("part", part_number)]),
            };
            ui.label(RichText::new(identity).strong());
        }
        let quantity = context.quantity().to_string();
        ui.label(RichText::new(crate::i18n::format("solidbom.workspace.quantity_basis", &[("quantity", &quantity)])).strong());
        ui.label(crate::i18n::tr("solidbom.workspace.definition"));
        ui.label(&context.definition_id);
        if !context.occurrence_ids.is_empty() {
            ui.label(crate::i18n::format("solidbom.workspace.occurrences_identity", &[("occurrences", &context.occurrence_ids.join(", "))]));
        }
    }

    // Engineering review.
    let engineering = crate::i18n::tr("solidbom.engineering").to_string();

    ui_kit::section(
        ui,
        ("solidbom_engineering", panel.section.clone()),
        &engineering,
        matches!(panel.section.as_str(), "engineering" | "volume"),
        |ui| {
            let mut confirmed = panel.fields.confirmed;
            ui_kit::row(ui, crate::i18n::tr("solidbom.confirmed"), |ui| {
                ui_kit::checkbox(ui, "", &mut confirmed);
            });
            panel.fields.confirmed = confirmed;
            ui_kit::expr_row(ui, crate::i18n::tr("solidbom.classification"), &mut panel.fields.classification);
            let mut sourcing = usize::from(!panel.fields.manufactured);
            ui_kit::choice_row(
                ui,
                crate::i18n::tr("solidbom.manufactured"),
                "solidbom_sourcing",
                &mut sourcing,
                &[crate::i18n::tr("solidbom.manufactured"), crate::i18n::tr("solidbom.purchased")],
            );
            panel.fields.manufactured = sourcing == 0;
            ui_kit::expr_row(ui, crate::i18n::tr("solidbom.volume"), &mut panel.fields.volume);
            ui.label(format!(
                "{} · {} · {}",
                match part.volume_mm3.source {
                    solidbom_contract::MeasurementSource::Extracted => crate::i18n::tr("solidbom.measured"),
                    solidbom_contract::MeasurementSource::Manual => crate::i18n::tr("solidbom.manual"),
                },
                match part.volume_mm3.review {
                    solidbom_contract::ReviewState::Confirmed => crate::i18n::tr("solidbom.reviewed"),
                    solidbom_contract::ReviewState::Pending => crate::i18n::tr("solidbom.pending"),
                },
                match part.volume_mm3.unit {
                    Unit::MillimetreCubed => "mm³",
                    _ => "",
                }
            ));
            // A manual surface value is only a review input once its source, reason and
            // confirmation are explicit; the app never turns an absent enclosed volume into zero.
            if part.volume_mm3.source == solidbom_contract::MeasurementSource::Manual {
                let reason = part
                    .volume_mm3
                    .reason
                    .as_deref()
                    .map(str::trim)
                    .filter(|reason| !reason.is_empty())
                    .unwrap_or(crate::i18n::tr("solidbom.workspace.unavailable"));
                let review = match part.volume_mm3.review {
                    solidbom_contract::ReviewState::Confirmed => crate::i18n::tr("solidbom.reviewed"),
                    solidbom_contract::ReviewState::Pending => crate::i18n::tr("solidbom.pending"),
                };
                let tone = if part.volume_mm3.review == solidbom_contract::ReviewState::Confirmed {
                    ui_kit::StatusTone::Dim
                } else {
                    ui_kit::StatusTone::Warning
                };
                ui_kit::status_line(ui, &crate::i18n::format("solidbom.workspace.manual_volume", &[("reason", reason), ("review", review)]), tone);
                if part.volume_mm3.reason.as_deref().is_none_or(|reason| reason.trim().is_empty()) {
                    ui_kit::status_line(ui, crate::i18n::tr("solidbom.workspace.manual_volume_missing_reason"), ui_kit::StatusTone::Warning);
                }
            }
            ui_kit::expr_row(ui, crate::i18n::tr("solidbom.manual_reason"), &mut panel.fields.manual_reason);
            let mut volume_confirmed = panel.fields.volume_confirmed;
            ui_kit::row(ui, crate::i18n::tr("solidbom.reviewed"), |ui| {
                ui_kit::checkbox(ui, "", &mut volume_confirmed);
            });
            panel.fields.volume_confirmed = volume_confirmed;
            material_choice(app, ui, panel);
        },
    );

    // Manufacturing inputs.
    let manufacturing = crate::i18n::tr("solidbom.manufacturing").to_string();
    ui_kit::section(
        ui,
        ("solidbom_manufacturing", panel.section.clone()),
        &manufacturing,
        matches!(panel.section.as_str(), "material" | "cycle" | "production_rate" | "setup"),
        |ui| {
            ui_kit::expr_row(ui, crate::i18n::tr("solidbom.allowance"), &mut panel.fields.allowance);
            ui_kit::expr_row(ui, crate::i18n::tr("solidbom.cycle"), &mut panel.fields.cycle);
            if panel.focus_cavities {
                ui_kit::row(ui, crate::i18n::tr("solidbom.cavities"), |ui| {
                    ui_kit::focused_field(ui, &mut panel.fields.cavities);
                });
                panel.focus_cavities = false;
            } else {
                ui_kit::expr_row(ui, crate::i18n::tr("solidbom.cavities"), &mut panel.fields.cavities);
            }
            rate_choice(app, ui, panel, true);
            ui_kit::expr_row(ui, crate::i18n::tr("solidbom.setup"), &mut panel.fields.setup_hours);
            rate_choice(app, ui, panel, false);
        },
    );
    ui.horizontal(|ui| {
        let changed = panel.fields != panel.baseline;
        for (label, action) in [("solidbom.apply", "view.bom.apply"), ("solidbom.workspace.revert", "view.bom.revert")] {
            let response = ui_kit::button(ui, crate::i18n::tr(label), ui_kit::Button::Enabled(changed));
            let response = if changed { response } else { response.on_disabled_hover_text(crate::i18n::tr("bom.ribbon.no_pending")) };
            if response.clicked() {
                panel.action = Some(action.into());
            }
        }
    });
    let overhead_title = crate::i18n::tr("solidbom.overhead").to_string();
    ui_kit::section(ui, ("solidbom_overhead", panel.section.clone()), &overhead_title, panel.section == "overhead", |ui| {
        ui_kit::expr_row(ui, &overhead_title, &mut panel.overhead);
        ui.horizontal(|ui| {
            let changed = panel.overhead != panel.overhead_baseline;
            let apply = ui_kit::button(ui, crate::i18n::tr("solidbom.apply"), ui_kit::Button::Enabled(changed));
            let apply = if changed { apply } else { apply.on_disabled_hover_text(crate::i18n::tr("bom.ribbon.no_pending")) };
            if apply.clicked() {
                panel.section = "overhead".into();
                panel.action = Some("view.bom.apply".into());
            }
            let revert = ui_kit::button(ui, crate::i18n::tr("solidbom.workspace.revert"), ui_kit::Button::Enabled(changed));
            let revert = if changed { revert } else { revert.on_disabled_hover_text(crate::i18n::tr("bom.ribbon.no_pending")) };
            if revert.clicked() {
                panel.section = "overhead".into();
                panel.action = Some("view.bom.revert".into());
            }
        });
    });

    // Cost explanation.
    let cost = crate::i18n::tr("solidbom.cost").to_string();
    ui_kit::section(ui, ("solidbom_cost", panel.section.clone()), &cost, matches!(panel.section.as_str(), "cost" | "calculation"), |ui| {
        let state = &app.session.solidbom;
        let currency = state.draft.as_ref().map(|draft| draft.currency.clone()).unwrap_or_default();
        ui.label(RichText::new(crate::i18n::tr("solidbom.prototype_label")).strong());
        ui_kit::status_line(ui, crate::i18n::tr("solidbom.omitted"), ui_kit::StatusTone::Dim);

        if matches!(state.result, ResultState::Stale) {
            ui_kit::status_line(ui, crate::i18n::tr("solidbom.workspace.stale_prior"), ui_kit::StatusTone::Warning);
            if command_button(app, ui, "solidbom.calculate", crate::i18n::tr("solidbom.workspace.recalculate")) {
                panel.action = Some("solidbom.calculate".into());
            }
        }

        let entries = history_entries(state);
        let selected = panel.history;
        let selected_is_current = selected.is_none_or(|id| state.current.as_ref().map(|current| current.calculation_id) == Some(id));
        if let Some(snapshot) = selected_snapshot(state, selected) {
            ui.label(RichText::new(snapshot_state_line(state, snapshot)).strong());
            // An incomplete result has no complete subtotal. Its issue lines stay visible, but a
            // partial backend amount is never presented as the part or assembly cost.
            let incomplete = snapshot.outputs.state == solidbom_contract::ResultState::Incomplete;
            if incomplete {
                ui_kit::status_line(ui, crate::i18n::tr("solidbom.workspace.incomplete_total"), ui_kit::StatusTone::Warning);
            } else {
                if let Some(part_cost) = snapshot.outputs.parts.iter().find(|cost| cost.part_version_id == part_id) {
                    let per_part = amount_text_of(part_cost.per_part_subtotal);
                    ui.label(crate::i18n::format("solidbom.workspace.per_part_total", &[("amount", &per_part), ("currency", &currency)]));
                    ui.label(RichText::new(crate::i18n::tr("solidbom.workspace.exact_lines")).small());
                    for line in &part_cost.explanations {
                        // The explanation lines keep the backend's exact amounts; only the summary
                        // amounts are shown at two places.
                        let amount = exact_text_of(line.amount);
                        ui.label(format!("{}: {amount}", line.label));
                        if !line.basis.is_empty() {
                            ui.label(RichText::new(format!("  {}", line.basis)).small().color(Tokens::get().text_dim));
                        }
                    }
                }
                let assembly = amount_text_of(snapshot.outputs.assembly_subtotal);
                ui.label(crate::i18n::format("solidbom.workspace.assembly_total_amount", &[("amount", &assembly), ("currency", &currency)]));
                let setup = amount_text_of(snapshot.outputs.setup_total);
                ui.label(crate::i18n::format("solidbom.workspace.setup_total_amount", &[("amount", &setup), ("currency", &currency)]));
                ui_kit::status_line(ui, crate::i18n::tr("solidbom.workspace.setup_separate"), ui_kit::StatusTone::Dim);
            }
            if !selected_is_current && let Some(inputs) = snapshot_inputs_line(snapshot, part_id) {
                ui.label(RichText::new(crate::i18n::tr("solidbom.workspace.history_inputs_title")).strong());
                ui.label(inputs);
            }
            // A snapshot carries the same finding in its inputs and in its outcome; show it once.
            let mut seen = std::collections::BTreeSet::new();
            let issues = snapshot
                .issues
                .iter()
                .chain(snapshot.outputs.issues.iter())
                .filter(|issue| seen.insert((issue.field_path.clone(), issue.code.clone())));
            let issues: Vec<_> = issues.collect();
            if incomplete && issues.is_empty() {
                ui_kit::status_line(ui, crate::i18n::tr("solidbom.incomplete"), ui_kit::StatusTone::Dim);
            }
            for issue in issues {
                ui.label(RichText::new(format!("{} ({})", issue.field_path, issue.code)).color(Tokens::get().warning));
            }
        } else if let Some(summary) = selected_summary(state, selected) {
            let version = summary.saved_version.to_string();
            ui_kit::status_line(ui, &crate::i18n::format("solidbom.workspace.history_not_loaded", &[("version", &version)]), ui_kit::StatusTone::Dim);
        } else if matches!(state.result, ResultState::Incomplete) {
            ui_kit::status_line(ui, crate::i18n::tr("solidbom.incomplete"), ui_kit::StatusTone::Dim);
        } else {
            ui_kit::status_line(ui, crate::i18n::tr("solidbom.no_result"), ui_kit::StatusTone::Dim);
        }

        if !entries.is_empty() {
            ui.label(RichText::new(crate::i18n::tr("solidbom.workspace.history")).strong());
            for entry in &entries {
                let version = entry.saved_version.to_string();
                let current = state.current.as_ref().map(|current| current.calculation_id) == Some(entry.calculation_id);
                let label = if current {
                    crate::i18n::format("solidbom.workspace.history_current_row", &[("version", &version)])
                } else {
                    crate::i18n::format("solidbom.workspace.history_row", &[("version", &version)])
                };
                let response = ui_kit::button(ui, label, ui_kit::Button::Normal);
                crate::scenario::publish_target(&format!("bom:history:{}", entry.calculation_id), response.rect);
                if response.clicked() {
                    panel.history = Some(entry.calculation_id);
                }
            }
        }
    });

    ui.separator();
    ui.horizontal_wrapped(|ui| {
        if command_button(app, ui, "solidbom.save", crate::i18n::tr("solidbom.save")) {
            panel.action = Some("solidbom.save".into());
        }
        if command_button(app, ui, "solidbom.calculate", crate::i18n::tr("solidbom.calculate")) {
            panel.action = Some("solidbom.calculate".into());
        }
        let retryable = app.session.solidbom.failed.as_ref().is_some_and(|failed| failed.retryable);
        if retryable && command_button(app, ui, "solidbom.retry", crate::i18n::tr("solidbom.retry")) {
            panel.action = Some("solidbom.retry".into());
        }
    });
    // A central project save and a local CAD file save are different actions; neither implies the
    // other, and a failed central write never counts as saved.
    ui_kit::status_line(ui, crate::i18n::tr("solidbom.workspace.save_separation"), ui_kit::StatusTone::Dim);
    if let Some(error) = &panel.error {
        ui_kit::status_line(ui, error, ui_kit::StatusTone::Error);
    }
}

pub(super) fn sync_selection(app: &mut SolveApp, panel: &mut Panel) {
    let Some(draft) = app.session.solidbom.draft.clone() else { return };
    let overhead = draft.material_overhead_rate.map(|value| value.to_string()).unwrap_or_default();
    if panel.overhead == panel.overhead_baseline {
        panel.overhead.clone_from(&overhead);
    }
    panel.overhead_baseline = overhead;
    if panel.part.is_some() && panel.observed_selection == app.session.selection {
        return;
    }
    panel.observed_selection.clone_from(&app.session.selection);
    // An occurrence selection is a runtime CAD id: map it through the import's rebuilt runtime
    // map to the stable source path the reviewed record persists. Body, face and edge picks need
    // the exact-geometry resolver, which refuses a part placed more than once.
    let selected = app.session.selection.iter().find_map(|selection| match selection {
        solidcraft_engine::Sel::Occurrence { path } => path
            .last()
            .and_then(|runtime| app.session.solidbom.runtime.as_ref()?.source_path(*runtime).ok())
            .and_then(|source| draft.analysis.occurrences.iter().find(|occurrence| occurrence.source_path == source).cloned()),
        _ => solidcraft_engine::solidbom::resolve_pick(&app.session, &draft.analysis, selection).ok(),
    });
    let part = selected
        .and_then(|occurrence| draft.parts.iter().find(|part| part.definition_id == occurrence.definition_id).map(|part| part.part_version_id))
        .or_else(|| draft.parts.first().map(|part| part.part_version_id));
    if let Some(part) = part
        && panel.part != Some(part)
    {
        load(app, panel, part);
        panel.reveal_part = true;
    }
}

/// What the panel needs to re-resolve its material and rate choices. A reopen publishes the saved
/// review before its reference data arrives, so the choice indices are re-read when it does.
fn reference_fingerprint(app: &SolveApp) -> (usize, usize) {
    app.session.solidbom.reference.as_ref().map_or((0, 0), |reference| (reference.materials.len(), reference.rates.len()))
}

fn load(app: &SolveApp, panel: &mut Panel, part_id: Uuid) {
    let Some(draft) = &app.session.solidbom.draft else { return };
    let Some(part) = draft.parts.iter().find(|part| part.part_version_id == part_id) else { return };
    panel.stash();
    if let Some((fields, baseline, generation)) = panel.drafts.get(&part_id)
        && fields != baseline
    {
        panel.fields = fields.clone();
        panel.baseline = baseline.clone();
        panel.loaded = Some((part_id, *generation, reference_fingerprint(app)));
        panel.part = Some(part_id);
        return;
    }
    panel.fields = Fields {
        classification: part.classification.clone(),
        confirmed: part.classification_confirmed,
        manufactured: part.sourcing == Sourcing::Manufactured,
        volume: part.volume_mm3.value.map(|value| value.to_string()).unwrap_or_default(),
        manual_reason: part.volume_mm3.reason.clone().unwrap_or_default(),
        volume_confirmed: part.volume_mm3.review == solidbom_contract::ReviewState::Confirmed,
        material: app
            .session
            .solidbom
            .reference
            .as_ref()
            .and_then(|reference| reference.materials.iter().position(|material| Some(material.material_version_id) == part.material_version_id))
            .map_or(0, |index| index + 1),
        allowance: part.allowance_rate.map(|value| value.to_string()).unwrap_or_default(),
        cycle: part.operation.as_ref().and_then(|operation| operation.cycle_seconds).map(|value| value.to_string()).unwrap_or_default(),
        cavities: part.operation.as_ref().and_then(|operation| operation.cavities).map(|value| value.to_string()).unwrap_or_default(),
        production_rate: app
            .session
            .solidbom
            .reference
            .as_ref()
            .and_then(|reference| {
                reference
                    .rates
                    .iter()
                    .position(|rate| Some(rate.rate_version_id) == part.operation.as_ref().and_then(|operation| operation.production_rate_version_id))
            })
            .map_or(0, |index| index + 1),
        setup_hours: part.operation.as_ref().and_then(|operation| operation.setup_hours).map(|value| value.to_string()).unwrap_or_default(),
        setup_rate: app
            .session
            .solidbom
            .reference
            .as_ref()
            .and_then(|reference| {
                reference
                    .rates
                    .iter()
                    .position(|rate| Some(rate.rate_version_id) == part.operation.as_ref().and_then(|operation| operation.setup_rate_version_id))
            })
            .map_or(0, |index| index + 1),
    };
    panel.baseline = panel.fields.clone();
    panel.part = Some(part_id);
    panel.loaded = Some((part_id, app.session.solidbom.generation, reference_fingerprint(app)));
}

fn material_choice(app: &mut SolveApp, ui: &mut egui::Ui, panel: &mut Panel) {
    let mut names = vec!["—".to_string()];
    if let Some(reference) = &app.session.solidbom.reference {
        names.extend(reference.materials.iter().map(|material| material.name.clone()));
    }
    let borrowed: Vec<&str> = names.iter().map(String::as_str).collect();
    ui_kit::choice_row(ui, crate::i18n::tr("solidbom.material"), "solidbom_material", &mut panel.fields.material, &borrowed);
}

fn rate_choice(app: &mut SolveApp, ui: &mut egui::Ui, panel: &mut Panel, production: bool) {
    let mut names = vec!["—".to_string()];
    if let Some(reference) = &app.session.solidbom.reference {
        names.extend(reference.rates.iter().map(|rate| rate.name.clone()));
    }
    let borrowed: Vec<&str> = names.iter().map(String::as_str).collect();
    let label = if production { crate::i18n::tr("solidbom.production_rate") } else { crate::i18n::tr("solidbom.setup") };
    let id = if production { "solidbom_production_rate" } else { "solidbom_setup_rate" };
    let selected = if production { &mut panel.fields.production_rate } else { &mut panel.fields.setup_rate };
    ui_kit::choice_row(ui, label, id, selected, &borrowed);
}

fn params(app: &SolveApp, part: Uuid, fields: &Fields) -> Result<Value, String> {
    let cavities = if fields.cavities.trim().is_empty() {
        None
    } else {
        Some(
            fields
                .cavities
                .parse::<u32>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| crate::i18n::tr("solidbom.workspace.invalid_cavities").to_string())?,
        )
    };
    let material = app
        .session
        .solidbom
        .reference
        .as_ref()
        .and_then(|reference| reference.materials.get(fields.material.checked_sub(1).unwrap_or(usize::MAX)))
        .map(|material| material.material_version_id.to_string())
        .unwrap_or_default();
    let production_rate = app
        .session
        .solidbom
        .reference
        .as_ref()
        .and_then(|reference| reference.rates.get(fields.production_rate.checked_sub(1).unwrap_or(usize::MAX)))
        .map(|rate| rate.rate_version_id.to_string())
        .unwrap_or_default();
    let setup_rate = app
        .session
        .solidbom
        .reference
        .as_ref()
        .and_then(|reference| reference.rates.get(fields.setup_rate.checked_sub(1).unwrap_or(usize::MAX)))
        .map(|rate| rate.rate_version_id.to_string())
        .unwrap_or_default();
    Ok(json!({
        "part_version_id": part.to_string(),
        "classification": fields.classification,
        "classification_confirmed": fields.confirmed,
        "sourcing": if fields.manufactured { "manufactured" } else { "purchased" },
        "volume_mm3": fields.volume,
        "manual_reason": (!fields.manual_reason.trim().is_empty()).then_some(fields.manual_reason.clone()),
        "volume_confirmed": fields.volume_confirmed,
        "material_version_id": material,
        "allowance_rate": fields.allowance,
        "cycle_seconds": fields.cycle,
        "cavities": cavities,
        "production_rate_version_id": production_rate,
        "setup_hours": fields.setup_hours,
        "setup_rate_version_id": setup_rate,
    }))
}

fn phase_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Idle => crate::i18n::tr("solidbom.phase.idle"),
        Phase::Importing => crate::i18n::tr("solidbom.phase.importing"),
        Phase::Reviewing => crate::i18n::tr("solidbom.phase.reviewing"),
    }
}

fn save_label(save: &SaveState) -> &'static str {
    match save {
        SaveState::Clean => crate::i18n::tr("solidbom.save.clean"),
        SaveState::Dirty => crate::i18n::tr("solidbom.save.dirty"),
        SaveState::Saving { .. } => crate::i18n::tr("solidbom.save.saving"),
        SaveState::SaveFailed { .. } => crate::i18n::tr("solidbom.save.failed"),
    }
}

fn result_label(result: &ResultState) -> &'static str {
    match result {
        ResultState::NoResult => crate::i18n::tr("solidbom.result.none"),
        ResultState::Calculating { .. } => crate::i18n::tr("solidbom.result.calculating"),
        ResultState::Current => crate::i18n::tr("solidbom.result.current"),
        ResultState::Stale => crate::i18n::tr("solidbom.result.stale"),
        ResultState::Incomplete => crate::i18n::tr("solidbom.result.incomplete"),
    }
}

pub fn workspace_ui(app: &mut SolveApp, ui: &mut egui::Ui) {
    super::workspace::show(app, ui);
}

pub(super) fn select_part(app: &SolveApp, panel: &mut Panel, part: Uuid) {
    load(app, panel, part);
    panel.reveal_part = true;
}

/// Draft buffers participate in review close/quit independently of CAD undo history.
pub fn has_active_pending(app: &SolveApp) -> bool {
    app.solidbom
        .as_ref()
        .is_some_and(|panel| if panel.section == "overhead" { panel.overhead != panel.overhead_baseline } else { panel.fields != panel.baseline })
}

pub fn has_pending(app: &SolveApp) -> bool {
    app.solidbom.as_ref().is_some_and(Panel::has_pending)
}

pub fn apply_all(app: &mut SolveApp) -> Result<(), String> {
    let Some(mut panel) = app.solidbom.take() else { return Ok(()) };
    let result = apply_edits(app, &mut panel, true, false);
    app.solidbom = Some(panel);
    result
}

fn apply_edits(app: &mut SolveApp, panel: &mut Panel, all: bool, overhead_only: bool) -> Result<(), String> {
    panel.stash();
    let mut patches = Vec::new();
    let mut applied = Vec::new();
    for (&id, (fields, baseline, _)) in &panel.drafts {
        if fields == baseline || overhead_only || (!all && panel.part != Some(id)) {
            continue;
        }
        match params(app, id, fields) {
            Ok(patch) => {
                patches.push(patch);
                applied.push(id);
            }
            Err(error) => {
                let error = error.clone();
                load(app, panel, id);
                panel.section = "cycle".into();
                panel.focus_cavities = true;
                panel.inspector = true;
                panel.error = Some(error.clone());
                return Err(error);
            }
        }
    }
    let overhead_changed = (all || overhead_only) && panel.overhead != panel.overhead_baseline;
    if patches.is_empty() && !overhead_changed {
        return Ok(());
    }
    let mut request = json!({"parts":patches});
    if overhead_changed {
        request["material_overhead_rate"] = json!(panel.overhead);
    }
    match app.run("solidbom.inputs.batch", request) {
        Ok(_) => {
            for id in applied {
                panel.drafts.remove(&id);
            }
            if !overhead_only {
                panel.baseline = panel.fields.clone();
                panel.loaded = panel.part.map(|id| (id, app.session.solidbom.generation, reference_fingerprint(app)));
            }
            if overhead_changed {
                panel.overhead_baseline.clone_from(&panel.overhead);
            }
            panel.error = None;
            Ok(())
        }
        Err(error) => {
            panel.error = Some(error.clone());
            Err(error)
        }
    }
}

/// Apply validated presentation replies through the same route as ribbon and automation.
pub fn apply_view_command(app: &mut SolveApp, id: &str, value: &Value) -> Result<(), String> {
    if id == "view.workspace" {
        let workspace = serde_json::from_value(value.get("workspace").cloned().unwrap_or(Value::Null)).map_err(|error| format!("{error}"))?;
        return crate::workspace::enter(app, workspace);
    }
    if !id.starts_with("view.bom.") {
        return Ok(());
    }
    crate::workspace::enter(app, crate::workspace::WorkspaceKind::Bom)?;
    if id == "view.bom.tab" {
        if let Some(tab) = value.get("tab").and_then(Value::as_str) {
            app.ui.tab = format!("BOM {}", tab.to_uppercase());
            app.workspace.bom_tab.clone_from(&app.ui.tab);
        }
        return Ok(());
    }
    let mut panel = app.solidbom.take().unwrap_or_default();
    let action = value.get("action").and_then(Value::as_str).unwrap_or_else(|| id.strip_prefix("view.bom.").unwrap_or_default());
    let mut result = Ok(());
    match action {
        "section" => {
            if let Some(section) = value.get("section").and_then(Value::as_str) {
                panel.section = section.into();
            }
            panel.inspector = true;
        }
        "filter" => {
            if let Some(filter) = value.get("filter").and_then(Value::as_str) {
                panel.filter = filter.into();
            }
            if let Some(query) = value.get("query").and_then(Value::as_str) {
                panel.query = query.into();
            }
            panel.table = true;
        }
        "panels" => {
            if value.get("reset").and_then(Value::as_bool) == Some(true) {
                reset_layout(app, &mut panel);
            }
            if let Some(visible) = value.get("browser").and_then(Value::as_bool) {
                app.ui.show_browser = visible;
            }
            if let Some(visible) = value.get("table").and_then(Value::as_bool) {
                panel.table = visible;
            }
            if let Some(visible) = value.get("inspector").and_then(Value::as_bool) {
                panel.inspector = visible;
            }
        }
        "apply" => {
            let overhead = panel.section == "overhead";
            result = apply_edits(app, &mut panel, false, overhead);
        }
        "revert" => {
            if panel.section == "overhead" {
                panel.overhead.clone_from(&panel.overhead_baseline);
            } else {
                panel.fields = panel.baseline.clone();
                if let Some(id) = panel.part {
                    panel.drafts.remove(&id);
                }
            }
            panel.error = None;
        }
        "next_issue" => {
            let rows = super::workspace::rows(app);
            let current = rows
                .iter()
                .position(|row| row.get("part_version_id").and_then(Value::as_str).and_then(|id| Uuid::parse_str(id).ok()) == panel.part)
                .unwrap_or(0);
            let next = rows.iter().cycle().skip(current + 1).take(rows.len()).find(|row| {
                row.get("needs_review").and_then(Value::as_bool) == Some(true)
                    || row.get("issues").and_then(Value::as_array).is_some_and(|issues| !issues.is_empty())
            });
            if let Some(row) = next {
                super::workspace::select_row(app, &mut panel, row);
            }
        }
        "missing_inputs" => {
            panel.filter = "missing_inputs".into();
            panel.table = true;
            panel.section = "cost".into();
            panel.inspector = true;
        }
        "browser" => app.ui.show_browser = !app.ui.show_browser,
        "table" => panel.table = !panel.table,
        "inspector" => panel.inspector = !panel.inspector,
        "reset" => reset_layout(app, &mut panel),
        other => {
            panel.section = match other {
                "project_details" => "project",
                "cost_breakdown" => "cost",
                "calculation_details" => "calculation",
                other => other,
            }
            .into();
            panel.inspector = true;
        }
    }
    app.solidbom = Some(panel);
    result
}

fn reset_layout(app: &mut SolveApp, panel: &mut Panel) {
    panel.table = true;
    panel.inspector = true;
    panel.table_fraction = 0.35;
    panel.layout_generation = panel.layout_generation.wrapping_add(1);
    app.ui.show_browser = true;
}

pub fn start_interactive(app: &mut SolveApp, id: &str) -> bool {
    match id {
        "solidbom.import" => app.pick_open_for(&["step", "stp"], |app, path| {
            if let Some(path) = path
                && let Err(error) = open_review_tab(app, "solidbom.import", json!({"path":path}))
            {
                app.set_status(crate::i18n::format("solidbom.workspace.import_failed", &[("error", &error)]), true);
            }
        }),
        "solidbom.reopen" => app.solidbom.get_or_insert_with(Panel::default).project_id = Some(String::new()),
        "solidbom.save" | "solidbom.calculate" => {
            if let Err(error) = apply_all(app).and_then(|()| app.run(id, json!({})).map(|_| ())) {
                app.set_status(error, true);
            }
        }
        "solidbom.review" | "solidbom.retry" => {
            let _ = app.run(id, json!({}));
        }
        id if id.starts_with("view.bom.") => {
            let _ = app.run(id, json!({}));
        }
        _ => return false,
    }
    true
}

pub(super) fn open_review_tab(app: &mut SolveApp, command: &str, params: Value) -> Result<(), String> {
    let mut session = solidcraft_engine::Session::default();
    session.solidbom_host = app.services.solidbom.clone().or_else(|| app.session.solidbom_host.clone());
    // The imported source is opened under the host's pre-command check like any other design.
    session.set_command_check(app.session.command_check());
    session.set_validation(app.preferences.validation);
    session.execute(command, &params).map_err(|error| error.to_string())?;
    crate::documents::open_tab(app, session)?;
    open(app);
    app.home.open = false;
    Ok(())
}

pub fn inspect(app: &SolveApp) -> Value {
    let Some(panel) = app.solidbom.as_ref() else { return Value::Null };
    json!({"tab":app.ui.tab, "part_version_id":panel.part, "section":panel.section,"filter":panel.filter,"query":panel.query,
        "sort":panel.sort,"descending":panel.descending,"table":panel.table,"inspector":panel.inspector,
        "table_fraction":panel.table_fraction,"legacy_window":app.preferences.solidbom_legacy_window,
        "unapplied":panel.has_pending(),"error":panel.error})
}

#[cfg(test)]
mod editor_tests {
    use super::*;
    use solidbom_contract::{
        CalculationInputs, CalculationOutcome, CalculationSummary, CostLine, CycleBasis, MaterialVersion, OperationInputs, PartCost, RateVersion,
        ReferenceData, ResolvedPartInput, ResultState as OutcomeState,
    };

    fn fixture(name: &str) -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/solidbom/fixtures").join(name)
    }

    fn imported(fixture_name: &str) -> crate::scenario::Harness {
        let mut h = crate::scenario::Harness::new();
        h.app.run("solidbom.import", json!({"path": fixture(fixture_name).to_string_lossy()})).unwrap();
        h.settle();
        h.call("ui.resize", json!({"width": 1600, "height": 1000}));
        h.frames(3);
        h
    }

    /// A saved calculation for the three-box fixture: one reviewed definition with three
    /// occurrences, exact unrounded amounts and one explained line.
    fn snapshot(version: u64, part: Uuid, cycle: &str, per_part: &str, assembly: &str, setup: &str) -> CalculationSnapshot {
        CalculationSnapshot {
            calculation_id: Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("calculation:{version}").as_bytes()),
            project_id: Uuid::new_v5(&Uuid::NAMESPACE_OID, b"project"),
            saved_version: version,
            input_hash: "a".repeat(64),
            calculator_version: "test".into(),
            inputs: CalculationInputs {
                currency: "CNY".into(),
                material_overhead_rate: Some("0.05".parse().expect("overhead")),
                parts: vec![ResolvedPartInput {
                    part_version_id: part,
                    quantity: 3,
                    volume_mm3: Some("2400".parse().expect("volume")),
                    density_g_cm3: Some("2.7".parse().expect("density")),
                    material_price_per_kg: Some("20".parse().expect("price")),
                    allowance_rate: Some("0.02".parse().expect("allowance")),
                    cycle_seconds: Some(cycle.parse().expect("cycle")),
                    cavities: Some(1),
                    production_rate_per_hour: Some("120".parse().expect("rate")),
                    setup_hours: Some("0.5".parse().expect("hours")),
                    setup_rate_per_hour: Some("120".parse().expect("rate")),
                }],
            },
            outputs: CalculationOutcome {
                state: OutcomeState::Complete,
                parts: vec![PartCost {
                    part_version_id: part,
                    quantity: 3,
                    net_mass_kg: Some("0.00648".parse().expect("mass")),
                    allowed_material_cost: Some("0.132192".parse().expect("cost")),
                    material_overhead: Some("0.0066096".parse().expect("cost")),
                    production_cost: Some("1".parse().expect("cost")),
                    per_part_subtotal: Some(per_part.parse().expect("subtotal")),
                    line_total: Some(assembly.parse().expect("line total")),
                    explanations: vec![CostLine {
                        code: "production".into(),
                        label: "Production cost".into(),
                        amount: Some("1".parse().expect("cost")),
                        basis: "30 / 3600 x 120".into(),
                    }],
                }],
                assembly_subtotal: Some(assembly.parse().expect("assembly")),
                setup_total: Some(setup.parse().expect("setup")),
                issues: Vec::new(),
            },
            issues: Vec::new(),
        }
    }

    #[test]
    fn amounts_are_rounded_to_two_places_from_the_exact_backend_values() {
        assert_eq!(amount_text(Some("1.1388016")), "1.14");
        assert_eq!(amount_text(Some("3.4164048")), "3.42");
        assert_eq!(amount_text(Some("60")), "60.00");
        assert_eq!(amount_text(Some("2.1388016")), "2.14");
        assert_eq!(amount_text(Some("6.4164048")), "6.42");
        assert_eq!(amount_text(Some("0.0066096")), "0.01");
        assert_eq!(amount_text(Some("999.995")), "1000.00");
        assert_eq!(amount_text(Some("-1.138")), "-1.14");
        assert_eq!(amount_text(Some("0")), "0.00", "an explicit zero is a zero, not missing");
        assert_eq!(amount_text(None), "Unavailable");
        assert_eq!(amount_text(Some("")), "Unavailable");
        assert_eq!(amount_two_places(Some("not a number")), None);
    }

    #[test]
    fn the_selected_part_names_its_shared_definition_and_expanded_quantity() {
        let h = imported("three_boxes.step");
        let draft = h.app.session.solidbom.draft.as_ref().expect("the draft");
        let part = draft.parts[0].part_version_id;
        let context = part_context(draft, part).expect("the selected part's context");
        assert_eq!(context.definition_id, draft.analysis.definitions[0].definition_id.to_string());
        assert_eq!(context.quantity(), 3, "the three occurrences share one reviewed record");
        assert_eq!(context.occurrence_ids.len(), 3);
        let occurrence_ids: Vec<String> = draft.analysis.occurrences.iter().map(|occurrence| occurrence.occurrence_id.to_string()).collect();
        assert_eq!(context.occurrence_ids, occurrence_ids, "each placement keeps its own occurrence identity");

        let revisions = imported("revisions_bc.step");
        let draft = revisions.app.session.solidbom.draft.as_ref().expect("the revisions draft");
        assert_eq!(draft.parts.len(), 2);
        let left = part_context(draft, draft.parts[0].part_version_id).expect("the first revision");
        let right = part_context(draft, draft.parts[1].part_version_id).expect("the second revision");
        assert_ne!(left.definition_id, right.definition_id, "same part number and different revisions stay separate");
        assert_eq!(left.quantity(), 1);
        assert_eq!(right.quantity(), 1);
    }

    #[test]
    fn a_flat_source_surfaces_a_review_required_grouping_issue() {
        let h = imported("flat_multi_body.step");
        let draft = h.app.session.solidbom.draft.as_ref().expect("the flat draft");
        let lines = analysis_issue_lines(draft);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].contains("unresolved_grouping"), "the named issue code stays visible: {lines:?}");
        assert!(lines[0].contains("review the bodies as components"), "the issue is a review requirement: {lines:?}");
    }

    #[test]
    fn a_confirmed_manual_volume_resolves_the_missing_volume_review_requirement() {
        let mut h = imported("open_surface.step");
        let draft = h.app.session.solidbom.draft.as_ref().expect("the open-surface draft");
        let part = draft.parts[0].part_version_id;
        assert!(!analysis_issue_lines(draft).is_empty(), "the open surface asks for a volume before review");
        h.app
            .run(
                "solidbom.inputs",
                json!({"part_version_id": part.to_string(), "volume_mm3": "1200", "manual_reason": "measured the surface model", "volume_confirmed": true}),
            )
            .expect("the manual volume");
        let draft = h.app.session.solidbom.draft.as_ref().expect("the reviewed draft");
        assert_eq!(draft.parts[0].volume_mm3.source, solidbom_contract::MeasurementSource::Manual);
        assert!(
            analysis_issue_lines(draft).is_empty(),
            "the source fact is recorded, but the missing-volume review requirement is satisfied: {:?}",
            analysis_issue_lines(draft)
        );
    }

    #[test]
    fn the_same_issue_arriving_in_both_snapshot_lists_is_shown_once() {
        let mut h = imported("three_boxes.step");
        let part = h.app.session.solidbom.draft.as_ref().expect("the draft").parts[0].part_version_id;
        let issue = Issue::new("missing_cycle_time", "parts[0].operation.cycle_seconds", "the cycle time is missing");
        let mut result = snapshot(2, part, "30", "1.1388016", "3.4164048", "60");
        result.outputs.state = OutcomeState::Incomplete;
        result.issues = vec![issue.clone()];
        result.outputs.issues = vec![issue];
        h.app.session.solidbom.current = Some(result);
        h.app.session.solidbom.result = ResultState::Incomplete;
        h.app.run("view.bom.section", json!({"section": "cost"})).unwrap();
        h.frames(3);
        assert_eq!(h.widgets("parts[0].operation.cycle_seconds (missing_cycle_time)").len(), 1, "the same finding is not repeated");
    }

    #[test]
    fn a_failed_save_is_never_shown_as_saved_and_keeps_its_message() {
        let failed = SaveState::SaveFailed { request_id: "request-1".into(), message: "the service is unreachable".into() };
        assert_eq!(save_failure_message(&failed), Some("the service is unreachable"));
        assert_eq!(save_label(&failed), "Save failed");
        assert_ne!(save_label(&failed), save_label(&SaveState::Clean));
        assert_eq!(save_failure_message(&SaveState::Clean), None);
        assert_eq!(save_failure_message(&SaveState::Dirty), None);

        let mut h = imported("three_boxes.step");
        h.app.session.solidbom.save = failed;
        h.app.run("view.bom.section", json!({"section": "cost"})).unwrap();
        h.frames(3);
        assert!(
            h.widget("Save failed: the service is unreachable. The project is not saved.").is_some(),
            "the failure and that the project is not saved are visible"
        );
    }

    #[test]
    fn the_history_lists_snapshots_and_summaries_newest_first() {
        let part = Uuid::from_u128(7);
        let first = snapshot(2, part, "30", "1.1388016", "3.4164048", "60");
        let second = snapshot(3, part, "60", "2.1388016", "6.4164048", "60");
        let summary = CalculationSummary { calculation_id: Uuid::from_u128(9), saved_version: 1, calculator_version: "test".into() };
        let mut state = State {
            calculations: vec![first.clone(), second.clone()],
            current: Some(second.clone()),
            summaries: vec![summary.clone()],
            result: ResultState::Current,
            ..Default::default()
        };

        let entries = history_entries(&state);
        let versions: Vec<u64> = entries.iter().map(|entry| entry.saved_version).collect();
        assert_eq!(versions, vec![3, 2, 1]);
        assert!(entries[0].loaded && entries[1].loaded && !entries[2].loaded, "{entries:?}");
        assert_eq!(selected_snapshot(&state, None).map(|snapshot| snapshot.calculation_id), Some(second.calculation_id));
        assert!(selected_summary(&state, Some(summary.calculation_id)).is_some());
        assert!(selected_snapshot(&state, Some(summary.calculation_id)).is_none(), "a summary has no explanations");

        assert!(is_current_snapshot(&state, &second));
        assert!(!is_current_snapshot(&state, &first));
        assert!(snapshot_state_line(&state, &first).contains("not current"), "{}", snapshot_state_line(&state, &first));
        assert!(snapshot_state_line(&state, &second).contains("Current result"));
        state.result = ResultState::Stale;
        assert!(snapshot_state_line(&state, &second).contains("stale"), "{}", snapshot_state_line(&state, &second));
    }

    #[test]
    fn an_earlier_result_shows_its_explanations_and_its_own_inputs() {
        let part = Uuid::from_u128(7);
        let earlier = snapshot(2, part, "30", "1.1388016", "3.4164048", "60");
        let later = snapshot(3, part, "60", "2.1388016", "6.4164048", "60");
        let inputs = snapshot_inputs_line(&earlier, part).expect("the earlier result's inputs");
        assert!(inputs.contains("cycle 30 s"), "the earlier result keeps its own cycle: {inputs}");
        assert!(inputs.contains("Quantity 3"), "{inputs}");
        let later_inputs = snapshot_inputs_line(&later, part).expect("the later result's inputs");
        assert!(later_inputs.contains("cycle 60 s"), "{later_inputs}");
        let explanations: Vec<&str> = earlier.outputs.parts[0].explanations.iter().map(|line| line.label.as_str()).collect();
        assert_eq!(explanations, vec!["Production cost"]);
    }

    #[test]
    fn reference_data_arriving_after_the_review_resolves_the_material_and_rate_choices() {
        let mut h = imported("three_boxes.step");
        let material = Uuid::parse_str("00000000-0000-4000-8000-000000000001").expect("material");
        let rate = Uuid::parse_str("00000000-0000-4000-8000-000000000101").expect("rate");
        {
            // The reopened saved review carries its choices before the reference data arrives.
            let state = &mut h.app.session.solidbom;
            let draft = state.draft.as_mut().expect("the draft");
            draft.parts[0].material_version_id = Some(material);
            draft.parts[0].operation = Some(OperationInputs {
                cycle_seconds: Some("30".parse().expect("cycle")),
                cycle_basis: CycleBasis::PerPiece,
                cavities: Some(1),
                production_rate_version_id: Some(rate),
                setup_hours: Some("0.5".parse().expect("hours")),
                setup_rate_version_id: Some(rate),
            });
            state.generation = state.generation.wrapping_add(1);
        }
        h.frames(2);
        assert_eq!(h.app.solidbom.as_ref().expect("the panel").fields.material, 0, "no reference data yet: no choice");
        h.app.session.solidbom.reference = Some(ReferenceData {
            materials: vec![MaterialVersion {
                material_version_id: material,
                name: "PA12 test fixture".into(),
                density_g_cm3: "2.7".parse().expect("density"),
                price_per_kg: "20".parse().expect("price"),
                currency: "CNY".into(),
            }],
            rates: vec![RateVersion {
                rate_version_id: rate,
                name: "Machining test fixture".into(),
                rate_per_hour: "120".parse().expect("rate"),
                currency: "CNY".into(),
            }],
        });
        h.frames(3);
        let fields = &h.app.solidbom.as_ref().expect("the panel").fields;
        assert_eq!(fields.material, 1, "the arriving reference data resolves the material choice");
        assert_eq!(fields.production_rate, 1);
        assert_eq!(fields.setup_rate, 1);
    }

    #[test]
    fn an_incomplete_result_shows_no_complete_cost() {
        let mut h = imported("three_boxes.step");
        let part = h.app.session.solidbom.draft.as_ref().expect("the draft").parts[0].part_version_id;
        let mut incomplete = snapshot(2, part, "30", "1.4164048", "1.4164048", "60");
        incomplete.outputs.state = OutcomeState::Incomplete;
        h.app.session.solidbom.current = Some(incomplete);
        h.app.session.solidbom.result = ResultState::Incomplete;
        h.app.run("view.bom.section", json!({"section": "cost"})).unwrap();
        h.frames(3);
        assert!(h.widgets("Direct cost per part: 1.42 CNY").is_empty(), "no per-part total for an incomplete result");
        assert!(h.widgets("Assembly direct-cost subtotal: 1.42 CNY").is_empty(), "no assembly total for an incomplete result");
        assert!(h.widgets("Setup charge (separate): 60.00 CNY").is_empty(), "no setup total for an incomplete result");
        assert!(h.widget("No complete direct-cost subtotal is shown: required inputs are missing or the grouping is unresolved.").is_some());
        assert!(h.widget("Incomplete result: version 2.").is_some());
    }

    #[test]
    fn a_failed_save_hides_an_older_saved_notice() {
        let mut h = imported("three_boxes.step");
        h.app.session.solidbom.save = SaveState::SaveFailed { request_id: "request-1".into(), message: "the service is unreachable".into() };
        h.app.session.solidbom.notice = Some("SolidBOM project saved as version 2".into());
        h.frames(3);
        assert!(h.widget("Save failed: the service is unreachable. The project is not saved.").is_some());
        assert!(h.widget("SolidBOM project saved as version 2").is_none(), "a failed save never reads as the saved one");
    }

    #[test]
    fn the_selected_part_shows_its_customer_part_number_and_revision_when_known() {
        let mut h = imported("three_boxes.step");
        let mut draft = h.app.session.solidbom.draft.clone().expect("the draft");
        draft.analysis.definitions[0].customer_part_number = Some("PL-100".into());
        draft.analysis.definitions[0].customer_revision = Some("B".into());
        let context = part_context(&draft, draft.parts[0].part_version_id).expect("the context");
        assert_eq!(context.part_number.as_deref(), Some("PL-100"));
        assert_eq!(context.revision.as_deref(), Some("B"));
        h.app.session.solidbom.draft = Some(draft);
        h.frames(3);
        assert!(h.widget("PL-100 · revision B").is_some(), "the reviewed part number is shown above its identity");
    }

    #[test]
    fn a_loaded_result_for_an_older_saved_version_is_history_not_current() {
        let mut h = imported("three_boxes.step");
        let part = h.app.session.solidbom.draft.as_ref().expect("the draft").parts[0].part_version_id;
        let earlier = snapshot(2, part, "30", "1.1388016", "3.4164048", "60");
        {
            let state = &mut h.app.session.solidbom;
            state.calculations = vec![earlier.clone()];
            state.current = None;
            state.result = ResultState::NoResult;
            if let Some(draft) = state.draft.as_mut() {
                draft.version = 3;
            }
        }
        h.app.solidbom.as_mut().expect("the panel").history = Some(earlier.calculation_id);
        h.app.run("view.bom.section", json!({"section": "cost"})).unwrap();
        h.frames(3);
        assert!(h.widget("Earlier result: version 2; it is not current.").is_some());
        assert!(h.widget("Current result: version 2").is_none(), "an older saved version is never the current result");
        assert!(h.widget("Direct cost per part: 1.14 CNY").is_some(), "the saved earlier result stays readable");
        assert!(h.widget("Version 2").is_some(), "the history lists the loaded older result");
        let state = &h.app.session.solidbom;
        assert!(state.current.is_none());
        assert!(!is_current_snapshot(state, &earlier));
    }

    #[test]
    fn the_cost_section_shows_two_place_totals_separate_setup_and_history() {
        let mut h = imported("three_boxes.step");
        let part = h.app.session.solidbom.draft.as_ref().expect("the draft").parts[0].part_version_id;
        let first = snapshot(2, part, "30", "1.1388016", "3.4164048", "60");
        let second = snapshot(3, part, "60", "2.1388016", "6.4164048", "60");
        h.app.session.solidbom.current = Some(second.clone());
        h.app.session.solidbom.calculations = vec![first.clone(), second.clone()];
        h.app.session.solidbom.result = ResultState::Current;
        h.app.run("view.bom.section", json!({"section": "cost"})).unwrap();
        h.frames(4);
        assert!(h.widget("Direct cost per part: 2.14 CNY").is_some(), "the current per-part amount is two places");
        assert!(h.widget("Assembly direct-cost subtotal: 6.42 CNY").is_some());
        assert!(h.widget("Setup charge (separate): 60.00 CNY").is_some());
        assert!(h.widget("Calculation history").is_some());
        assert!(h.widget("Version 3 (current)").is_some());
        assert!(h.widget("Version 2").is_some());

        // Selecting the earlier result shows its explanations and its own inputs, clearly not current.
        h.step(&json!({"click": {"handle": format!("bom:history:{}", first.calculation_id)}})).unwrap();
        h.frames(3);
        assert!(h.widget("Earlier result: version 2; it is not current.").is_some());
        assert!(h.widget("Inputs saved with this result").is_some());
        assert!(h.widget("Direct cost per part: 1.14 CNY").is_some());
        assert!(h.widget("Production cost: 1").is_some(), "the exact explanation line stays visible");

        // A stale result keeps the prior snapshot readable and offers an explicit recalculation.
        h.app.session.solidbom.result = ResultState::Stale;
        h.frames(3);
        assert!(h.widget("The prior result is stale because the inputs changed. Recalculate to show a current result.").is_some());
        assert!(h.widget("Recalculate").is_some(), "the stale result has an explicit recalculation path");
    }

    #[test]
    fn an_invalid_cavity_count_focuses_the_cycle_section_and_keeps_the_draft() {
        let mut h = crate::scenario::Harness::new();
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/solidbom/fixtures/revisions_bc.step");
        h.app.run("solidbom.import", json!({"path": path.to_string_lossy()})).unwrap();
        h.settle();
        let id = h.app.session.solidbom.draft.as_ref().unwrap().parts[0].part_version_id;
        let mut panel = Panel { part: Some(id), ..Default::default() };
        load(&h.app, &mut panel, id);
        panel.fields.cavities = "0".into();
        h.app.solidbom = Some(panel);
        let error = h.app.run("view.bom.apply", json!({})).unwrap_err();
        assert!(!error.is_empty());
        let panel = h.app.solidbom.as_ref().expect("the panel");
        assert_eq!(panel.section, "cycle");
        assert!(panel.inspector);
        assert!(panel.focus_cavities);
        assert_eq!(panel.fields.cavities, "0", "the invalid draft stays visible");
    }

    #[test]
    fn switching_parts_keeps_unapplied_classification() {
        let mut h = crate::scenario::Harness::new();
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/solidbom/fixtures/revisions_bc.step");
        h.app.run("solidbom.import", json!({"path":path})).unwrap();
        h.settle();
        let ids: Vec<_> = h.app.session.solidbom.draft.as_ref().unwrap().parts.iter().map(|p| p.part_version_id).collect();
        let mut panel = Panel { part: Some(ids[0]), ..Default::default() };
        load(&h.app, &mut panel, ids[0]);
        panel.fields.classification = "Retained draft".into();
        load(&h.app, &mut panel, ids[1]);
        load(&h.app, &mut panel, ids[0]);
        assert_eq!(panel.fields.classification, "Retained draft");
    }
}
