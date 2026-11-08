//! The SolidBOM review window: engineering review, manufacturing inputs and cost explanation.
//!
//! The panel is thin: it reads the session's review state and acts through the SolidBOM commands.
//! It never talks to the service and never does cost arithmetic.

use std::collections::BTreeMap;

use egui::RichText;
use serde_json::{Value, json};
use solidbom_contract::{Sourcing, Unit};
use solidcraft_engine::solidbom::state::{Phase, ResultState, SaveState};
use uuid::Uuid;

use crate::SolveApp;
use crate::theme::Tokens;
use crate::ui_kit;

#[derive(Debug, Clone)]
pub struct Panel {
    pub(super) part: Option<Uuid>,
    fields: Fields,
    baseline: Fields,
    loaded: Option<(Uuid, u64)>,
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
        if let Some((part, generation)) = self.loaded {
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

    let (phase, save, result, source, project, version, definitions, occurrences, issues, needs_service, error, notice) = {
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
    if let Some(error) = &error {
        ui_kit::status_line(ui, error, ui_kit::StatusTone::Error);
    }
    if let Some(notice) = &notice {
        ui_kit::status_line(ui, notice, ui_kit::StatusTone::Dim);
    }
    if !issues.is_empty() {
        ui.label(RichText::new(format!("{}: {}", crate::i18n::tr("solidbom.issues"), issues.join(", "))).color(Tokens::get().warning));
    }

    let Some(part_id) = panel.part else {
        ui_kit::status_line(ui, crate::i18n::tr("solidbom.select_part"), ui_kit::StatusTone::Dim);
        return;
    };
    let generation = app.session.solidbom.generation;
    if panel.loaded != Some((part_id, generation)) && panel.fields == panel.baseline {
        load(app, panel, part_id);
    }
    let Some(draft) = app.session.solidbom.draft.clone() else { return };
    let Some(part) = draft.parts.iter().find(|part| part.part_version_id == part_id) else { return };

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
        ui.label(RichText::new(crate::i18n::tr("solidbom.prototype_label")).strong());
        if let Some(snapshot) = &state.current {
            if let Some(cost) = snapshot.outputs.parts.iter().find(|cost| cost.part_version_id == part_id) {
                for line in &cost.explanations {
                    if let Some(amount) = line.amount {
                        ui.label(format!("{}: {amount}", line.label));
                    }
                }
            }
            if let Some(total) = snapshot.outputs.assembly_subtotal {
                ui.label(format!("{}: {total}", crate::i18n::tr("solidbom.workspace.assembly_total")));
            }
            if let Some(setup) = snapshot.outputs.setup_total {
                ui.label(format!("{}: {setup}", crate::i18n::tr("solidbom.workspace.setup_total")));
            }
            for issue in &snapshot.issues {
                ui.label(format!("{} ({})", issue.field_path, issue.code));
            }
        } else if matches!(state.result, ResultState::Incomplete) {
            ui_kit::status_line(ui, crate::i18n::tr("solidbom.incomplete"), ui_kit::StatusTone::Dim);
        } else {
            ui_kit::status_line(ui, crate::i18n::tr("solidbom.no_result"), ui_kit::StatusTone::Dim);
        }
        if matches!(state.result, ResultState::Stale) {
            ui_kit::status_line(ui, crate::i18n::tr("solidbom.stale"), ui_kit::StatusTone::Warning);
        }
        ui_kit::status_line(ui, crate::i18n::tr("solidbom.omitted"), ui_kit::StatusTone::Dim);
    });

    ui.separator();
    ui.horizontal_wrapped(|ui| {
        let command_button = |ui: &mut egui::Ui, id: &str, label: &str| {
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
        };
        if command_button(ui, "solidbom.save", crate::i18n::tr("solidbom.save")) {
            panel.action = Some("solidbom.save".into());
        }
        if command_button(ui, "solidbom.calculate", crate::i18n::tr("solidbom.calculate")) {
            panel.action = Some("solidbom.calculate".into());
        }
        let retryable = app.session.solidbom.failed.as_ref().is_some_and(|failed| failed.retryable);
        if retryable && command_button(ui, "solidbom.retry", crate::i18n::tr("solidbom.retry")) {
            panel.action = Some("solidbom.retry".into());
        }
    });
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

fn load(app: &SolveApp, panel: &mut Panel, part_id: Uuid) {
    let Some(draft) = &app.session.solidbom.draft else { return };
    let Some(part) = draft.parts.iter().find(|part| part.part_version_id == part_id) else { return };
    panel.stash();
    if let Some((fields, baseline, generation)) = panel.drafts.get(&part_id)
        && fields != baseline
    {
        panel.fields = fields.clone();
        panel.baseline = baseline.clone();
        panel.loaded = Some((part_id, *generation));
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
    panel.loaded = Some((part_id, app.session.solidbom.generation));
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
                panel.loaded = panel.part.map(|id| (id, app.session.solidbom.generation));
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
