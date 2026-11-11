//! The internal summary PDF renderer (package SB-21).
//!
//! The caller is [`super::resolve`]; the values come from [`super::model::ExportModel`]. This module
//! writes a minimal, dependency-free PDF document by hand: the exported result's metadata and
//! freshness statement, the cost summary, the annual prices and EBIT of every saved scenario, the
//! tooling and setup charges, and the unresolved issues. It performs no business arithmetic, it only
//! formats, wraps and paginates the values the model found.
//!
//! The document is a PDF 1.4 file: A4 portrait, one uncompressed content stream per page (so an
//! independent parser can extract the text) and the two standard Helvetica fonts with
//! [`WinAnsiEncoding`](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/PDF32000_2008.pdf).
//! The bytes are deterministic: the same model renders the same file, and the file carries no
//! timestamp or random identifier of its own.
//!
//! # Text encoding limitation
//!
//! Helvetica with WinAnsiEncoding covers the Latin-1 range and the usual Windows punctuation. A
//! character outside that set (CJK, Cyrillic, Greek, emoji, ...) cannot be written by these standard
//! fonts and is rendered as `?`. The detailed Excel workbook of the same export carries the exact
//! Unicode text; this summary is internal review material, so losing a glyph in the PDF is
//! preferable to embedding a font and a new dependency.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use solidbom_contract::{
    BaselineSetupOwner, CommercialStaleReason, ExportBaselineSelection, ExportLabel, ExportStaleReason, ResultState, ToolingTreatment,
};
use uuid::Uuid;

use super::model::ExportModel;

// ---------------------------------------------------------------------------
// Page geometry
// ---------------------------------------------------------------------------

/// A4 portrait width inPostScript points (595.28 pt rounded to the whole point).
const PAGE_WIDTH: f64 = 595.0;
/// A4 portrait height in PostScript points (841.89 pt rounded to the whole point).
const PAGE_HEIGHT: f64 = 842.0;
/// Left content margin.
const LEFT: f64 = 40.0;
/// Right content margin: the right edge of every column.
const RIGHT: f64 = 555.0;
/// Content width between the margins.
const CONTENT_WIDTH: f64 = RIGHT - LEFT;
/// The baseline of the first line of a page.
const TOP: f64 = 802.0;
/// No body text is drawn below this baseline; the footer lives below it.
const CONTENT_BOTTOM: f64 = 64.0;

const BODY_SIZE: f64 = 9.0;
const BODY_LINE: f64 = 12.5;
const KEY_SIZE: f64 = 8.5;
const KEY_LINE: f64 = 11.0;
const KEY_WIDTH: f64 = 150.0;
const VALUE_X: f64 = LEFT + KEY_WIDTH + 6.0;
const VALUE_WIDTH: f64 = RIGHT - VALUE_X;
const HEADING_SIZE: f64 = 12.0;
const HEADING_LINE: f64 = 15.0;
const SUBHEADING_SIZE: f64 = 10.0;
const SUBHEADING_LINE: f64 = 13.0;
const TITLE_SIZE: f64 = 22.0;
const TITLE_LINE: f64 = 26.0;
const BAND_SIZE: f64 = 10.0;
const BAND_LINE: f64 = 13.5;
const BAND_PADDING: f64 = 8.0;
const FOOTER_SIZE: f64 = 7.5;
const TABLE_SIZE: f64 = 7.0;
const TABLE_LINE: f64 = 9.5;
const HEADER_SIZE: f64 = 7.0;
const CELL_PADDING: f64 = 2.0;

// ---------------------------------------------------------------------------
// Page layout entry point
// ---------------------------------------------------------------------------

/// Render the internal summary document.
pub(super) fn render(model: &ExportModel) -> Vec<u8> {
    let mut document = Document::new(model);
    document.title_block(model);
    document.metadata_block(model);
    document.cost_summary(model);
    document.commercial_scenarios(model);
    document.tooling_and_setup(model);
    document.issues(model);
    document.finish()
}

/// One laid-out page: an uncompressed PDF content stream.
struct Page {
    content: Vec<u8>,
}

/// The document builder: page list, vertical cursor and the table header a continuation repeats.
struct Document {
    pages: Vec<Page>,
    /// Baseline of the next line to draw.
    y: f64,
    /// The table whose header is repeated on every continuation page, while a table is open.
    table: Option<&'static [Column]>,
    /// The selected result, as the footer and the metadata table name it.
    identity: String,
    /// The project name, as the footer carries it.
    project: String,
    /// `DRAFT RESULT`, `INCOMPLETE` or both, repeated in the footer when labelled.
    freshness: Option<String>,
    /// The `/Info` dictionary body (deterministic: no creation or modification date).
    info: Vec<u8>,
}

impl Document {
    fn new(model: &ExportModel) -> Self {
        let identity = selected_result_text(&model.metadata.baseline);
        let project = model.metadata.project_name.clone();
        let freshness = footer_freshness(&model.metadata.labels);
        let info = info_object(&model.metadata.project_name);
        let mut document = Self { pages: Vec::new(), y: TOP, table: None, identity, project, freshness, info };
        document.begin_page();
        document
    }

    fn begin_page(&mut self) {
        self.pages.push(Page { content: Vec::new() });
        self.y = TOP;
    }

    /// Start a page and repeat the table header when a table is open.
    fn new_page(&mut self) {
        self.begin_page();
        if let Some(columns) = self.table {
            self.draw_table_header(columns);
        }
    }

    /// Append the footer to every page and serialize the file.
    fn finish(mut self) -> Vec<u8> {
        let total = self.pages.len();
        for index in 0..total {
            let footer = self.footer(index + 1, total);
            if let Some(page) = self.pages.get_mut(index) {
                page.content.extend_from_slice(&footer);
            }
        }
        build_pdf(&self.pages, &self.info)
    }

    fn footer(&self, number: usize, total: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let review = encode(&format!("Internal review only - page {number} of {total}"));
        push_text(&mut out, LEFT, 34.0, FOOTER_SIZE, false, 100.0, &review, "0 g");
        let review_width = text_width(&review, FOOTER_SIZE, false);
        let identity = encode(&self.identity);
        let identity_room = (CONTENT_WIDTH - review_width - 12.0).max(24.0);
        push_right_fit(&mut out, RIGHT, 34.0, FOOTER_SIZE, false, identity_room, &identity, "0 g");
        let project = encode(&format!("Project: {}", self.project));
        match &self.freshness {
            Some(freshness) => {
                let freshness = encode(freshness);
                let freshness_width = text_width(&freshness, FOOTER_SIZE, false);
                push_right_fit(&mut out, RIGHT, 20.0, FOOTER_SIZE, false, CONTENT_WIDTH, &freshness, "0 g");
                let project_room = (CONTENT_WIDTH - freshness_width - 12.0).max(24.0);
                push_left_fit(&mut out, LEFT, 20.0, FOOTER_SIZE, false, project_room, &project, "0 g");
            }
            None => push_left_fit(&mut out, LEFT, 20.0, FOOTER_SIZE, false, CONTENT_WIDTH, &project, "0 g"),
        }
        out
    }

    // -- primitives ---------------------------------------------------------

    fn content(&mut self) -> Option<&mut Vec<u8>> {
        self.pages.last_mut().map(|page| &mut page.content)
    }

    fn draw_text(&mut self, x: f64, baseline: f64, size: f64, bold: bool, scale: f64, bytes: &[u8]) {
        if let Some(content) = self.content() {
            push_text(content, x, baseline, size, bold, scale, bytes, "0 g");
        }
    }

    fn draw_text_fill(&mut self, x: f64, baseline: f64, size: f64, bold: bool, scale: f64, bytes: &[u8], fill: &str) {
        if let Some(content) = self.content() {
            push_text(content, x, baseline, size, bold, scale, bytes, fill);
        }
    }

    fn draw_right_fit(&mut self, right: f64, baseline: f64, size: f64, bold: bool, room: f64, bytes: &[u8]) {
        if let Some(content) = self.content() {
            push_right_fit(content, right, baseline, size, bold, room, bytes, "0 g");
        }
    }

    fn draw_left_fit(&mut self, left: f64, baseline: f64, size: f64, bold: bool, room: f64, bytes: &[u8]) {
        if let Some(content) = self.content() {
            push_left_fit(content, left, baseline, size, bold, room, bytes, "0 g");
        }
    }

    fn filled_rect(&mut self, x: f64, y: f64, width: f64, height: f64, fill: &str) {
        if let Some(content) = self.content() {
            content.extend_from_slice(format!("q {fill} rg {x:.2} {y:.2} {width:.2} {height:.2} re f Q\n").as_bytes());
        }
    }

    fn rule(&mut self) {
        let y = self.y;
        if let Some(content) = self.content() {
            content.extend_from_slice(format!("q 0.55 G 0.6 w {LEFT:.2} {y:.2} m {RIGHT:.2} {y:.2} l S Q\n").as_bytes());
        }
    }

    fn space(&mut self, amount: f64) {
        self.y -= amount;
        if self.y < CONTENT_BOTTOM {
            self.new_page();
        }
    }

    fn available_lines(&self, line_height: f64) -> usize {
        let room = self.y - CONTENT_BOTTOM;
        if room <= 0.0 || line_height <= 0.0 { 0 } else { (room / line_height).floor() as usize }
    }

    fn ensure_space(&mut self, height: f64) {
        if self.y - height < CONTENT_BOTTOM {
            self.new_page();
        }
    }

    // -- text blocks --------------------------------------------------------

    /// A wrapped paragraph at the left margin, in the body font.
    fn paragraph(&mut self, text: &str, size: f64, line_height: f64, bold: bool) {
        let lines = wrap(&encode(text), size, bold, CONTENT_WIDTH);
        self.paragraph_lines(LEFT, lines, size, line_height, bold);
    }

    /// Draw already wrapped lines, chunking them across pages when they do not fit.
    fn paragraph_lines(&mut self, x: f64, lines: Vec<Vec<u8>>, size: f64, line_height: f64, bold: bool) {
        let mut index = 0;
        loop {
            let available = self.available_lines(line_height);
            if available == 0 {
                self.new_page();
                continue;
            }
            let take = available.min(lines.len() - index);
            for line in lines.get(index..index + take).unwrap_or_default() {
                let baseline = self.y - size;
                self.draw_text(x, baseline, size, bold, 100.0, line);
                self.y -= line_height;
            }
            index += take;
            if index >= lines.len() {
                break;
            }
            self.new_page();
        }
    }

    fn heading(&mut self, text: &str) {
        let lines = wrap(&encode(text), HEADING_SIZE, true, CONTENT_WIDTH);
        let height = lines.len() as f64 * HEADING_LINE + 16.0;
        self.ensure_space(height);
        self.y -= 10.0;
        self.paragraph_lines(LEFT, lines, HEADING_SIZE, HEADING_LINE, true);
        self.rule();
        self.y -= 6.0;
    }

    fn subheading(&mut self, text: &str) {
        self.space(4.0);
        let lines = wrap(&encode(text), SUBHEADING_SIZE, true, CONTENT_WIDTH);
        self.ensure_space(lines.len() as f64 * SUBHEADING_LINE + 6.0);
        self.paragraph_lines(LEFT, lines, SUBHEADING_SIZE, SUBHEADING_LINE, true);
        self.y -= 2.0;
    }

    /// A subheading whose next element is a table: reserve room for the header and a few rows too,
    /// so a heading never sits alone at the bottom of a page.
    fn table_heading(&mut self, text: &str) {
        self.space(4.0);
        let lines = wrap(&encode(text), SUBHEADING_SIZE, true, CONTENT_WIDTH);
        let height = lines.len() as f64 * SUBHEADING_LINE + 8.0 * TABLE_LINE + 18.0;
        self.ensure_space(height);
        self.paragraph_lines(LEFT, lines, SUBHEADING_SIZE, SUBHEADING_LINE, true);
        self.y -= 2.0;
    }

    /// One key/value row: the key in the bold key column, the value wrapped beside it.
    fn kv(&mut self, key: &str, value: &str) {
        let key_lines = wrap(&encode(key), KEY_SIZE, true, KEY_WIDTH);
        let value_lines = wrap(&encode(value), KEY_SIZE, false, VALUE_WIDTH);
        let mut offset = 0;
        loop {
            let remaining = key_lines.len().max(value_lines.len()).saturating_sub(offset);
            if remaining == 0 {
                break;
            }
            let available = self.available_lines(KEY_LINE);
            if available == 0 {
                self.new_page();
                continue;
            }
            let take = available.min(remaining);
            let top = self.y;
            for line_index in 0..take {
                let baseline = top - KEY_SIZE - line_index as f64 * KEY_LINE;
                if let Some(line) = key_lines.get(offset + line_index) {
                    self.draw_text(LEFT, baseline, KEY_SIZE, true, 100.0, line);
                }
                if let Some(line) = value_lines.get(offset + line_index) {
                    self.draw_text(VALUE_X, baseline, KEY_SIZE, false, 100.0, line);
                }
            }
            self.y -= take as f64 * KEY_LINE;
            offset += take;
            if key_lines.len() > offset || value_lines.len() > offset {
                self.new_page();
            }
        }
        self.y -= 3.0;
    }

    // -- tables -------------------------------------------------------------

    fn begin_table(&mut self, columns: &'static [Column]) {
        self.table = None;
        let header_lines = columns.iter().map(|column| wrap(&encode(column.title), HEADER_SIZE, true, column.width).len()).max().unwrap_or(1).max(1);
        self.ensure_space(header_lines as f64 * TABLE_LINE + 3.0 * TABLE_LINE + 14.0);
        self.table = Some(columns);
        self.draw_table_header(columns);
    }

    fn end_table(&mut self) {
        self.table = None;
        self.y -= 6.0;
    }

    fn draw_table_header(&mut self, columns: &'static [Column]) {
        let prepared: Vec<Vec<Vec<u8>>> =
            columns.iter().map(|column| wrap(&encode(column.title), HEADER_SIZE, true, column.width - 2.0 * CELL_PADDING)).collect();
        let lines = prepared.iter().map(Vec::len).max().unwrap_or(1).max(1);
        self.ensure_space(lines as f64 * TABLE_LINE + 8.0);
        let top = self.y;
        for (column, cell_lines) in columns.iter().zip(prepared.iter()) {
            for (index, line) in cell_lines.iter().enumerate() {
                let baseline = top - HEADER_SIZE - index as f64 * TABLE_LINE;
                match column.align {
                    Align::Left => {
                        let room = column.width - 2.0 * CELL_PADDING;
                        self.draw_left_fit(column.x + CELL_PADDING, baseline, HEADER_SIZE, true, room, line);
                    }
                    Align::Right => {
                        self.draw_right_fit(
                            column.x + column.width - CELL_PADDING,
                            baseline,
                            HEADER_SIZE,
                            true,
                            column.width - 2.0 * CELL_PADDING,
                            line,
                        );
                    }
                }
            }
        }
        self.y = top - lines as f64 * TABLE_LINE - 2.0;
        self.rule();
        self.y -= 4.0;
    }

    /// One table row. Cells wrap; a row that does not fit continues on the next page under a
    /// repeated header, so a long message never overlaps another column or leaves the page.
    fn row(&mut self, cells: &[String]) {
        let Some(columns) = self.table else {
            return;
        };
        let prepared: Vec<Vec<Vec<u8>>> = columns
            .iter()
            .enumerate()
            .map(|(index, column)| {
                let text = cells.get(index).map(String::as_str).unwrap_or_default();
                let encoded = encode(text);
                if column.single_line || column.align == Align::Right {
                    vec![encoded]
                } else {
                    wrap(&encoded, TABLE_SIZE, false, column.width - 2.0 * CELL_PADDING)
                }
            })
            .collect();
        let mut offset = 0;
        loop {
            let remaining = prepared.iter().map(Vec::len).max().unwrap_or(0).saturating_sub(offset);
            if remaining == 0 {
                break;
            }
            let available = self.available_lines(TABLE_LINE);
            if available == 0 {
                self.new_page();
                continue;
            }
            let take = available.min(remaining);
            let top = self.y;
            for (index, column) in columns.iter().enumerate() {
                let Some(cell_lines) = prepared.get(index) else {
                    continue;
                };
                for (line_index, line) in cell_lines.iter().skip(offset).take(take).enumerate() {
                    let baseline = top - TABLE_SIZE - line_index as f64 * TABLE_LINE;
                    match column.align {
                        Align::Left => {
                            let room = column.width - 2.0 * CELL_PADDING;
                            if column.single_line {
                                self.draw_left_fit(column.x + CELL_PADDING, baseline, TABLE_SIZE, false, room, line);
                            } else {
                                self.draw_text(column.x + CELL_PADDING, baseline, TABLE_SIZE, false, 100.0, line);
                            }
                        }
                        Align::Right => self.draw_right_fit(
                            column.x + column.width - CELL_PADDING,
                            baseline,
                            TABLE_SIZE,
                            false,
                            column.width - 2.0 * CELL_PADDING,
                            line,
                        ),
                    }
                }
            }
            self.y -= take as f64 * TABLE_LINE;
            offset += take;
            if prepared.iter().any(|cell_lines| cell_lines.len() > offset) {
                self.new_page();
            }
        }
        self.y -= 2.0;
    }

    // -- sections -----------------------------------------------------------

    fn title_block(&mut self, model: &ExportModel) {
        let metadata = &model.metadata;
        self.paragraph("Internal export", TITLE_SIZE, TITLE_LINE, true);
        self.paragraph("Internal review material - not a customer quotation", BODY_SIZE, BODY_LINE, false);
        self.space(4.0);
        self.paragraph(&metadata.project_name, 14.0, 18.0, true);
        let customer = if metadata.customer.trim().is_empty() { "(none recorded)".to_string() } else { metadata.customer.clone() };
        self.paragraph(&format!("Customer: {customer}"), BODY_SIZE, BODY_LINE, false);
        self.paragraph(&format!("Exported at: {}", metadata.exported_at), BODY_SIZE, BODY_LINE, false);
        self.paragraph(&format!("Currency: {} - every amount below is stated in it", metadata.currency), BODY_SIZE, BODY_LINE, false);
        self.paragraph(
            "Characters outside the standard font's WinAnsi set are shown as '?'; the Excel workbook carries the exact text.",
            KEY_SIZE,
            KEY_LINE,
            false,
        );
        self.space(6.0);
        self.status_band(&metadata.labels);
    }

    fn status_band(&mut self, labels: &[ExportLabel]) {
        let text = status_text(labels);
        let lines = wrap(&encode(&text), BAND_SIZE, true, CONTENT_WIDTH - 2.0 * BAND_PADDING);
        let height = 2.0 * BAND_PADDING + lines.len() as f64 * BAND_LINE;
        self.ensure_space(height + 14.0);
        let top = self.y;
        self.filled_rect(LEFT, top - height, CONTENT_WIDTH, height, "0.16 0.16 0.16");
        for (index, line) in lines.iter().enumerate() {
            let baseline = top - BAND_PADDING - BAND_SIZE - index as f64 * BAND_LINE;
            self.draw_text_fill(LEFT + BAND_PADDING, baseline, BAND_SIZE, true, 100.0, line, "1 g");
        }
        self.y = top - height - 12.0;
    }

    fn metadata_block(&mut self, model: &ExportModel) {
        let metadata = &model.metadata;
        let versions = metadata.versions;
        self.heading("Result metadata");
        self.kv("Selected result", &self.identity.clone());
        self.kv("Project record", &format!("v{} (pinned)", versions.project_version));
        self.kv("Engineering record", &format!("v{} (pinned)", versions.engineering_version));
        self.kv("Manufacturing record", &format!("v{} (pinned)", versions.manufacturing_version));
        let feasibility_state = if versions.feasibility_pinned { "pinned" } else { "read at export time" };
        self.kv("Feasibility record", &format!("v{} ({feasibility_state})", versions.feasibility_version));
        self.kv("Calculation id", &metadata.calculation.calculation_id.to_string());
        self.kv("Calculator version", &metadata.calculation.calculator_version);
        self.kv("Input hash", &metadata.calculation.input_hash);
        self.kv("Result state", result_state_display(metadata.calculation.state));
        self.kv("Freshness labels", &labels_text(&metadata.labels));
        self.kv("Stale reasons", &stale_reasons_text(&metadata.stale_reasons));
    }

    fn cost_summary(&mut self, model: &ExportModel) {
        self.heading("Cost summary");
        self.kv("Lifetime manufacturing total (including setup)", &money(model.project_total_manufacturing_cost));
        self.kv("Separate setup total", &money(model.setup_total));
        if model.annual.is_empty() {
            self.paragraph("No annual manufacturing cost series is stored.", BODY_SIZE, BODY_LINE, false);
            return;
        }
        self.table_heading("Annual manufacturing cost");
        self.begin_table(&ANNUAL_COLUMNS);
        for line in &model.annual {
            self.row(&[line.year.to_string(), money(line.manufacturing_cost)]);
        }
        self.end_table();
    }

    fn commercial_scenarios(&mut self, model: &ExportModel) {
        self.heading("Commercial scenarios");
        if model.scenarios.is_empty() {
            self.paragraph("No saved commercial scenario is part of this export.", BODY_SIZE, BODY_LINE, false);
            return;
        }
        for scenario in &model.scenarios {
            self.subheading(&format!("Scenario: {}", scenario.name));
            self.kv("Saved version", &format!("v{}", scenario.version));
            self.kv("Comparability", &comparability_text(scenario.stale, &scenario.stale_reasons));
            let project = &scenario.outcome.project;
            self.kv("Lifetime revenue", &money(project.lifetime_revenue));
            self.kv("Lifetime EBIT", &money(project.lifetime_ebit_amount));
            self.kv("Tooling investment (counted once)", &money(project.tooling_investment));
            self.kv("Tooling separate charge", &money(project.tooling_separate_charge));
            self.kv("Projected tooling recovery", &money(project.tooling_projected_recovery));
            self.kv("Tooling under/over recovery", &money(project.tooling_under_over_recovery));
            self.kv("Separate setup total", &money(scenario.outcome.setup_total));
            self.table_heading("Per product and year");
            self.begin_table(&SCENARIO_COLUMNS);
            for product in &scenario.outcome.products {
                let name = product_display_name(model, product.product_id);
                if product.years.is_empty() {
                    self.row(&[name, "-".into(), "-".into(), "-".into(), "-".into(), "-".into(), "-".into(), "-".into()]);
                    continue;
                }
                for year in &product.years {
                    self.row(&[
                        name.clone(),
                        year.year.to_string(),
                        year.forecast.map(format_quantity).unwrap_or_else(missing_text),
                        money(year.manual_base_price),
                        money(year.revenue),
                        money(year.ebit_amount),
                        money(year.tooling_contribution),
                        money(year.suggested_total_price),
                    ]);
                }
            }
            self.end_table();
        }
    }

    fn tooling_and_setup(&mut self, model: &ExportModel) {
        self.heading("Tooling and setup");
        match &model.tooling {
            Some(tooling) => {
                self.kv("Treatment", treatment_display(tooling.treatment));
                self.kv("Project tooling investment", &money(tooling.project_investment));
                self.kv("Projected recovery", &money(tooling.project_amortized_recovered));
                self.kv("Under/over recovery", &money(tooling.project_under_over_recovery));
                self.kv("Separate charge", &money(tooling.project_separate_charge));
                if tooling.annual_recovery.is_empty() {
                    self.paragraph("No annual tooling recovery is projected.", BODY_SIZE, BODY_LINE, false);
                } else {
                    self.table_heading("Annual tooling recovery");
                    self.begin_table(&ANNUAL_COLUMNS_RECOVERY);
                    for line in &tooling.annual_recovery {
                        self.row(&[line.year.to_string(), money(line.recovered)]);
                    }
                    self.end_table();
                }
            }
            None => self.paragraph("No tooling evaluation is part of the selected result.", BODY_SIZE, BODY_LINE, false),
        }
        self.table_heading("Separate setup lines");
        if model.setup.is_empty() {
            self.paragraph("No separate setup line is stored.", BODY_SIZE, BODY_LINE, false);
            return;
        }
        self.begin_table(&SETUP_COLUMNS);
        for line in &model.setup {
            self.row(&[setup_owner_text(model, line.owner), money(line.setup_charge)]);
        }
        self.end_table();
    }

    fn issues(&mut self, model: &ExportModel) {
        self.heading("Unresolved issues and missing inputs");
        if model.issues.is_empty() {
            self.paragraph("No unresolved issue or missing input is recorded.", BODY_SIZE, BODY_LINE, false);
            return;
        }
        self.paragraph(&format!("{} listed entries.", format_quantity(model.issues.len() as u64)), BODY_SIZE, BODY_LINE, false);
        self.begin_table(&ISSUE_COLUMNS);
        for issue in &model.issues {
            self.row(&[
                issue.source.as_str().to_string(),
                issue.code.clone(),
                issue.scope.map(|scope| scope.to_string()).unwrap_or_else(|| "-".into()),
                issue.message.clone(),
            ]);
        }
        self.end_table();
    }
}

// ---------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    Right,
}

/// One table column: its heading, left edge, width, alignment and whether its cells stay on one
/// line (a wrapped 36-character scope id would be unreadable, so it is scaled to fit instead).
#[derive(Debug, Clone, Copy)]
struct Column {
    title: &'static str,
    x: f64,
    width: f64,
    align: Align,
    single_line: bool,
}

const ANNUAL_COLUMNS: [Column; 2] = [
    Column { title: "Year", x: LEFT, width: 70.0, align: Align::Left, single_line: false },
    Column { title: "Manufacturing cost", x: 280.0, width: RIGHT - 280.0, align: Align::Right, single_line: true },
];

const ANNUAL_COLUMNS_RECOVERY: [Column; 2] = [
    Column { title: "Year", x: LEFT, width: 70.0, align: Align::Left, single_line: false },
    Column { title: "Recovered", x: 280.0, width: RIGHT - 280.0, align: Align::Right, single_line: true },
];

const SCENARIO_COLUMNS: [Column; 8] = [
    Column { title: "Product", x: LEFT, width: 106.0, align: Align::Left, single_line: false },
    Column { title: "Year", x: 150.0, width: 26.0, align: Align::Left, single_line: true },
    Column { title: "Forecast", x: 180.0, width: 56.0, align: Align::Right, single_line: true },
    Column { title: "Manual price", x: 240.0, width: 58.0, align: Align::Right, single_line: true },
    Column { title: "Revenue", x: 302.0, width: 58.0, align: Align::Right, single_line: true },
    Column { title: "EBIT amount", x: 364.0, width: 58.0, align: Align::Right, single_line: true },
    Column { title: "Tooling contribution", x: 426.0, width: 64.0, align: Align::Right, single_line: true },
    Column { title: "Suggested price", x: 494.0, width: RIGHT - 494.0, align: Align::Right, single_line: true },
];

const SETUP_COLUMNS: [Column; 2] = [
    Column { title: "Owner", x: LEFT, width: 360.0, align: Align::Left, single_line: false },
    Column { title: "Setup charge", x: 404.0, width: RIGHT - 404.0, align: Align::Right, single_line: true },
];

const ISSUE_COLUMNS: [Column; 4] = [
    Column { title: "Source", x: LEFT, width: 52.0, align: Align::Left, single_line: true },
    Column { title: "Code", x: 96.0, width: 92.0, align: Align::Left, single_line: false },
    Column { title: "Scope", x: 192.0, width: 126.0, align: Align::Left, single_line: true },
    Column { title: "Message", x: 322.0, width: RIGHT - 322.0, align: Align::Left, single_line: false },
];

// ---------------------------------------------------------------------------
// Labels and formats
// ---------------------------------------------------------------------------

fn selected_result_text(baseline: &ExportBaselineSelection) -> String {
    match baseline {
        ExportBaselineSelection::AcceptedBaseline { sequence, .. } => format!("Accepted baseline #{sequence}"),
        ExportBaselineSelection::DraftCalculation { calculation_id } => format!("Draft calculation {calculation_id}"),
    }
}

fn footer_freshness(labels: &[ExportLabel]) -> Option<String> {
    let mut parts = Vec::new();
    for label in labels {
        match label {
            ExportLabel::DraftResult => parts.push("DRAFT RESULT"),
            ExportLabel::IncompleteResult => parts.push("INCOMPLETE"),
            _ => {}
        }
    }
    if parts.is_empty() { None } else { Some(parts.join(" - ")) }
}

fn status_text(labels: &[ExportLabel]) -> String {
    if labels.is_empty() {
        return "FRESH RESULT".to_string();
    }
    labels.iter().map(|label| label_display(*label)).collect::<Vec<_>>().join(" | ")
}

fn label_display(label: ExportLabel) -> &'static str {
    match label {
        ExportLabel::DraftResult => "DRAFT RESULT",
        ExportLabel::IncompleteResult => "INCOMPLETE",
        ExportLabel::MissingInputs => "MISSING INPUTS",
        ExportLabel::UnresolvedFindings => "UNRESOLVED FINDINGS",
        ExportLabel::OutdatedCosts => "OUTDATED COSTS",
        ExportLabel::SupersededAcceptedBaseline => "SUPERSEDED ACCEPTED BASELINE",
        ExportLabel::StaleScenarios => "STALE SCENARIOS",
        ExportLabel::UnpinnedFeasibilityRecord => "UNPINNED FEASIBILITY RECORD",
    }
}

fn labels_text(labels: &[ExportLabel]) -> String {
    if labels.is_empty() { "none".to_string() } else { labels.iter().map(|label| label.as_str()).collect::<Vec<_>>().join(", ") }
}

fn stale_reasons_text(reasons: &[ExportStaleReason]) -> String {
    if reasons.is_empty() { "none".to_string() } else { reasons.iter().map(|reason| stale_reason_display(*reason)).collect::<Vec<_>>().join(", ") }
}

fn stale_reason_display(reason: ExportStaleReason) -> &'static str {
    match reason {
        ExportStaleReason::NewerProjectVersion => "newer project version",
        ExportStaleReason::NewerEngineeringVersion => "newer engineering version",
        ExportStaleReason::NewerManufacturingVersion => "newer manufacturing version",
        ExportStaleReason::NewerSharedData => "newer shared data",
    }
}

fn result_state_display(state: ResultState) -> &'static str {
    match state {
        ResultState::Complete => "complete",
        ResultState::Incomplete => "incomplete",
    }
}

fn treatment_display(treatment: Option<ToolingTreatment>) -> &'static str {
    match treatment {
        Some(ToolingTreatment::Separate) => "separate charge",
        Some(ToolingTreatment::Amortized) => "amortized per unit",
        None => "not selected",
    }
}

fn comparability_text(stale: bool, reasons: &[CommercialStaleReason]) -> String {
    if !stale {
        return "directly comparable with the exported result".to_string();
    }
    if reasons.is_empty() {
        return "stale - the saved scenario is not directly comparable".to_string();
    }
    let named = reasons.iter().map(|reason| format!("{} [{}]", reason.message, reason.code)).collect::<Vec<_>>().join("; ");
    format!("stale - {named}")
}

fn missing_text() -> String {
    "missing".to_string()
}

fn money(value: Option<Decimal>) -> String {
    value.map(format_money).unwrap_or_else(missing_text)
}

/// Format an amount with two fractional digits, grouped in threes and a leading minus for a
/// negative value. A value outside the `i128` cent range falls back to the plain decimal text.
fn format_money(value: Decimal) -> String {
    let rounded = value.round_dp(2);
    let negative = rounded.is_sign_negative() && !rounded.is_zero();
    let magnitude = rounded.abs();
    let cents = magnitude.checked_mul(Decimal::ONE_HUNDRED).and_then(|scaled| scaled.trunc().to_i128());
    let Some(cents) = cents else {
        let text = format!("{rounded}");
        return if negative { format!("-{text}") } else { text };
    };
    let digits = cents.unsigned_abs().to_string();
    let (whole, fraction) = if digits.len() <= 2 {
        ("0".to_string(), format!("{:0>2}", digits))
    } else {
        let split = digits.len() - 2;
        (digits.get(..split).unwrap_or_default().to_string(), digits.get(split..).unwrap_or_default().to_string())
    };
    let sign = if negative { "-" } else { "" };
    format!("{sign}{}.{}", group_digits(&whole), fraction_for(&fraction))
}

fn fraction_for(fraction: &str) -> String {
    if fraction.len() >= 2 { fraction.to_string() } else { format!("{fraction:0>2}") }
}

fn group_digits(digits: &str) -> String {
    let bytes = digits.as_bytes();
    let mut grouped = String::with_capacity(bytes.len() + bytes.len() / 3);
    for (index, byte) in bytes.iter().enumerate() {
        if index > 0 && (bytes.len() - index).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(char::from(*byte));
    }
    grouped
}

fn format_quantity(value: u64) -> String {
    group_digits(&value.to_string())
}

fn product_display_name(model: &ExportModel, product_id: Uuid) -> String {
    if let Some(product) = model.products.iter().find(|product| product.product_id == product_id) {
        return product.display_name();
    }
    if let Some(product) = model.scope.products.iter().find(|product| product.product_id == product_id) {
        return product.display_name();
    }
    product_id.to_string()
}

fn setup_owner_text(model: &ExportModel, owner: BaselineSetupOwner) -> String {
    match owner {
        BaselineSetupOwner::PartVersion { part_version_id } => {
            if let Some(part) = model.parts.iter().find(|part| part.part_version_id == part_version_id) {
                format!("Part {}", part.display_name())
            } else {
                format!("Part {part_version_id}")
            }
        }
        BaselineSetupOwner::Product { product_id } => format!("Product {}", product_display_name(model, product_id)),
    }
}

// ---------------------------------------------------------------------------
// Text encoding: WinAnsi and PDF string escaping
// ---------------------------------------------------------------------------

/// Replace a character outside WinAnsiEncoding with `?`; return the single WinAnsi byte.
fn encode_character(character: char) -> u8 {
    let code = character as u32;
    if (0x20..=0x7E).contains(&code) {
        return code as u8;
    }
    if (0xA0..=0xFF).contains(&code) {
        return code as u8;
    }
    if let Some(byte) = winansi_special(character) {
        return byte;
    }
    match character {
        '\n' | '\r' | '\t' => b' ',
        _ => b'?',
    }
}

/// The characters WinAnsiEncoding places in the 0x80..=0x9F block.
fn winansi_special(character: char) -> Option<u8> {
    Some(match character {
        '\u{20AC}' => 0x80,
        '\u{201A}' => 0x82,
        '\u{0192}' => 0x83,
        '\u{201E}' => 0x84,
        '\u{2026}' => 0x85,
        '\u{2020}' => 0x86,
        '\u{2021}' => 0x87,
        '\u{02C6}' => 0x88,
        '\u{2030}' => 0x89,
        '\u{0160}' => 0x8A,
        '\u{2039}' => 0x8B,
        '\u{0152}' => 0x8C,
        '\u{017D}' => 0x8E,
        '\u{2018}' => 0x91,
        '\u{2019}' => 0x92,
        '\u{201C}' => 0x93,
        '\u{201D}' => 0x94,
        '\u{2022}' => 0x95,
        '\u{2013}' => 0x96,
        '\u{2014}' => 0x97,
        '\u{02DC}' => 0x98,
        '\u{2122}' => 0x99,
        '\u{0161}' => 0x9A,
        '\u{203A}' => 0x9B,
        '\u{0153}' => 0x9C,
        '\u{017E}' => 0x9E,
        '\u{0178}' => 0x9F,
        _ => return None,
    })
}

fn encode(text: &str) -> Vec<u8> {
    text.chars().map(encode_character).collect()
}

fn push_escaped(out: &mut Vec<u8>, bytes: &[u8]) {
    for byte in bytes {
        match byte {
            b'\\' | b'(' | b')' => {
                out.push(b'\\');
                out.push(*byte);
            }
            _ => out.push(*byte),
        }
    }
}

// ---------------------------------------------------------------------------
// Font metrics and wrapping
// ---------------------------------------------------------------------------

/// Helvetica advance widths in 1/1000 em for the printable ASCII range, from the standard AFM.
const HELVETICA_UNITS: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584,
    584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667,
    611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500,
    722, 500, 500, 500, 334, 260, 334, 584,
];

/// Helvetica-Bold advance widths in 1/1000 em for the printable ASCII range, from the standard AFM.
const HELVETICA_BOLD_UNITS: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584,
    584, 584, 611, 975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667,
    611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556,
    778, 556, 556, 500, 389, 280, 389, 584,
];

fn char_units(byte: u8, bold: bool) -> u16 {
    if (0x20..=0x7E).contains(&byte) {
        let index = usize::from(byte - 0x20);
        let table = if bold { &HELVETICA_BOLD_UNITS } else { &HELVETICA_UNITS };
        table.get(index).copied().unwrap_or(556)
    } else {
        // A conservative width for the Latin-1 letters and the WinAnsi punctuation block.
        722
    }
}

fn text_width(bytes: &[u8], size: f64, bold: bool) -> f64 {
    let units: u32 = bytes.iter().map(|byte| u32::from(char_units(*byte, bold))).sum();
    f64::from(units) * size / 1000.0
}

/// Wrap WinAnsi bytes to lines no wider than `max_width`; a word longer than a line is split.
fn wrap(bytes: &[u8], size: f64, bold: bool, max_width: f64) -> Vec<Vec<u8>> {
    let mut lines: Vec<Vec<u8>> = Vec::new();
    let mut current: Vec<u8> = Vec::new();
    let mut current_width = 0.0;
    let space_width = text_width(b" ", size, bold);
    for token in bytes.split(|byte| *byte == b' ') {
        if token.is_empty() {
            continue;
        }
        let mut remaining = token.to_vec();
        while text_width(&remaining, size, bold) > max_width {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                current_width = 0.0;
            }
            let mut split_at = 1;
            let mut width = 0.0;
            for (index, byte) in remaining.iter().enumerate() {
                width += f64::from(char_units(*byte, bold)) * size / 1000.0;
                if width > max_width {
                    break;
                }
                split_at = index + 1;
            }
            let (head, tail) = remaining.split_at(split_at.min(remaining.len()).max(1));
            lines.push(head.to_vec());
            remaining = tail.to_vec();
        }
        if remaining.is_empty() {
            continue;
        }
        let token_width = text_width(&remaining, size, bold);
        if current.is_empty() {
            current = remaining;
            current_width = token_width;
        } else if current_width + space_width + token_width <= max_width {
            current.push(b' ');
            current.extend_from_slice(&remaining);
            current_width += space_width + token_width;
        } else {
            lines.push(std::mem::take(&mut current));
            current = remaining;
            current_width = token_width;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    lines
}

// ---------------------------------------------------------------------------
// Content-stream primitives
// ---------------------------------------------------------------------------

/// Write one text object: fill colour, font, optional horizontal scaling, absolute position and
/// the escaped string. Keeping every line in its own `BT`/`ET` makes the stream deterministic and
/// easy for an independent parser to read.
fn push_text(out: &mut Vec<u8>, x: f64, baseline: f64, size: f64, bold: bool, scale: f64, bytes: &[u8], fill: &str) {
    out.extend_from_slice(fill.as_bytes());
    out.extend_from_slice(b" BT /");
    out.extend_from_slice(if bold { b"F2" } else { b"F1" });
    out.extend_from_slice(b" ");
    out.extend_from_slice(format!("{size:.2}").as_bytes());
    out.extend_from_slice(b" Tf");
    if (scale - 100.0).abs() > 0.005 {
        out.extend_from_slice(b" ");
        out.extend_from_slice(format!("{scale:.2}").as_bytes());
        out.extend_from_slice(b" Tz");
    }
    out.extend_from_slice(b" 1 0 0 1 ");
    out.extend_from_slice(format!("{x:.2}").as_bytes());
    out.push(b' ');
    out.extend_from_slice(format!("{baseline:.2}").as_bytes());
    out.extend_from_slice(b" Tm (");
    push_escaped(out, bytes);
    out.extend_from_slice(b") Tj ET\n");
}

/// Right-align text at `right`, horizontally scaling it down when it is wider than `room`.
fn push_right_fit(out: &mut Vec<u8>, right: f64, baseline: f64, size: f64, bold: bool, room: f64, bytes: &[u8], fill: &str) {
    let width = text_width(bytes, size, bold);
    let scale = if width > room && width > 0.0 { (100.0 * room / width).clamp(1.0, 100.0) } else { 100.0 };
    push_text(out, right - width * scale / 100.0, baseline, size, bold, scale, bytes, fill);
}

/// Left-align text at `left`, horizontally scaling it down when it is wider than `room`.
fn push_left_fit(out: &mut Vec<u8>, left: f64, baseline: f64, size: f64, bold: bool, room: f64, bytes: &[u8], fill: &str) {
    let width = text_width(bytes, size, bold);
    let scale = if width > room && width > 0.0 { (100.0 * room / width).clamp(1.0, 100.0) } else { 100.0 };
    push_text(out, left, baseline, size, bold, scale, bytes, fill);
}

// ---------------------------------------------------------------------------
// PDF file assembly
// ---------------------------------------------------------------------------

const PDF_HEADER: &[u8] = b"%PDF-1.4\n";
const BINARY_COMMENT: &[u8] = b"%\xE2\xE3\xCF\xD3\n";

/// The `/Info` dictionary body. It carries no timestamp, so two renders of one model are equal.
fn info_object(project_name: &str) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"<< /Title (Internal export) /Subject (Internal summary for ");
    push_escaped(&mut body, &encode(project_name));
    body.extend_from_slice(b") /Creator (SolidBOM) /Producer (SolidBOM internal export) >>");
    body
}

/// Serialize the laid-out pages as a PDF 1.4 file with an object for every page and content stream,
/// a correct cross-reference table and trailer.
fn build_pdf(pages: &[Page], info: &[u8]) -> Vec<u8> {
    let page_count = pages.len();
    let object_count = 5 + page_count * 2;
    let mut out = Vec::with_capacity(page_count * 1500);
    out.extend_from_slice(PDF_HEADER);
    out.extend_from_slice(BINARY_COMMENT);

    // offsets[0] is the sentinel for object 0; offsets[id] is the byte offset of object `id`.
    let mut offsets: Vec<usize> = Vec::with_capacity(object_count + 1);
    offsets.push(0);

    write_object(&mut out, &mut offsets, b"<< /Type /Catalog /Pages 2 0 R >>");

    let mut pages_body = Vec::new();
    pages_body.extend_from_slice(b"<< /Type /Pages /Kids [");
    for index in 0..page_count {
        if index > 0 {
            pages_body.push(b' ');
        }
        pages_body.extend_from_slice(format!("{} 0 R", 6 + index * 2).as_bytes());
    }
    pages_body.extend_from_slice(format!("] /Count {page_count} >>").as_bytes());
    write_object(&mut out, &mut offsets, &pages_body);

    write_object(&mut out, &mut offsets, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>");
    write_object(&mut out, &mut offsets, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>");
    write_object(&mut out, &mut offsets, info);

    for (index, page) in pages.iter().enumerate() {
        let page_id = 6 + index * 2;
        let content_id = page_id + 1;
        let page_body = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_WIDTH:.0} {PAGE_HEIGHT:.0}] /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {content_id} 0 R >>"
        );
        write_object(&mut out, &mut offsets, page_body.as_bytes());

        let mut content = page.content.clone();
        if !content.ends_with(b"\n") {
            content.push(b'\n');
        }
        let mut stream = Vec::with_capacity(content.len() + 48);
        stream.extend_from_slice(format!("<< /Length {} >>\nstream\n", content.len()).as_bytes());
        stream.extend_from_slice(&content);
        stream.extend_from_slice(b"endstream");
        write_object(&mut out, &mut offsets, &stream);
    }

    let xref_offset = out.len();
    out.extend_from_slice(b"xref\n");
    out.extend_from_slice(format!("0 {}\n", object_count + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R /Info 5 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n", object_count + 1).as_bytes());
    out
}

fn write_object(out: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]) {
    let id = offsets.len();
    offsets.push(out.len());
    out.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
    out.extend_from_slice(body);
    out.extend_from_slice(b"\nendobj\n");
}

#[cfg(test)]
mod tests {
    use super::super::model::{ExportIssue, ExportIssueSource, ExportProduct, ExportScenario, ExportScope, ExportScopedProduct};
    use super::*;
    use solidbom_contract::{
        AnnualForecast, BaselineAnnualProjectCost, BaselineSetupLine, CommercialOutcome, CommercialProductLine, CommercialProjectTotals,
        CommercialStaleReason, CommercialYearLine, ExportCalculation, ExportFormat, ExportMetadata, ExportRecordVersions, ProductKind,
        QuotationState, ToolingAnnualRecovery, ToolingOutcome, YearRange,
    };

    // -- fixtures -----------------------------------------------------------

    fn id(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }

    fn decimal(text: &str) -> Decimal {
        text.parse().unwrap_or(Decimal::ZERO)
    }

    fn metadata(project_name: &str) -> ExportMetadata {
        ExportMetadata {
            project_id: id(1),
            project_name: project_name.into(),
            customer: "Acme Industrial".into(),
            currency: "CNY".into(),
            exported_at: "2026-11-11T13:00:00Z".into(),
            format: ExportFormat::Pdf,
            baseline: ExportBaselineSelection::DraftCalculation { calculation_id: id(2) },
            versions: ExportRecordVersions {
                project_version: 4,
                engineering_version: 3,
                manufacturing_version: 2,
                feasibility_version: 1,
                feasibility_pinned: false,
            },
            calculation: ExportCalculation {
                calculation_id: id(2),
                calculator_version: "solidbom-manufacturing-baseline/1".into(),
                input_hash: "a".repeat(64),
                state: ResultState::Incomplete,
            },
            labels: vec![ExportLabel::DraftResult, ExportLabel::IncompleteResult],
            stale: true,
            stale_reasons: vec![ExportStaleReason::NewerProjectVersion, ExportStaleReason::NewerSharedData],
        }
    }

    fn accepted_metadata(project_name: &str) -> ExportMetadata {
        let mut metadata = metadata(project_name);
        metadata.baseline = ExportBaselineSelection::AcceptedBaseline { accepted_baseline_id: id(3), sequence: 7 };
        metadata.versions.feasibility_pinned = true;
        metadata.labels = Vec::new();
        metadata.stale = false;
        metadata.stale_reasons = Vec::new();
        metadata
    }

    fn tooling() -> ToolingOutcome {
        ToolingOutcome {
            state: ResultState::Complete,
            treatment: Some(ToolingTreatment::Amortized),
            tools: Vec::new(),
            products: Vec::new(),
            project_investment: Some(decimal("200000.00")),
            project_amortized_recovered: Some(decimal("180000.00")),
            project_under_over_recovery: Some(decimal("-20000.00")),
            project_separate_charge: None,
            annual_recovery: vec![
                ToolingAnnualRecovery { year: 2026, recovered: Some(decimal("60000.00")) },
                ToolingAnnualRecovery { year: 2027, recovered: Some(decimal("60000.00")) },
                ToolingAnnualRecovery { year: 2028, recovered: Some(decimal("60000.00")) },
            ],
            explanations: Vec::new(),
            issues: Vec::new(),
        }
    }

    fn year_line(year: u16, forecast: u64) -> CommercialYearLine {
        CommercialYearLine {
            year,
            forecast: Some(forecast),
            recurring_unit_cost: Some(decimal("90.00")),
            suggested_base_price: Some(decimal("100.00")),
            manual_base_price: Some(decimal("120.50")),
            tooling_contribution: Some(decimal("3.25")),
            suggested_total_price: Some(decimal("103.25")),
            manual_total_price: Some(decimal("123.75")),
            suggested_unit_ebit_amount: Some(decimal("10.00")),
            suggested_unit_ebit_rate: Some(decimal("0.10")),
            manual_unit_ebit_amount: Some(decimal("30.50")),
            manual_unit_ebit_rate: Some(decimal("0.25")),
            revenue: Some(decimal("1205000.00")),
            ebit_amount: Some(decimal("305000.00")),
            tooling_recovery_revenue: Some(decimal("32500.00")),
        }
    }

    fn commercial_product_line(product_id: Uuid, year_numbers: &[u16], index: usize) -> CommercialProductLine {
        CommercialProductLine {
            product_id,
            product_revision_id: Some(id(500 + index as u128)),
            schedule: YearRange { sop_year: year_numbers.first().copied().unwrap_or(2026), eop_year: year_numbers.last().copied().unwrap_or(2026) },
            manual_price: Some(decimal("120.50")),
            suggested_first_year_price: Some(decimal("100.00")),
            manual_first_year_ebit_amount: Some(decimal("30.50")),
            manual_first_year_ebit_rate: Some(decimal("0.25")),
            tooling_initial_contribution: Some(decimal("3.25")),
            allocated_investment: Some(decimal("5000.00")),
            years: year_numbers.iter().enumerate().map(|(offset, year)| year_line(*year, 1000 * (index as u64 + 1) * (offset as u64 + 1))).collect(),
            explanations: Vec::new(),
        }
    }

    fn scenario(product_ids: &[Uuid], year_numbers: &[u16]) -> ExportScenario {
        ExportScenario {
            scenario_id: id(30),
            version: 4,
            name: "Baseline 2026 price".into(),
            stale: true,
            stale_reasons: vec![CommercialStaleReason::new("saved_baseline_mismatch", "the saved scenario version was pinned to another baseline")],
            outcome: CommercialOutcome {
                state: ResultState::Complete,
                ebit_target_rate: Some(decimal("0.20")),
                annual_productivity_rate: Some(decimal("0.03")),
                productivity_applies_to_tooling: true,
                products: product_ids
                    .iter()
                    .enumerate()
                    .map(|(index, product_id)| commercial_product_line(*product_id, year_numbers, index))
                    .collect(),
                project: CommercialProjectTotals {
                    lifetime_revenue: Some(decimal("14460000.00")),
                    lifetime_ebit_amount: Some(decimal("3660000.00")),
                    years: Vec::new(),
                    tooling_investment: Some(decimal("200000.00")),
                    tooling_separate_charge: None,
                    tooling_projected_recovery: Some(decimal("180000.00")),
                    tooling_under_over_recovery: Some(decimal("-20000.00")),
                },
                tooling: tooling(),
                setup_total: Some(decimal("12000.00")),
                explanations: Vec::new(),
                issues: Vec::new(),
            },
        }
    }

    fn export_product(product_id: Uuid, name: &str, year_numbers: &[u16]) -> ExportProduct {
        ExportProduct {
            product_id,
            name: name.into(),
            kind: ProductKind::Assembly,
            customer_part_number: Some("CPN-100".into()),
            quotation_state: QuotationState::Included,
            product_revision_id: Some(id(500)),
            revision_state: None,
            schedule: YearRange { sop_year: year_numbers.first().copied().unwrap_or(2026), eop_year: year_numbers.last().copied().unwrap_or(2026) },
            forecasts: year_numbers.iter().map(|year| AnnualForecast { year: *year, quantity: 1000 }).collect(),
            quoted: true,
            root_operations: Vec::new(),
            root_attached_costs: Vec::new(),
            parts: Vec::new(),
            lifetime_demand: Some(6000),
            material_cost: Some(decimal("40.00")),
            production_cost: Some(decimal("30.00")),
            attached_cost: Some(decimal("20.00")),
            root_production_cost: Some(decimal("5.00")),
            root_attached_cost: Some(decimal("2.00")),
            categories: Vec::new(),
            unit_total: Some(decimal("92.00")),
            recurring_unit_cost: Some(decimal("90.00")),
        }
    }

    const ISSUE_SOURCES: [ExportIssueSource; 5] = [
        ExportIssueSource::Costing,
        ExportIssueSource::Tooling,
        ExportIssueSource::Commercial,
        ExportIssueSource::Feasibility,
        ExportIssueSource::Staleness,
    ];

    fn model_with(metadata: ExportMetadata, issue_count: usize, product_count: usize, year_count: u16) -> ExportModel {
        let year_numbers: Vec<u16> = (0..year_count).map(|offset| 2026 + offset).collect();
        let product_ids: Vec<Uuid> = (0..product_count).map(|index| if index == 0 { id(7) } else { id(200 + index as u128) }).collect();
        let primary = product_ids.first().copied().unwrap_or_else(|| id(7));
        let project_name = metadata.project_name.clone();
        let customer = metadata.customer.clone();
        let currency = metadata.currency.clone();
        let products: Vec<ExportProduct> = product_ids
            .iter()
            .enumerate()
            .map(|(index, product_id)| {
                let name = if index == 0 { "Housing assembly".to_string() } else { format!("Component family {index}") };
                export_product(*product_id, &name, &year_numbers)
            })
            .collect();
        let scope_product = ExportScopedProduct {
            product_id: primary,
            name: "Housing assembly".into(),
            kind: ProductKind::Assembly,
            customer_part_number: Some("CPN-100".into()),
            quotation_state: QuotationState::Included,
            product_revision_id: Some(id(500)),
            revision_state: None,
            schedule: YearRange { sop_year: year_numbers.first().copied().unwrap_or(2026), eop_year: year_numbers.last().copied().unwrap_or(2026) },
            forecasts: vec![AnnualForecast { year: year_numbers.first().copied().unwrap_or(2026), quantity: 1000 }],
        };
        let issues: Vec<ExportIssue> = (0..issue_count)
            .map(|index| ExportIssue {
                source: ISSUE_SOURCES.get(index % ISSUE_SOURCES.len()).copied().unwrap_or(ExportIssueSource::Costing),
                code: format!("EXPORT_ISSUE_{index:03}"),
                scope: Some(id(1000 + index as u128)),
                message: if index.is_multiple_of(4) {
                    format!(
                        "Missing required input {index}: the stored manufacturing record carries no reviewed volume for this part version, so the pinned snapshot leaves the affected amount missing. The renderer prints the stored explanation as written and never recomputes it."
                    )
                } else {
                    format!(
                        "Unresolved finding {index}: a costing-relevant shared record was published after the selected result read its inputs. This value is stated from the pinned snapshot and stays stale until the result is re-run against the newer record."
                    )
                },
            })
            .collect();
        let scenarios = if product_ids.is_empty() || year_numbers.is_empty() { Vec::new() } else { vec![scenario(&product_ids, &year_numbers)] };
        ExportModel {
            metadata,
            scope: ExportScope {
                project_name,
                customer,
                internal_team: None,
                platform: Some("Platform A".into()),
                application: Some("Application A".into()),
                currency,
                default_schedule: YearRange {
                    sop_year: year_numbers.first().copied().unwrap_or(2026),
                    eop_year: year_numbers.last().copied().unwrap_or(2026),
                },
                products: vec![scope_product],
            },
            parts: Vec::new(),
            products,
            setup: vec![BaselineSetupLine {
                owner: BaselineSetupOwner::Product { product_id: primary },
                setup_charge: Some(decimal("12000.00")),
                explanations: Vec::new(),
            }],
            setup_categories: Vec::new(),
            setup_total: Some(decimal("12000.00")),
            annual: year_numbers
                .iter()
                .map(|year| BaselineAnnualProjectCost { year: *year, manufacturing_cost: Some(decimal("1440000.00")) })
                .collect(),
            project_total_manufacturing_cost: Some(decimal("4321000.50")),
            tooling: Some(tooling()),
            scenarios,
            feasibility: None,
            issues,
        }
    }

    fn fixture(project_name: &str, issue_count: usize, product_count: usize, year_count: u16) -> ExportModel {
        model_with(metadata(project_name), issue_count, product_count, year_count)
    }

    // -- PDF parsing helpers used by the assertions -------------------------

    fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        if needle.is_empty() || haystack.len() < needle.len() {
            return None;
        }
        haystack.windows(needle.len()).position(|window| window == needle)
    }

    fn rfind_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
        if needle.is_empty() || haystack.len() < needle.len() {
            return None;
        }
        haystack.windows(needle.len()).rposition(|window| window == needle)
    }

    fn count_subslice(haystack: &[u8], needle: &[u8]) -> usize {
        if needle.is_empty() || haystack.len() < needle.len() {
            return 0;
        }
        haystack.windows(needle.len()).filter(|window| *window == needle).count()
    }

    fn parse_usize(bytes: &[u8]) -> Option<usize> {
        std::str::from_utf8(bytes).ok()?.trim().parse().ok()
    }

    fn parse_f64(bytes: &[u8]) -> Option<f64> {
        std::str::from_utf8(bytes).ok()?.trim().parse().ok()
    }

    /// Assert the file is a structurally valid PDF and return its page count.
    fn assert_valid_pdf(bytes: &[u8]) -> usize {
        assert!(bytes.starts_with(PDF_HEADER), "the file starts with the PDF header");
        assert!(bytes.ends_with(b"%%EOF\n"), "the file ends with the EOF marker");
        let marker = b"startxref\n";
        let start = find_subslice(bytes, marker).map(|index| index + marker.len()).unwrap_or(0);
        assert!(start > 0, "a startxref keyword is present");
        let digits = bytes.get(start..).map(|rest| rest.iter().take_while(|byte| byte.is_ascii_digit()).count()).unwrap_or(0);
        let xref_offset = bytes.get(start..start + digits).and_then(parse_usize).unwrap_or(0);
        assert!(xref_offset > 0 && xref_offset < bytes.len(), "the startxref offset is inside the file");
        let xref = bytes.get(xref_offset..).unwrap_or_default();
        assert!(xref.starts_with(b"xref\n"), "the startxref offset points at the xref table");
        let body_start = find_subslice(xref, b"xref\n").map(|index| index + b"xref\n".len()).unwrap_or(0);
        let header_end = body_start + find_subslice(xref.get(body_start..).unwrap_or_default(), b"\n").unwrap_or(0);
        let mut header = std::str::from_utf8(xref.get(body_start..header_end).unwrap_or_default()).unwrap_or_default().split_whitespace();
        let first = header.next().and_then(|text| text.parse::<usize>().ok()).unwrap_or(usize::MAX);
        let count = header.next().and_then(|text| text.parse::<usize>().ok()).unwrap_or(0);
        assert_eq!(first, 0, "the xref subsection starts at object 0");
        assert!(count >= 2, "the xref table lists the document objects");
        let entries = header_end + 1;
        for object in 1..count {
            let entry = xref.get(entries + object * 20..entries + object * 20 + 20).unwrap_or_default();
            let offset = entry.get(..10).and_then(parse_usize).unwrap_or(0);
            assert!(offset > 0 && offset < bytes.len(), "object {object} has an in-file offset");
            let expected = format!("{object} 0 obj");
            assert!(
                bytes.get(offset..).is_some_and(|rest| rest.starts_with(expected.as_bytes())),
                "the xref offset of object {object} points at its body"
            );
        }
        let trailer = find_subslice(bytes, b"trailer\n").map(|index| &bytes[index..]).unwrap_or_default();
        assert!(trailer.starts_with(b"trailer\n<< /Size "), "a trailer follows the xref table");
        let size_text = trailer.get(b"trailer\n<< /Size ".len()..).unwrap_or_default();
        let size_digits = size_text.iter().take_while(|byte| byte.is_ascii_digit()).count();
        let declared = size_text.get(..size_digits).and_then(parse_usize).unwrap_or(0);
        assert_eq!(declared, count, "the trailer /Size counts the xref entries");
        let pages = count_subslice(bytes, b"/Type /Page /Parent");
        assert!(pages > 0, "the document has at least one page");
        assert_eq!(count_subslice(bytes, b"/Type /Pages "), 1, "exactly one page tree dictionary exists");
        assert_eq!(count, 5 + pages * 2 + 1, "the object count is five dictionaries plus a page and a stream each");
        assert_eq!(pages, count_subslice(bytes, b"Internal review only - page "), "every page carries the footer");
        for page in 1..=pages {
            let marker = format!("Internal review only - page {page} of {pages}");
            assert_eq!(count_subslice(bytes, marker.as_bytes()), 1, "page {page} states the total page count");
        }
        pages
    }

    struct TextOp {
        x: f64,
        baseline: f64,
        size: f64,
        bold: bool,
        scale: f64,
        text: Vec<u8>,
    }

    fn unescape(bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(bytes.len());
        let mut index = 0;
        while index < bytes.len() {
            if bytes.get(index) == Some(&b'\\') && index + 1 < bytes.len() {
                index += 1;
                out.push(bytes.get(index).copied().unwrap_or_default());
            } else {
                out.push(bytes.get(index).copied().unwrap_or_default());
            }
            index += 1;
        }
        out
    }

    fn parse_text_op(line: &[u8]) -> Option<TextOp> {
        let bt = find_subslice(line, b"BT /")?;
        let font = *line.get(bt + 5)?;
        let tf = find_subslice(line.get(bt..)?, b" Tf")? + bt;
        let size = parse_f64(line.get(bt + 7..tf)?)?;
        let mut cursor = tf + 3;
        let mut scale = 100.0;
        if !line.get(cursor..)?.starts_with(b" 1 0 0 1 ") {
            let tz = find_subslice(line.get(cursor..)?, b" Tz")? + cursor;
            scale = parse_f64(line.get(cursor + 1..tz)?)?;
            cursor = tz + 3;
        }
        let tm = find_subslice(line.get(cursor..)?, b" 1 0 0 1 ")? + cursor;
        let position = tm + b" 1 0 0 1 ".len();
        let x_end = position + line.get(position..)?.iter().take_while(|byte| !byte.is_ascii_whitespace()).count();
        let x = parse_f64(line.get(position..x_end)?)?;
        let y_start = x_end + 1;
        let y_end = find_subslice(line.get(y_start..)?, b" Tm (")? + y_start;
        let baseline = parse_f64(line.get(y_start..y_end)?)?;
        let text_start = y_end + b" Tm (".len();
        let text_end = rfind_subslice(line.get(text_start..)?, b") Tj ET")? + text_start;
        Some(TextOp { x, baseline, size, bold: font == b'2', scale, text: unescape(line.get(text_start..text_end)?) })
    }

    fn text_ops(bytes: &[u8]) -> Vec<TextOp> {
        bytes.split(|byte| *byte == b'\n').filter_map(parse_text_op).collect()
    }

    fn assert_text_inside_margins(bytes: &[u8]) {
        let ops = text_ops(bytes);
        assert!(ops.len() > 30, "the document draws a substantial amount of text");
        for op in &ops {
            let width = text_width(&op.text, op.size, op.bold) * op.scale / 100.0;
            let shown = String::from_utf8_lossy(&op.text);
            assert!(op.x >= LEFT - 0.02, "text starts inside the left margin: {shown}");
            assert!(op.x + width <= RIGHT + 0.02, "text ends inside the right margin: {shown}");
            assert!(op.baseline >= 10.0 && op.baseline <= PAGE_HEIGHT, "text is drawn on the page: {shown}");
        }
    }

    fn right_edge_of_line_containing(bytes: &[u8], needle: &str) -> Option<f64> {
        text_ops(bytes).iter().find_map(|op| {
            if find_subslice(&op.text, needle.as_bytes()).is_some() {
                Some(op.x + text_width(&op.text, op.size, op.bold) * op.scale / 100.0)
            } else {
                None
            }
        })
    }

    // -- tests --------------------------------------------------------------

    #[test]
    fn a_minimal_export_is_a_valid_pdf_with_correct_xref_offsets() {
        let model = fixture("Three boxes", 3, 1, 3);
        let bytes = render(&model);
        let pages = assert_valid_pdf(&bytes);
        assert!(pages >= 2, "the summary spans several pages, got {pages}");
        assert!(count_subslice(&bytes, b"Internal export") >= 1, "the title is drawn");
        assert!(count_subslice(&bytes, b"Three boxes") >= 1, "the project name is drawn");
        assert!(count_subslice(&bytes, b"Draft calculation ") >= 1, "the selected draft result is named");
        assert!(count_subslice(&bytes, b"Accepted baseline") == 0, "an accepted baseline is not named");
        assert!(count_subslice(&bytes, b"4,321,000.50") == 1, "money is grouped and exact");
        assert!(count_subslice(&bytes, b"DRAFT RESULT") >= 2, "the draft label is in the status band and the footer");
        assert!(count_subslice(&bytes, b"INCOMPLETE") >= 2, "the incomplete label is in the status band and the footer");
    }

    #[test]
    fn text_outside_winansi_is_written_as_a_question_mark() {
        let model = fixture("Böx 项目 Ω", 1, 1, 2);
        let bytes = render(&model);
        assert_valid_pdf(&bytes);
        assert!(count_subslice(&bytes, b"B\xF6x") >= 1, "an accented letter uses its WinAnsi byte");
        assert!(count_subslice(bytes.as_slice(), "项目".as_bytes()) == 0, "CJK text is not written as UTF-8");
        assert!(count_subslice(bytes.as_slice(), "\u{03A9}".as_bytes()) == 0, "Greek text is not written as UTF-8");
        assert!(count_subslice(&bytes, b"B\xF6x ?? ?") >= 1, "unsupported characters become question marks");
    }

    #[test]
    fn a_long_unbroken_label_wraps_inside_its_column() {
        let long_name = "A very long product family name that keeps going well past the width of its table column";
        let model = fixture(long_name, 1, 1, 2);
        let bytes = render(&model);
        assert_valid_pdf(&bytes);
        assert_text_inside_margins(&bytes);
    }

    #[test]
    fn a_large_issue_set_paginates_and_grows_the_page_count() {
        let small = fixture("Three boxes", 0, 1, 2);
        let large = fixture("Three boxes", 500, 1, 2);
        let small_pages = assert_valid_pdf(&render(&small));
        let large_bytes = render(&large);
        let large_pages = assert_valid_pdf(&large_bytes);
        assert!(large_pages > small_pages, "500 issues need more pages: {small_pages} -> {large_pages}");
        assert!(large_pages >= 10, "500 issue rows span many pages, got {large_pages}");
        assert!(count_subslice(&large_bytes, b"EXPORT_ISSUE_000") == 1, "the first issue code is listed");
        assert!(count_subslice(&large_bytes, b"EXPORT_ISSUE_499") == 1, "the last issue code is listed");
        assert!(count_subslice(&large_bytes, b"Message") >= 2, "the issue table header repeats on continuation pages");
        assert_text_inside_margins(&large_bytes);
    }

    #[test]
    fn many_product_year_rows_grow_the_page_count() {
        let small = fixture("Three boxes", 1, 1, 2);
        let large = fixture("Three boxes", 1, 10, 6);
        let small_pages = assert_valid_pdf(&render(&small));
        let large_bytes = render(&large);
        let large_pages = assert_valid_pdf(&large_bytes);
        assert!(large_pages > small_pages, "60 product-year rows need more pages: {small_pages} -> {large_pages}");
        assert!(count_subslice(&large_bytes, b"Component family 9") >= 1, "the last product is listed");
        assert_text_inside_margins(&large_bytes);
    }

    #[test]
    fn a_single_line_cell_stays_inside_its_column() {
        let model = fixture("Three boxes", 1, 1, 2);
        let bytes = render(&model);
        assert_valid_pdf(&bytes);
        let scope_edge = right_edge_of_line_containing(&bytes, "00000000-0000-0000-0000-0000000003e8");
        assert!(scope_edge.is_some(), "the issue scope id is drawn");
        assert!(scope_edge.unwrap_or_default() <= 192.0 + 126.0 - CELL_PADDING + 0.02, "the scope id ends inside its column, got {scope_edge:?}");
    }

    #[test]
    fn money_is_right_aligned_in_its_column() {
        let model = fixture("Three boxes", 1, 1, 2);
        let bytes = render(&model);
        assert_valid_pdf(&bytes);
        let revenue_edge = right_edge_of_line_containing(&bytes, "1,205,000.00");
        assert!(revenue_edge.is_some(), "the revenue amount is drawn");
        assert!((revenue_edge.unwrap_or_default() - 358.0).abs() < 0.02, "the revenue amount ends at its column edge, got {revenue_edge:?}");
        assert!(count_subslice(&bytes, b"missing") >= 1, "a missing value is named, never zero");
    }

    #[test]
    fn wrapped_lines_never_exceed_the_width_they_wrap_to() {
        let samples =
            ["Housing assembly", "averylongsinglewordwithoutanyspacesatallthatmustbesplitacrossseverallines", "项目 mixed unicode Ω text", ""];
        for sample in samples {
            for width in [20.0_f64, 60.0, 150.0] {
                let lines = wrap(&encode(sample), 7.0, false, width);
                assert!(!lines.is_empty(), "wrapping always returns a line");
                for line in &lines {
                    assert!(text_width(line, 7.0, false) <= width + 0.01, "line fits: {line:?} at {width}");
                }
            }
        }
    }

    #[test]
    fn known_values_are_extracted_from_the_content_streams() {
        let model = fixture("Three boxes", 2, 1, 3);
        let bytes = render(&model);
        assert_valid_pdf(&bytes);
        let extracted = text_ops(&bytes).iter().map(|op| String::from_utf8_lossy(&op.text).into_owned()).collect::<Vec<_>>().join("\n");
        assert!(extracted.contains("Internal export"), "the title is extractable");
        assert!(extracted.contains("Three boxes"), "the project name is extractable");
        assert!(extracted.contains("4,321,000.50"), "the lifetime total is extractable");
        assert!(extracted.contains("1,205,000.00"), "a scenario revenue is extractable");
        assert!(extracted.contains("DRAFT RESULT"), "the status band is extractable");
        assert!(extracted.contains("Internal review only - page 1 of "), "the footer is extractable");
    }

    #[test]
    fn an_accepted_baseline_names_its_sequence_and_no_draft_marker() {
        let model = model_with(accepted_metadata("Line 4 programme"), 2, 1, 3);
        let bytes = render(&model);
        assert_valid_pdf(&bytes);
        assert!(count_subslice(&bytes, b"Accepted baseline #7") >= 1, "the accepted sequence is named");
        assert!(count_subslice(&bytes, b"Draft calculation ") == 0, "no draft calculation is named");
        assert!(count_subslice(&bytes, b"DRAFT RESULT") == 0, "no draft label is written");
        assert!(count_subslice(&bytes, b"read at export time") == 0, "the pinned feasibility record is not read at export time");
        assert!(count_subslice(&bytes, b"Feasibility record") >= 1, "the feasibility version is named");
        assert_text_inside_margins(&bytes);
    }

    #[test]
    fn an_export_without_scenarios_tooling_or_issues_still_renders() {
        let mut model = fixture("Empty programme", 0, 1, 2);
        model.scenarios = Vec::new();
        model.tooling = None;
        model.setup = Vec::new();
        model.setup_total = None;
        model.annual = Vec::new();
        model.project_total_manufacturing_cost = None;
        let bytes = render(&model);
        assert_valid_pdf(&bytes);
        assert!(count_subslice(&bytes, b"No saved commercial scenario") >= 1, "the empty scenario section is stated");
        assert!(count_subslice(&bytes, b"No tooling evaluation") >= 1, "the empty tooling section is stated");
        assert!(count_subslice(&bytes, b"No separate setup line") >= 1, "the empty setup section is stated");
        assert!(count_subslice(&bytes, b"No unresolved issue") >= 1, "the empty issue section is stated");
        assert!(count_subslice(&bytes, b"missing") >= 2, "a missing total is named, never zero");
        assert_text_inside_margins(&bytes);
    }

    /// Writes a rendered fixture to `target/` so an independent parser can check the document
    /// outside the unit tests. Ignored by default because it writes an evidence artifact.
    #[test]
    #[ignore = "writes an evidence PDF into target/ on demand"]
    fn write_evidence_pdf_for_an_independent_parser() {
        let model = fixture("Three boxes", 12, 3, 5);
        let bytes = render(&model);
        assert_valid_pdf(&bytes);
        let path = std::path::Path::new("../../target/solidbom-export-pdf-evidence.pdf");
        assert!(std::fs::write(path, &bytes).is_ok(), "the evidence PDF is written");
        let large = fixture("Three boxes", 500, 3, 5);
        let large_bytes = render(&large);
        assert_valid_pdf(&large_bytes);
        let large_path = std::path::Path::new("../../target/solidbom-export-pdf-large-issues.pdf");
        assert!(std::fs::write(large_path, &large_bytes).is_ok(), "the large-issue evidence PDF is written");
    }

    #[test]
    fn two_renders_of_one_model_are_byte_identical() {
        let model = fixture("Three boxes", 17, 3, 4);
        assert_eq!(render(&model), render(&model));
    }
}
