//! BOM workspace content; the shared shell owns every outer host and camera gesture.

use egui::RichText;
use serde_json::{Value, json};
use uuid::Uuid;

use super::panel::{self, Panel};
use crate::SolveApp;
use crate::theme::Tokens;
use crate::ui_kit::{self, Container, Edge, Surface};
use solidcraft_engine::solidbom::state::Phase;

pub(super) fn rows(app: &SolveApp) -> Vec<Value> {
    app.session
        .solidbom_parts()
        .ok()
        .and_then(|reply| serde_json::to_value(reply.rows).ok())
        .and_then(|rows| rows.as_array().cloned())
        .unwrap_or_default()
}

fn cached_rows(app: &SolveApp, panel: &mut Panel) -> Vec<Value> {
    let state = &app.session.solidbom;
    let key = format!(
        "{}:{}:{:?}:{:?}:{:?}",
        state.generation,
        state.version(),
        state.project_id(),
        state.result,
        (state.current.as_ref().map(|snapshot| snapshot.calculation_id), state.reference.as_ref().map(|r| (r.materials.len(), r.rates.len())))
    );
    if panel.projection.as_ref().is_none_or(|(previous, _)| previous != &key) {
        panel.projection = Some((key, rows(app)));
    }
    panel.projection.as_ref().map(|(_, rows)| rows.clone()).unwrap_or_default()
}

pub(super) fn select_row(app: &mut SolveApp, panel: &mut Panel, row: &Value) {
    let Some(part) = row.get("part_version_id").and_then(Value::as_str).and_then(|id| Uuid::parse_str(id).ok()) else { return };
    panel::select_part(app, panel, part);
    let items: Vec<_> =
        row.get("occurrence_paths").and_then(Value::as_array).into_iter().flatten().map(|path| json!({"type":"occurrence","path":path})).collect();
    if !items.is_empty()
        && let Err(error) = app.run("select.set", json!({"items":items}))
    {
        panel.error = Some(error);
    }
}

pub(crate) fn show(app: &mut SolveApp, ui: &mut egui::Ui) {
    crate::panel_runtime::hide_app_host(app);
    crate::status_bar::show(app, ui);
    crate::toolbar::toolbar(app, ui);
    let mut panel = app.solidbom.take().unwrap_or_default();
    panel::sync_selection(app, &mut panel);
    let rows = cached_rows(app, &mut panel);
    let t = Tokens::get();
    let host = Container::new(ui.ctx(), Surface::Panel);
    let document = app.session.uid;
    // A narrow window drops the inspector before it can squeeze the model to nothing. The
    // person's own choice stays in `Panel`, so the pane returns when the window widens again.
    let browser = if app.ui.show_browser { crate::browser::browser_width(app.preferences.browser_width) } else { 0.0 };
    let narrow = ui.available_width() - browser < t.form_card_width * 1.8;
    if panel.inspector && !narrow && app.session.solidbom.draft.is_some() {
        let title = crate::i18n::tr("solidbom.workspace.inspector").to_string();
        let right = host
            .edge(format!("bom_inspector:{document}:{}", panel.layout_generation), Edge::Right)
            .resizable(true)
            .default_size(t.form_card_width.min(ui.available_width() * 0.45))
            .min_size((t.form_card_width * 0.8).min(ui.available_width() * 0.45))
            .max_size(ui.available_width() * 0.6);
        let response = host.show_edge(ui, right, host.edge_frame(t.panel), |ui| {
            ui_kit::header(ui, "bom", &title, || panel.inspector = false);
            egui::ScrollArea::vertical().id_salt(format!("bom_inspector_body:{document}")).show(ui, |ui| {
                if matches!(panel.section.as_str(), "project" | "calculation") {
                    details(app, &panel, ui);
                } else if app.session.solidbom.draft.is_some() {
                    panel::draw(app, &mut panel, ui);
                } else {
                    ui.label(crate::i18n::tr("solidbom.workspace.no_project"));
                }
            });
        });
        crate::shell::register_panel(app, "bom:inspector", Some(response.response.rect), Some(Edge::Right));
        crate::scenario::publish_rect("bom:inspector", response.response.rect);
    } else {
        crate::shell::register_panel(app, "bom:inspector", None, None);
    }
    if panel.table && app.session.solidbom.draft.is_some() {
        let available = ui.available_height();
        let frame = host.edge_frame(t.panel);
        let margins = frame.inner_margin.sum().y + frame.outer_margin.sum().y;
        let bottom = host
            .edge(format!("bom_table:{document}:{}", panel.layout_generation), Edge::Bottom)
            .resizable(true)
            .default_size(available * panel.table_fraction)
            .min_size(t.control_height * 4.0)
            .max_size((available * 0.65).max(t.control_height * 4.0));
        let response = host.show_edge(ui, bottom, frame, |ui| table(app, &mut panel, &rows, ui));
        if available > 0.0 {
            panel.table_fraction = ((response.response.rect.height() - margins) / available).clamp(0.15, 0.65);
        }
        crate::shell::register_panel(app, "bom:table", Some(response.response.rect), Some(Edge::Bottom));
        crate::scenario::publish_rect("bom:table", response.response.rect);
    } else {
        crate::shell::register_panel(app, "bom:table", None, None);
    }
    crate::browser::publish_row_prefs(app, ui.ctx());
    if app.ui.show_browser {
        crate::browser::browser(app, ui);
    } else {
        app.shell.browser = None;
    }
    crate::semantic_content::finalize(app, ui.ctx());
    Container::new(ui.ctx(), Surface::Viewport).central().show(ui, |ui| {
        app.shell.content = Some(ui.max_rect());
        let opening = app.session.solidbom_analysis.is_some() || app.session.solidbom.phase == Phase::Importing;
        if app.session.solidbom.draft.is_none() && app.session.solidbom.pending.is_empty() && !opening {
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.25);
                ui.heading(crate::i18n::tr("solidbom.title"));
                ui.label(crate::i18n::tr("solidbom.workspace.empty"));
                ui.add_space(t.space_12);
                for (key, command) in [("solidbom.workspace.import", "solidbom.import"), ("solidbom.workspace.open", "solidbom.reopen")] {
                    let response = ui_kit::button(ui, crate::i18n::tr(key), ui_kit::Button::Normal);
                    crate::scenario::publish_rect(command, response.rect);
                    if response.clicked() {
                        panel.action = Some(command.into());
                    }
                }
            });
        } else if app.session.solidbom.draft.is_none() {
            // The source is being read and measured beside the window: the frame keeps drawing
            // and the workspace stays open instead of a modal busy state.
            ui.vertical_centered(|ui| {
                ui.add_space(ui.available_height() * 0.25);
                ui.heading(crate::i18n::tr("solidbom.title"));
                ui.label(crate::i18n::tr("solidbom.phase.importing"));
            });
        } else {
            crate::preview::update(app, ui.ctx());
            app.show_workspace(ui);
        }
    });
    let action = panel.action.take();
    app.solidbom = Some(panel);
    if let Some(action) = action {
        app.start(&action);
    }
    crate::browser::editors(app, ui);
    crate::agent_chat::show(app, ui);
}

fn text(row: &Value, key: &str) -> String {
    match row.get(key) {
        Some(Value::String(value)) if !value.is_empty() => value.clone(),
        Some(Value::Number(value)) => value.to_string(),
        _ => crate::i18n::tr("solidbom.workspace.unavailable").into(),
    }
}

fn cells(row: &Value) -> Vec<String> {
    let sourcing = if row.get("sourcing").and_then(Value::as_str) == Some("purchased") { "solidbom.purchased" } else { "solidbom.manufactured" };
    let status = if row.get("needs_review").and_then(Value::as_bool) == Some(true) { "solidbom.pending" } else { "solidbom.reviewed" };
    vec![
        text(row, "part"),
        text(row, "revision"),
        text(row, "quantity"),
        text(row, "classification"),
        crate::i18n::tr(sourcing).into(),
        text(row, "material"),
        text(row, "volume_mm3"),
        crate::i18n::tr(status).into(),
        text(row, "per_part_subtotal"),
    ]
}

fn matches(row: &Value, panel: &Panel) -> bool {
    let filtered = match panel.filter.as_str() {
        "needs_review" => row.get("needs_review").and_then(Value::as_bool) == Some(true),
        "missing_inputs" => row.get("missing_inputs").and_then(Value::as_array).is_some_and(|items| !items.is_empty()),
        "with_issues" => row.get("issues").and_then(Value::as_array).is_some_and(|items| !items.is_empty()),
        _ => true,
    };
    filtered && (panel.query.is_empty() || cells(row).iter().any(|cell| cell.to_lowercase().contains(&panel.query.to_lowercase())))
}

/// The number a sortable value column carries, when it has one. Missing values sort last.
fn numeric(row: &Value, column: usize) -> Option<f64> {
    let key = match column {
        2 => "quantity",
        6 => "volume_mm3",
        8 => "per_part_subtotal",
        _ => return None,
    };
    match row.get(key) {
        Some(Value::Number(value)) => value.as_f64(),
        Some(Value::String(value)) => value.parse().ok(),
        _ => None,
    }
}

fn table(app: &mut SolveApp, panel: &mut Panel, rows: &[Value], ui: &mut egui::Ui) {
    let t = Tokens::get();
    ui.horizontal_wrapped(|ui| {
        ui.strong(crate::i18n::tr("solidbom.workspace.table"));
        ui_kit::text_row(ui, crate::i18n::tr("solidbom.workspace.search"), &mut panel.query, "");
        let filters = ["all", "needs_review", "missing_inputs", "with_issues"];
        let mut picked = filters.iter().position(|filter| *filter == panel.filter).unwrap_or(0);
        let names: Vec<_> = filters.iter().map(|filter| crate::i18n::tr(&format!("solidbom.workspace.{filter}")).to_string()).collect();
        let borrowed: Vec<_> = names.iter().map(String::as_str).collect();
        ui_kit::choice_row(ui, crate::i18n::tr("solidbom.workspace.filter"), "bom_table_filter", &mut picked, &borrowed);
        panel.filter = filters.get(picked).copied().unwrap_or("all").into();
    });
    let mut visible: Vec<_> = rows.iter().filter(|row| matches(row, panel)).collect();
    visible.sort_by(|a, b| {
        let (left, right) = (numeric(a, panel.sort), numeric(b, panel.sort));
        let ordering = match (left, right) {
            (Some(left), Some(right)) => left.partial_cmp(&right).unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) if matches!(panel.sort, 2 | 6 | 8) => std::cmp::Ordering::Equal,
            _ => cells(a).get(panel.sort).cmp(&cells(b).get(panel.sort)),
        };
        let sorted = if panel.descending && (left.is_some() == right.is_some()) { ordering.reverse() } else { ordering };
        sorted.then_with(|| a.get("part_version_id").and_then(Value::as_str).cmp(&b.get("part_version_id").and_then(Value::as_str)))
    });
    let columns = [
        "solidbom.workspace.part",
        "solidbom.workspace.revision",
        "solidbom.workspace.quantity",
        "solidbom.classification",
        "solidbom.workspace.sourcing",
        "solidbom.material",
        "solidbom.volume",
        "solidbom.workspace.status",
        "solidbom.workspace.unit_cost",
    ];
    let width = t.form_card_width * 0.48;
    let total_width = width * columns.len() as f32;
    let height = ui.available_height().max(t.control_height);
    egui::ScrollArea::horizontal().id_salt(format!("bom_columns:{}", app.session.uid)).show(ui, |ui| {
        ui.set_min_width(total_width);
        ui.horizontal(|ui| {
            for (i, key) in columns.iter().enumerate() {
                ui.allocate_ui(egui::vec2(width, t.control_height), |ui| {
                    if ui_kit::button(ui, crate::i18n::tr(key), ui_kit::Button::Normal).clicked() {
                        panel.descending = panel.sort == i && !panel.descending;
                        panel.sort = i;
                    }
                });
            }
        });
        if visible.is_empty() {
            ui.label(crate::i18n::tr("solidbom.workspace.no_rows"));
        }
        let mut scroll = egui::ScrollArea::vertical().id_salt(format!("bom_rows:{}", app.session.uid)).auto_shrink([false, false]);
        if panel.reveal_part {
            if let Some(index) = visible
                .iter()
                .position(|row| row.get("part_version_id").and_then(Value::as_str).and_then(|id| Uuid::parse_str(id).ok()) == panel.part)
            {
                scroll = scroll.vertical_scroll_offset(index as f32 * (t.control_height + ui.spacing().item_spacing.y));
            }
            panel.reveal_part = false;
        }
        scroll.max_height((height - t.control_height * 2.0).max(t.control_height)).show_rows(ui, t.control_height, visible.len(), |ui, range| {
            for row in visible.iter().skip(range.start).take(range.len()) {
                let id = row.get("part_version_id").and_then(Value::as_str).unwrap_or_default();
                let picked = Uuid::parse_str(id).ok() == panel.part;
                ui.push_id(id, |ui| {
                    ui.horizontal(|ui| {
                        for (i, value) in cells(row).iter().enumerate() {
                            let response = ui
                                .allocate_ui(egui::vec2(width, t.control_height), |ui| {
                                    if i == 0 { ui_kit::list_picker(ui, id, value, picked) } else { ui.add(egui::Label::new(value).truncate()) }
                                })
                                .inner;
                            if i == 0 {
                                crate::scenario::publish_target(&format!("bom:part:{id}"), response.rect);
                                if response.clicked() {
                                    select_row(app, panel, row);
                                }
                            }
                        }
                    });
                });
            }
        });
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(
            RichText::new(crate::i18n::format(
                "solidbom.workspace.count",
                &[("shown", visible.len().to_string().as_str()), ("total", rows.len().to_string().as_str())],
            ))
            .small(),
        );
        if let Some(currency) = app.session.solidbom.draft.as_ref().map(|draft| draft.currency.as_str()) {
            ui.label(RichText::new(crate::i18n::format("solidbom.workspace.units", &[("currency", currency)])).small());
        }
    });
}

fn details(app: &SolveApp, panel: &Panel, ui: &mut egui::Ui) {
    let state = &app.session.solidbom;
    if panel.section == "calculation" {
        ui.heading(crate::i18n::tr("solidbom.workspace.calculation"));
        if let Some(snapshot) = &state.current {
            ui_kit::read_only_text(ui, &serde_json::to_string_pretty(snapshot).unwrap_or_default());
        } else {
            ui.label(crate::i18n::tr("solidbom.no_result"));
        }
    } else {
        ui.heading(crate::i18n::tr("solidbom.workspace.project"));
        if let Some(draft) = &state.draft {
            ui.label(&draft.source.original_name);
            ui.label(draft.project_id.to_string());
            ui.label(draft.version.to_string());
            ui_kit::read_only_text(ui, &serde_json::to_string_pretty(&draft.source).unwrap_or_default());
        } else {
            ui.label(crate::i18n::tr("solidbom.workspace.no_project"));
        }
    }
}

pub(super) fn project_dialog(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(mut panel) = app.solidbom.take() else { return };
    let Some(mut id) = panel.project_id.take() else {
        app.solidbom = Some(panel);
        return;
    };
    let mut open = false;
    let mut cancel = false;
    let closed = ui_kit::Window::new(egui::Id::unique("bom_open_project"), "open", crate::i18n::tr("solidbom.workspace.open")).show_body(ctx, |ui| {
        ui.label(crate::i18n::tr("solidbom.workspace.open_help"));
        ui_kit::text_row(ui, crate::i18n::tr("solidbom.workspace.project_id"), &mut id, "");
        if let Some(error) = &panel.error {
            ui_kit::status_line(ui, error, ui_kit::StatusTone::Error);
        }
        ui.horizontal(|ui| {
            open =
                ui_kit::button(ui, crate::i18n::tr("solidbom.workspace.open"), ui_kit::Button::Enabled(Uuid::parse_str(id.trim()).is_ok())).clicked();
            cancel = ui_kit::button(ui, crate::i18n::tr("ui.cancel"), ui_kit::Button::Normal).clicked();
        });
    });
    if !closed && !cancel {
        panel.project_id = Some(id.clone());
    }
    app.solidbom = Some(panel);
    if open {
        if let Some(panel) = app.solidbom.as_mut() {
            panel.project_id = None;
        }
        match panel::open_review_tab(app, "solidbom.reopen", json!({"project_id":id.trim()})) {
            Ok(()) => {
                if let Some(panel) = app.solidbom.as_mut() {
                    panel.project_id = None;
                }
            }
            Err(error) => {
                if let Some(panel) = app.solidbom.as_mut() {
                    panel.error = Some(error);
                    panel.project_id = Some(id);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn imported(fixture: &str) -> crate::scenario::Harness {
        let mut h = crate::scenario::Harness::new();
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/solidbom/fixtures").join(fixture);
        h.app.run("solidbom.import", json!({"path": path.to_string_lossy()})).expect("the import");
        // The analysis runs beside the window; wait for the review it publishes.
        h.settle();
        h.call("ui.resize", json!({"width": 1600, "height": 1000}));
        h.frames(3);
        h
    }

    #[test]
    fn rows_share_definitions_and_missing_values_are_unavailable() {
        let h = imported("three_boxes.step");
        let draft = h.app.session.solidbom.draft.as_ref().expect("the draft");
        let rows = rows(&h.app);
        assert_eq!(rows.len(), draft.parts.len());
        let quantity: u64 = rows.iter().filter_map(|row| row.get("quantity").and_then(Value::as_u64)).sum();
        assert_eq!(quantity as usize, draft.analysis.occurrences.len());
        assert_eq!(text(&json!({"volume_mm3": null}), "volume_mm3"), "Unavailable");
        assert_eq!(text(&json!({"volume_mm3": 12.5}), "volume_mm3"), "12.5");
    }

    #[test]
    fn a_filtered_out_selection_stays_selected_for_the_inspector() {
        let mut h = imported("three_boxes.step");
        let part = h.app.session.solidbom.draft.as_ref().expect("the draft").parts[0].part_version_id;
        let handle = format!("bom:part:{part}");
        assert!(crate::scenario::rect_of(&handle).is_some(), "the table publishes each part row");
        h.step(&json!({"click":{"handle": handle}})).unwrap();
        h.frames(2);
        assert_eq!(h.app.solidbom.as_ref().expect("the panel").part, Some(part));
        h.app.run("view.bom.filter", json!({"filter": "missing_inputs"})).unwrap();
        h.frames(2);
        let panel = h.app.solidbom.as_ref().expect("the panel");
        assert_eq!(panel.filter, "missing_inputs");
        assert_eq!(panel.part, Some(part));
    }

    #[test]
    fn column_headers_sort_and_toggle_descending() {
        let mut h = imported("three_boxes.step");
        h.step(&json!({"widget": "Quantity"})).unwrap();
        h.frames(2);
        assert_eq!(h.app.solidbom.as_ref().expect("the panel").sort, 2);
        h.step(&json!({"widget": "Quantity"})).unwrap();
        h.frames(2);
        assert!(h.app.solidbom.as_ref().expect("the panel").descending);
        assert_eq!(numeric(&json!({"quantity": 2}), 2), Some(2.0));
        assert_eq!(numeric(&json!({"volume_mm3": "12.5"}), 6), Some(12.5));
        assert_eq!(numeric(&json!({"volume_mm3": null}), 6), None);
    }

    #[test]
    fn narrow_windows_collapse_the_inspector_and_keep_the_choice() {
        let mut h = imported("three_boxes.step");
        assert!(crate::scenario::rect_of("bom:inspector").is_some());
        h.call("ui.resize", json!({"width": 520, "height": 800}));
        h.frames(3);
        assert!(crate::scenario::rect_of("bom:inspector").is_none(), "the inspector gives way first");
        assert!(h.app.solidbom.as_ref().expect("the panel").inspector, "the person's choice stays");
        assert!(crate::scenario::rect_of("bom:table").is_some(), "the table still scrolls");
        h.call("ui.resize", json!({"width": 1600, "height": 1000}));
        h.frames(3);
        assert!(crate::scenario::rect_of("bom:inspector").is_some(), "the pane returns when there is room");
        let table = crate::scenario::rect_of("bom:table").expect("the table");
        let inspector = crate::scenario::rect_of("bom:inspector").expect("the inspector");
        assert!(table.right() <= inspector.left(), "the panes never overlap: table {table:?}, inspector {inspector:?}");
    }

    #[test]
    fn each_review_tab_keeps_its_own_table_fraction() {
        let mut h = imported("three_boxes.step");
        let panel = h.app.solidbom.as_mut().expect("the panel");
        panel.table_fraction = 0.45;
        panel.layout_generation += 1;
        h.frames(4);
        let settled = h.app.solidbom.as_ref().expect("the panel").table_fraction;
        h.frames(10);
        let stable = h.app.solidbom.as_ref().expect("the panel").table_fraction;
        assert!((settled - stable).abs() < 0.005, "the split stays put ({settled} then {stable})");
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/solidbom/fixtures/open_surface.step");
        crate::solidbom::panel::open_review_tab(&mut h.app, "solidbom.import", json!({"path": path.to_string_lossy()}))
            .expect("the second review tab");
        h.settle();
        let before = h.app.solidbom.as_ref().expect("the second panel").table_fraction;
        assert!((before - 0.35).abs() < 0.06, "the second tab starts fresh ({before})");
        h.frames(3);
        let second = h.app.solidbom.as_ref().expect("the second panel").table_fraction;
        assert!((second - 0.35).abs() < 0.06, "the second tab starts at its own split ({second})");
        h.call("ui.documents", json!({"action": "switch", "index": 0}));
        h.frames(3);
        let first = h.app.solidbom.as_ref().expect("the first panel").table_fraction;
        assert!((first - 0.45).abs() < 0.06, "the first tab keeps its own split ({first})");
    }

    #[test]
    fn workspace_renders_in_both_themes_and_an_rtl_language() {
        let mut h = imported("three_boxes.step");
        for theme in ["light", "dark"] {
            h.app.preferences.theme = theme.into();
            h.frames(2);
            assert!(crate::scenario::rect_of("bom:table").is_some(), "{theme}");
        }
        h.app.preferences.language = "ar".into();
        h.frames(2);
        assert!(crate::scenario::rect_of("bom:table").is_some());
        assert!(crate::scenario::rect_of("bom:inspector").is_some());
    }
}
