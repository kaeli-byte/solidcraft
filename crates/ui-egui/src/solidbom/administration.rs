//! The shared-data administration surface: the version chains of materials, cost centers and
//! catalog components, the newest reference-change facts and the publish forms.
//!
//! The surface is only offered to a loaded Administrator identity, but that is not the check: the
//! service refuses every shared route for an identity without the permission set. The surface
//! reads the engine state and acts through the SolidBOM commands, exactly as the review panel
//! does, so a programmatic caller and a person use the same path.

use std::collections::BTreeMap;

use egui::RichText;
use serde_json::{Value, json};
use solidbom_contract::{CatalogComponentVersion, CostCenterPurpose, MaterialVersion, RateVersion, SharedDataView};
use uuid::Uuid;

use solidcraft_engine::solidbom::state::State;

use super::panel::Panel;
use crate::SolveApp;
use crate::ui_kit::{self, Button, StatusTone};

/// The draft fields of the two publish forms. Every value is sent as written; the engine parses
/// the exact decimal strings and refuses anything else as bad parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct AdminForm {
    /// 0 creates a new material record; n publishes over the nth existing record.
    pub material_record: usize,
    /// The record the material fields were last filled from, so a person's edits are not lost.
    material_loaded: Option<usize>,
    pub material_name: String,
    pub material_density: String,
    pub material_price: String,
    pub material_source: String,
    /// 0 creates a new cost center; n publishes over the nth existing record.
    pub rate_record: usize,
    rate_loaded: Option<usize>,
    pub rate_name: String,
    /// 0 is production, 1 is setup.
    pub rate_purpose: usize,
    pub rate_per_hour: String,
    pub rate_source: String,
    /// The form whose publish request is in flight. Its fields clear only after the service
    /// confirms that request; a refused or failed publish keeps them editable.
    awaiting: Option<AwaitingForm>,
    /// The exact material fields submitted in flight. A successful completion clears the form
    /// only while this snapshot still equals the current contents, so edits started for another
    /// record while the request runs are never discarded.
    submitted_material: Option<MaterialDraft>,
    submitted_rate: Option<RateDraft>,
}

/// One material form as it was when its publish request was sent.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MaterialDraft {
    record: usize,
    name: String,
    density: String,
    price: String,
    source: String,
}

/// One cost-center form as it was when its publish request was sent.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RateDraft {
    record: usize,
    name: String,
    purpose: usize,
    rate: String,
    source: String,
}

/// Which publish form is waiting for its request to finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AwaitingForm {
    Material,
    Rate,
}

impl Default for AdminForm {
    fn default() -> Self {
        Self {
            material_record: 0,
            material_loaded: Some(0),
            material_name: String::new(),
            material_density: String::new(),
            material_price: String::new(),
            material_source: String::new(),
            rate_record: 0,
            rate_loaded: Some(0),
            rate_name: String::new(),
            rate_purpose: 0,
            rate_per_hour: String::new(),
            rate_source: String::new(),
            awaiting: None,
            submitted_material: None,
            submitted_rate: None,
        }
    }
}

impl AdminForm {
    /// The publish parameters for the material form. A blank record publishes a new record; an
    /// existing one names its latest version as the version it replaces.
    pub(super) fn material_params(&self, shared: Option<&SharedDataView>) -> Value {
        let records = material_records(shared);
        let (id, expected_version) = match records.get(self.material_record.checked_sub(1).unwrap_or(usize::MAX)) {
            Some(record) => (record.id.to_string(), version_of(record.latest(), |version| version.version)),
            None => (String::new(), 0),
        };
        json!({
            "material_id": id,
            "expected_version": expected_version,
            "name": self.material_name,
            "density_g_cm3": self.material_density,
            "price_per_kg": self.material_price,
            "source_reference": self.material_source,
        })
    }

    /// The publish parameters for the cost-center form.
    pub(super) fn rate_params(&self, shared: Option<&SharedDataView>) -> Value {
        let records = rate_records(shared);
        let (id, expected_version) = match records.get(self.rate_record.checked_sub(1).unwrap_or(usize::MAX)) {
            Some(record) => (record.id.to_string(), version_of(record.latest(), |version| version.version)),
            None => (String::new(), 0),
        };
        json!({
            "cost_center_id": id,
            "expected_version": expected_version,
            "name": self.rate_name,
            "purpose": if self.rate_purpose == 1 { "setup" } else { "production" },
            "rate_per_hour": self.rate_per_hour,
            "source_reference": self.rate_source,
        })
    }

    /// Mark the form whose publish request was just submitted and remember the exact draft it
    /// sent. The fields stay until the completion arrives, so a refused or failed request keeps
    /// the person's draft.
    pub(super) fn await_publish(&mut self, material: bool) {
        self.awaiting = Some(if material { AwaitingForm::Material } else { AwaitingForm::Rate });
        if material {
            self.submitted_material = Some(self.material_draft());
        } else {
            self.submitted_rate = Some(self.rate_draft());
        }
    }

    /// Resolve the awaited publish once its request has finished: clear only the submitted form
    /// after a success, and keep both forms' fields on a failure. An unrelated error or a still
    /// running request never clears anything.
    pub(super) fn resolve_awaiting(&mut self, state: &State) {
        let Some(awaiting) = self.awaiting else { return };
        if state.publishing() {
            return;
        }
        if state.failed.is_some() || state.error.is_some() {
            self.awaiting = None;
            self.submitted_material = None;
            self.submitted_rate = None;
            return;
        }
        // Only a form whose contents still match what was submitted clears: a person may have
        // selected another record or edited a new draft while the first request ran.
        self.awaiting = None;
        match awaiting {
            AwaitingForm::Material => {
                if self.submitted_material.take().is_some_and(|submitted| submitted == self.material_draft()) {
                    self.clear_material();
                }
            }
            AwaitingForm::Rate => {
                if self.submitted_rate.take().is_some_and(|submitted| submitted == self.rate_draft()) {
                    self.clear_rate();
                }
            }
        }
    }

    fn material_draft(&self) -> MaterialDraft {
        MaterialDraft {
            record: self.material_record,
            name: self.material_name.clone(),
            density: self.material_density.clone(),
            price: self.material_price.clone(),
            source: self.material_source.clone(),
        }
    }

    fn rate_draft(&self) -> RateDraft {
        RateDraft {
            record: self.rate_record,
            name: self.rate_name.clone(),
            purpose: self.rate_purpose,
            rate: self.rate_per_hour.clone(),
            source: self.rate_source.clone(),
        }
    }

    /// Start the material form over without touching the cost-center form's unsent edits.
    fn clear_material(&mut self) {
        self.material_record = 0;
        self.material_loaded = Some(0);
        self.material_name.clear();
        self.material_density.clear();
        self.material_price.clear();
        self.material_source.clear();
        self.submitted_material = None;
    }

    /// Start the cost-center form over without touching the material form's unsent edits.
    fn clear_rate(&mut self) {
        self.rate_record = 0;
        self.rate_loaded = Some(0);
        self.rate_name.clear();
        self.rate_purpose = 0;
        self.rate_per_hour.clear();
        self.rate_source.clear();
        self.submitted_rate = None;
    }
}

/// One logical shared record with its append-only versions, oldest first.
struct Record<'a, T> {
    id: Uuid,
    versions: Vec<&'a T>,
}

impl<'a, T> Record<'a, T> {
    fn latest(&self) -> Option<&'a T> {
        self.versions.last().copied()
    }
}

fn version_of<T>(latest: Option<&T>, version: impl Fn(&T) -> u64) -> u64 {
    latest.map(version).unwrap_or(0)
}

fn material_records(shared: Option<&SharedDataView>) -> Vec<Record<'_, MaterialVersion>> {
    let Some(shared) = shared else { return Vec::new() };
    let mut grouped: BTreeMap<Uuid, Vec<&MaterialVersion>> = BTreeMap::new();
    for version in &shared.materials {
        grouped.entry(version.material_id).or_default().push(version);
    }
    ordered(grouped, |version| version.version, |version| version.name.as_str())
}

fn rate_records(shared: Option<&SharedDataView>) -> Vec<Record<'_, RateVersion>> {
    let Some(shared) = shared else { return Vec::new() };
    let mut grouped: BTreeMap<Uuid, Vec<&RateVersion>> = BTreeMap::new();
    for version in &shared.cost_centers {
        grouped.entry(version.cost_center_id).or_default().push(version);
    }
    ordered(grouped, |version| version.version, |version| version.name.as_str())
}

/// Oldest version first per record, and the records in the name order of their newest version.
fn ordered<'a, T, V, N>(grouped: BTreeMap<Uuid, Vec<&'a T>>, version: V, name: N) -> Vec<Record<'a, T>>
where
    V: Fn(&T) -> u64,
    N: Fn(&'a T) -> &'a str,
{
    let mut records: Vec<Record<'a, T>> = grouped
        .into_iter()
        .map(|(id, mut versions)| {
            versions.sort_by_key(|version_entry| version(version_entry));
            Record { id, versions }
        })
        .collect();
    records.sort_by(|left, right| left.latest().map(&name).cmp(&right.latest().map(&name)));
    records
}

/// Draw the administration surface. It is rendered only while the loaded identity holds the
/// Administrator permission set; an Engineer-only identity sees no administration controls.
pub(super) fn draw(app: &SolveApp, panel: &mut Panel, ui: &mut egui::Ui) {
    let _language = crate::i18n::scope(&app.preferences.language);
    // A publish completion clears only the form it came from, and only on success.
    panel.admin.resolve_awaiting(&app.session.solidbom);
    let state = &app.session.solidbom;
    if !state.is_administrator() {
        ui_kit::status_line(ui, crate::i18n::tr("solidbom.shared.requires_admin"), StatusTone::Warning);
        return;
    }
    ui.heading(crate::i18n::tr("solidbom.shared.title"));
    if let Some(identity) = &state.identity {
        let permissions: Vec<&str> = identity.permissions.iter().map(|permission| permission.as_str()).collect();
        ui.label(
            RichText::new(crate::i18n::format("solidbom.shared.identity", &[("name", &identity.name), ("permissions", &permissions.join(", "))]))
                .strong(),
        );
    }
    ui.horizontal_wrapped(|ui| {
        if ui_kit::button(ui, crate::i18n::tr("solidbom.shared.reload"), Button::Normal).clicked() {
            panel.action = Some("solidbom.shared".into());
        }
        if ui_kit::button(ui, crate::i18n::tr("solidbom.shared.back"), Button::Normal).clicked() {
            panel.section = "engineering".into();
        }
        if state.publishing() {
            ui.label(RichText::new(crate::i18n::tr("solidbom.shared.publishing")).small());
        }
    });
    if let Some(error) = &state.error {
        ui_kit::status_line(ui, error, StatusTone::Error);
    }
    if let Some(notice) = &state.notice {
        ui_kit::status_line(ui, notice, StatusTone::Dim);
    }
    let Some(shared) = &state.shared else {
        ui_kit::status_line(ui, crate::i18n::tr("solidbom.shared.loading"), StatusTone::Dim);
        return;
    };

    let publishing = state.publishing();
    version_chains(ui, &shared.materials.iter().map(material_line).collect::<Vec<_>>(), "solidbom.shared.materials", "solidbom.shared.no_records");
    version_chains(ui, &shared.cost_centers.iter().map(rate_line).collect::<Vec<_>>(), "solidbom.shared.cost_centers", "solidbom.shared.no_records");
    version_chains(ui, &shared.components.iter().map(component_line).collect::<Vec<_>>(), "solidbom.shared.components", "solidbom.shared.no_records");
    app_config(ui, shared);
    reference_changes(ui, shared);

    ui.separator();
    material_form(ui, panel, shared, publishing);
    ui.separator();
    rate_form(ui, panel, shared, publishing);
}

/// One line per published version, grouped per logical record.
fn version_chains(ui: &mut egui::Ui, lines: &[(String, Vec<String>)], title: &str, empty: &str) {
    let title = crate::i18n::tr(title).to_string();
    ui_kit::section(ui, ("solidbom_shared", title.clone()), &title, true, |ui| {
        let mut any = false;
        for (record, versions) in lines {
            ui.label(RichText::new(record).strong());
            for version in versions {
                ui.label(version);
            }
            any = true;
        }
        if !any {
            ui.label(crate::i18n::tr(empty));
        }
    });
}

fn material_line(version: &MaterialVersion) -> (String, Vec<String>) {
    let record = crate::i18n::format("solidbom.shared.material_record", &[("name", &version.name), ("version", &version.version.to_string())]);
    let price = match version.price_per_kg {
        Some(price) => crate::i18n::format("solidbom.shared.price_value", &[("amount", &price.to_string()), ("currency", &version.currency)]),
        None => crate::i18n::tr("solidbom.shared.missing_price").to_string(),
    };
    let density = crate::i18n::format("solidbom.shared.density_value", &[("amount", &version.density_g_cm3.to_string())]);
    let mut line = format!("{price} · {density}");
    if let Some(source) = &version.source_reference {
        line.push_str(&crate::i18n::format("solidbom.shared.source_value", &[("source", source)]));
    }
    (record, vec![line])
}

fn rate_line(version: &RateVersion) -> (String, Vec<String>) {
    let purpose = match version.purpose {
        CostCenterPurpose::Production => crate::i18n::tr("solidbom.shared.purpose.production"),
        CostCenterPurpose::Setup => crate::i18n::tr("solidbom.shared.purpose.setup"),
    };
    let record = crate::i18n::format(
        "solidbom.shared.cost_center_record",
        &[("name", &version.name), ("version", &version.version.to_string()), ("purpose", purpose)],
    );
    let rate = match version.rate_per_hour {
        Some(rate) => crate::i18n::format("solidbom.shared.rate_value", &[("amount", &rate.to_string()), ("currency", &version.currency)]),
        None => crate::i18n::tr("solidbom.shared.missing_rate").to_string(),
    };
    let mut line = rate;
    if let Some(source) = &version.source_reference {
        line.push_str(&crate::i18n::format("solidbom.shared.source_value", &[("source", source)]));
    }
    (record, vec![line])
}

fn component_line(version: &CatalogComponentVersion) -> (String, Vec<String>) {
    let record = crate::i18n::format("solidbom.shared.component_record", &[("name", &version.name), ("version", &version.version.to_string())]);
    let price = match version.price {
        Some(price) => crate::i18n::format("solidbom.shared.price_value", &[("amount", &price.to_string()), ("currency", &version.currency)]),
        None => crate::i18n::tr("solidbom.shared.missing_price").to_string(),
    };
    let specification = crate::i18n::format("solidbom.shared.specification_value", &[("specification", &version.specification)]);
    let mut line = format!("{price} · {specification}");
    if let Some(source) = &version.source_reference {
        line.push_str(&crate::i18n::format("solidbom.shared.source_value", &[("source", source)]));
    }
    (record, vec![line])
}

fn app_config(ui: &mut egui::Ui, shared: &SharedDataView) {
    let title = crate::i18n::tr("solidbom.shared.app_config").to_string();
    ui_kit::section(ui, ("solidbom_shared", title.clone()), &title, true, |ui| {
        let Some(config) = shared.app_config.as_ref() else {
            ui.label(crate::i18n::tr("solidbom.shared.no_app_config"));
            return;
        };
        let count = config.categories.len().to_string();
        let revisions = shared.app_config_versions.len().max(1).to_string();
        ui.label(
            RichText::new(crate::i18n::format(
                "solidbom.shared.app_config_line",
                &[("version", &config.version.to_string()), ("categories", &count), ("currency", &config.currency), ("revisions", &revisions)],
            ))
            .strong(),
        );
        for version in &shared.app_config_versions {
            let categories = version.categories.len().to_string();
            ui.label(crate::i18n::format(
                "solidbom.shared.app_config_version_line",
                &[("version", &version.version.to_string()), ("categories", &categories)],
            ));
        }
        let codes: Vec<String> = config.categories.iter().map(|category| format!("{} ({})", category.code, category.name)).collect();
        ui_kit::read_only_text(ui, &codes.join(", "));
    });
}

fn reference_changes(ui: &mut egui::Ui, shared: &SharedDataView) {
    let title = crate::i18n::tr("solidbom.shared.changes").to_string();
    ui_kit::section(ui, ("solidbom_shared", title.clone()), &title, true, |ui| {
        if shared.changes.is_empty() {
            ui.label(crate::i18n::tr("solidbom.shared.no_changes"));
            return;
        }
        for change in &shared.changes {
            let line = crate::i18n::format(
                "solidbom.shared.change_line",
                &[
                    ("kind", change.kind.as_str()),
                    ("version", &change.version.to_string()),
                    ("summary", &change.summary),
                    ("changed_by", &change.changed_by),
                    ("changed_at", &change.changed_at),
                ],
            );
            ui.label(line);
        }
    });
}

fn material_form(ui: &mut egui::Ui, panel: &mut Panel, shared: &SharedDataView, publishing: bool) {
    let records = material_records(Some(shared));
    let mut labels: Vec<String> = vec![crate::i18n::tr("solidbom.shared.new_record").to_string()];
    labels.extend(records.iter().map(|record| record.latest().map(|version| version.name.clone()).unwrap_or_default()));
    if panel.admin.material_record > records.len() {
        panel.admin.material_record = 0;
    }
    let borrowed: Vec<&str> = labels.iter().map(String::as_str).collect();
    let title = crate::i18n::tr("solidbom.shared.publish_material").to_string();
    ui_kit::section(ui, ("solidbom_shared", title.clone()), &title, true, |ui| {
        ui_kit::choice_row(
            ui,
            crate::i18n::tr("solidbom.shared.field.record"),
            "solidbom_shared_material_record",
            &mut panel.admin.material_record,
            &borrowed,
        );
        if panel.admin.material_loaded != Some(panel.admin.material_record) {
            prefill_material(&mut panel.admin, &records);
        }
        ui_kit::text_row(ui, crate::i18n::tr("solidbom.shared.field.name"), &mut panel.admin.material_name, "");
        ui_kit::text_row(ui, crate::i18n::tr("solidbom.shared.field.density"), &mut panel.admin.material_density, "");
        ui_kit::text_row(ui, crate::i18n::tr("solidbom.shared.field.price"), &mut panel.admin.material_price, "");
        ui_kit::text_row(ui, crate::i18n::tr("solidbom.shared.field.source"), &mut panel.admin.material_source, "");
        let ready = !panel.admin.material_name.trim().is_empty() && !panel.admin.material_density.trim().is_empty();
        let publish = ui_kit::button(ui, crate::i18n::tr("solidbom.shared.publish"), Button::Enabled(ready && !publishing));
        if publish.clicked() {
            panel.action = Some("solidbom.shared.material.version".into());
        }
    });
}

fn rate_form(ui: &mut egui::Ui, panel: &mut Panel, shared: &SharedDataView, publishing: bool) {
    let records = rate_records(Some(shared));
    let mut labels: Vec<String> = vec![crate::i18n::tr("solidbom.shared.new_record").to_string()];
    labels.extend(records.iter().map(|record| record.latest().map(|version| version.name.clone()).unwrap_or_default()));
    if panel.admin.rate_record > records.len() {
        panel.admin.rate_record = 0;
    }
    let borrowed: Vec<&str> = labels.iter().map(String::as_str).collect();
    let purposes = [crate::i18n::tr("solidbom.shared.purpose.production").to_string(), crate::i18n::tr("solidbom.shared.purpose.setup").to_string()];
    let purpose_refs: Vec<&str> = purposes.iter().map(String::as_str).collect();
    let title = crate::i18n::tr("solidbom.shared.publish_rate").to_string();
    ui_kit::section(ui, ("solidbom_shared", title.clone()), &title, true, |ui| {
        ui_kit::choice_row(
            ui,
            crate::i18n::tr("solidbom.shared.field.record"),
            "solidbom_shared_rate_record",
            &mut panel.admin.rate_record,
            &borrowed,
        );
        if panel.admin.rate_loaded != Some(panel.admin.rate_record) {
            prefill_rate(&mut panel.admin, &records);
        }
        ui_kit::text_row(ui, crate::i18n::tr("solidbom.shared.field.name"), &mut panel.admin.rate_name, "");
        ui_kit::choice_row(
            ui,
            crate::i18n::tr("solidbom.shared.field.purpose"),
            "solidbom_shared_rate_purpose",
            &mut panel.admin.rate_purpose,
            &purpose_refs,
        );
        ui_kit::text_row(ui, crate::i18n::tr("solidbom.shared.field.rate"), &mut panel.admin.rate_per_hour, "");
        ui_kit::text_row(ui, crate::i18n::tr("solidbom.shared.field.source"), &mut panel.admin.rate_source, "");
        let ready = !panel.admin.rate_name.trim().is_empty();
        let publish = ui_kit::button(ui, crate::i18n::tr("solidbom.shared.publish"), Button::Enabled(ready && !publishing));
        if publish.clicked() {
            panel.action = Some("solidbom.shared.cost_center.version".into());
        }
    });
}

fn prefill_material(form: &mut AdminForm, records: &[Record<'_, MaterialVersion>]) {
    form.material_loaded = Some(form.material_record);
    let Some(record) = records.get(form.material_record.checked_sub(1).unwrap_or(usize::MAX)) else {
        form.material_name.clear();
        form.material_density.clear();
        form.material_price.clear();
        form.material_source.clear();
        return;
    };
    let Some(latest) = record.latest() else { return };
    form.material_name.clone_from(&latest.name);
    form.material_density = latest.density_g_cm3.to_string();
    form.material_price = latest.price_per_kg.map(|price| price.to_string()).unwrap_or_default();
    form.material_source = latest.source_reference.clone().unwrap_or_default();
}

fn prefill_rate(form: &mut AdminForm, records: &[Record<'_, RateVersion>]) {
    form.rate_loaded = Some(form.rate_record);
    let Some(record) = records.get(form.rate_record.checked_sub(1).unwrap_or(usize::MAX)) else {
        form.rate_name.clear();
        form.rate_per_hour.clear();
        form.rate_source.clear();
        form.rate_purpose = 0;
        return;
    };
    let Some(latest) = record.latest() else { return };
    form.rate_name.clone_from(&latest.name);
    form.rate_purpose = usize::from(latest.purpose == CostCenterPurpose::Setup);
    form.rate_per_hour = latest.rate_per_hour.map(|rate| rate.to_string()).unwrap_or_default();
    form.rate_source = latest.source_reference.clone().unwrap_or_default();
}
