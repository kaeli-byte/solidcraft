//! The SolidBOM review window: engineering review, manufacturing inputs and cost explanation.
//!
//! The panel is thin: it reads the session's review state and acts through the SolidBOM commands.
//! It never talks to the service and never does cost arithmetic.

use egui::RichText;
use serde_json::{Value, json};
use solidbom_contract::{Sourcing, Unit};
use solidcraft_engine::solidbom::state::{Phase, ResultState, SaveState};
use uuid::Uuid;

use crate::SolveApp;
use crate::theme::Tokens;
use crate::ui_kit;

#[derive(Debug, Default)]
pub struct Panel {
    part: Option<Uuid>,
    fields: Fields,
    loaded: Option<Uuid>,
    error: Option<String>,
}

#[derive(Debug, Default, Clone)]
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
    overhead: String,
}

/// Open the window.
pub fn open(app: &mut SolveApp) {
    app.solidbom = Some(Panel::default());
}

/// Draw the window; it stays open until the user closes it.
pub fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(mut panel) = app.solidbom.take() else { return };
    let title = crate::i18n::tr("solidbom.title").to_string();
    let closed = ui_kit::Window::new(egui::Id::unique("sc_solidbom"), "bom", &title).cards(3.0).show_body(ctx, |ui| {
        draw(app, &mut panel, ui);
    });
    if !closed {
        app.solidbom = Some(panel);
    }
}

fn draw(app: &mut SolveApp, panel: &mut Panel, ui: &mut egui::Ui) {
    let _language = crate::i18n::scope(&app.preferences.language);
    sync_selection(app, panel);

    let (phase, save, result, source, definitions, occurrences, issues, needs_service, error, notice) = {
        let state = &app.session.solidbom;
        (
            state.phase,
            state.save.clone(),
            state.result.clone(),
            state.draft.as_ref().map(|draft| draft.source.original_name.clone()),
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
    });
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
    if panel.loaded != Some(part_id) {
        load(app, panel, part_id);
    }
    let Some(draft) = app.session.solidbom.draft.clone() else { return };
    let Some(part) = draft.parts.iter().find(|part| part.part_version_id == part_id) else { return };

    // Engineering review.
    let engineering = crate::i18n::tr("solidbom.engineering").to_string();
    let mut apply = false;
    ui_kit::section(ui, "solidbom_engineering", &engineering, true, |ui| {
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
                solidbom_contract::MeasurementSource::Manual => crate::i18n::tr("solidbom.manual_reason"),
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
    });

    // Manufacturing inputs.
    let manufacturing = crate::i18n::tr("solidbom.manufacturing").to_string();
    ui_kit::section(ui, "solidbom_manufacturing", &manufacturing, true, |ui| {
        ui_kit::expr_row(ui, crate::i18n::tr("solidbom.allowance"), &mut panel.fields.allowance);
        ui_kit::expr_row(ui, crate::i18n::tr("solidbom.cycle"), &mut panel.fields.cycle);
        ui_kit::expr_row(ui, crate::i18n::tr("solidbom.cavities"), &mut panel.fields.cavities);
        rate_choice(app, ui, panel, true);
        ui_kit::expr_row(ui, crate::i18n::tr("solidbom.setup"), &mut panel.fields.setup_hours);
        rate_choice(app, ui, panel, false);
        ui_kit::expr_row(ui, crate::i18n::tr("solidbom.overhead"), &mut panel.fields.overhead);
    });
    if ui_kit::button(ui, crate::i18n::tr("solidbom.apply"), ui_kit::Button::Normal).clicked() {
        apply = true;
    }

    // Cost explanation.
    let cost = crate::i18n::tr("solidbom.cost").to_string();
    ui_kit::section(ui, "solidbom_cost", &cost, true, |ui| {
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
                ui.label(format!("{}: {total}", crate::i18n::tr("solidbom.cost")));
            }
            if let Some(setup) = snapshot.outputs.setup_total {
                ui.label(format!("{}: {setup}", crate::i18n::tr("solidbom.setup")));
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
        if ui_kit::button(
            ui,
            crate::i18n::tr("solidbom.save"),
            ui_kit::Button::Enabled(matches!(app.session.solidbom.save, SaveState::Dirty | SaveState::SaveFailed { .. })),
        )
        .clicked()
        {
            let _ = app.run("solidbom.save", json!({}));
        }
        if ui_kit::button(ui, crate::i18n::tr("solidbom.calculate"), ui_kit::Button::Normal).clicked() {
            let _ = app.run("solidbom.calculate", json!({}));
        }
        if app.session.solidbom.failed.is_some() && ui_kit::button(ui, crate::i18n::tr("solidbom.retry"), ui_kit::Button::Normal).clicked() {
            let _ = app.run("solidbom.retry", json!({}));
        }
    });
    if let Some(error) = &panel.error {
        ui_kit::status_line(ui, error, ui_kit::StatusTone::Error);
    }

    if apply {
        let params = params(app, panel);
        match app.run("solidbom.inputs", params) {
            Ok(_) => panel.error = None,
            Err(error) => panel.error = Some(error),
        }
    }
}

fn sync_selection(app: &mut SolveApp, panel: &mut Panel) {
    let Some(draft) = app.session.solidbom.draft.clone() else { return };
    let selected =
        app.session.selection.iter().find_map(|selection| solidcraft_engine::solidbom::resolve_pick(&app.session, &draft.analysis, selection).ok());
    let part = selected
        .and_then(|occurrence| draft.parts.iter().find(|part| part.definition_id == occurrence.definition_id).map(|part| part.part_version_id))
        .or_else(|| draft.parts.first().map(|part| part.part_version_id));
    if let Some(part) = part
        && panel.part != Some(part)
    {
        panel.part = Some(part);
        panel.loaded = None;
    }
}

fn load(app: &SolveApp, panel: &mut Panel, part_id: Uuid) {
    let Some(draft) = &app.session.solidbom.draft else { return };
    let Some(part) = draft.parts.iter().find(|part| part.part_version_id == part_id) else { return };
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
        overhead: draft.material_overhead_rate.map(|value| value.to_string()).unwrap_or_default(),
    };
    panel.loaded = Some(part_id);
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

fn params(app: &SolveApp, panel: &Panel) -> Value {
    let material = app
        .session
        .solidbom
        .reference
        .as_ref()
        .and_then(|reference| reference.materials.get(panel.fields.material.checked_sub(1).unwrap_or(usize::MAX)))
        .map(|material| material.material_version_id.to_string())
        .unwrap_or_default();
    let production_rate = app
        .session
        .solidbom
        .reference
        .as_ref()
        .and_then(|reference| reference.rates.get(panel.fields.production_rate.checked_sub(1).unwrap_or(usize::MAX)))
        .map(|rate| rate.rate_version_id.to_string())
        .unwrap_or_default();
    let setup_rate = app
        .session
        .solidbom
        .reference
        .as_ref()
        .and_then(|reference| reference.rates.get(panel.fields.setup_rate.checked_sub(1).unwrap_or(usize::MAX)))
        .map(|rate| rate.rate_version_id.to_string())
        .unwrap_or_default();
    json!({
        "part_version_id": panel.part.map(|id| id.to_string()).unwrap_or_default(),
        "classification": panel.fields.classification,
        "classification_confirmed": panel.fields.confirmed,
        "sourcing": if panel.fields.manufactured { "manufactured" } else { "purchased" },
        "volume_mm3": panel.fields.volume,
        "manual_reason": (!panel.fields.manual_reason.trim().is_empty()).then_some(panel.fields.manual_reason.clone()),
        "volume_confirmed": panel.fields.volume_confirmed,
        "material_version_id": material,
        "allowance_rate": panel.fields.allowance,
        "cycle_seconds": panel.fields.cycle,
        "cavities": panel.fields.cavities.parse::<u32>().ok(),
        "production_rate_version_id": production_rate,
        "setup_hours": panel.fields.setup_hours,
        "setup_rate_version_id": setup_rate,
        "material_overhead_rate": panel.fields.overhead,
    })
}

fn phase_label(phase: Phase) -> &'static str {
    match phase {
        Phase::Idle => "Idle",
        Phase::Importing => "Importing",
        Phase::Reviewing => "Reviewing",
    }
}

fn save_label(save: &SaveState) -> &'static str {
    match save {
        SaveState::Clean => "Saved",
        SaveState::Dirty => "Unsaved",
        SaveState::Saving { .. } => "Saving",
        SaveState::SaveFailed { .. } => "Save failed",
    }
}

fn result_label(result: &ResultState) -> &'static str {
    match result {
        ResultState::NoResult => "No result",
        ResultState::Calculating { .. } => "Calculating",
        ResultState::Current => "Current",
        ResultState::Stale => "Stale",
        ResultState::Incomplete => "Incomplete",
    }
}
