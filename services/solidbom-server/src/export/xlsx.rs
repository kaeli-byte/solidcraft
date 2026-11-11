//! The detailed internal Excel workbook renderer (package SB-21).
//!
//! The caller is [`super::resolve`]; the values come from [`super::model::ExportModel`]. This module
//! writes a minimal Office Open XML spreadsheet workbook: the metadata summary, the BOM, routings,
//! feasibility, detailed costs and the commercial scenarios, one sheet each. It performs no
//! business arithmetic: every number and label is copied from the model, and this module only
//! decides where a value sits, how it is formatted and how it is escaped.
//!
//! The package is written by hand because the service must not gain a spreadsheet dependency:
//!
//! - `[Content_Types].xml`, `_rels/.rels` and the two property parts name every member;
//! - `xl/workbook.xml` lists the sheets, `xl/_rels/workbook.xml.rels` binds them to the worksheets;
//! - `xl/styles.xml` carries the header, money, quantity, percentage and wrapping formats;
//! - `xl/worksheets/sheetN.xml` hold one sheet each, with inline strings and numeric cells only (no
//!   shared-string table and no formulas, so a later saved workbook cannot change its own values).
//!
//! Text is escaped and invalid XML control characters are replaced with U+FFFD. Numbers are written
//! as numeric cells so Excel and independent parsers read them as numbers, and a missing value is a
//! blank cell, never a zero. Year columns are the years actually present in the model data.
//!
//! Every byte is a pure function of the model: entries are written in a fixed order, the ZIP
//! container has a fixed timestamp (see [`super::zip`]), collections are ordered by contract, and no
//! wall clock, random value or hash-map iteration is read.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use rust_decimal::Decimal;
use solidbom_contract::{
    AnnualForecast, AppliedRule, BaselineAnnualDemand, BaselineCategoryLine, BaselinePurchasePriceSource, BaselineSetupLine, BaselineSetupOwner,
    CommercialStaleReason, CycleBasis, ExceptionState, ExportBaselineSelection, ExportLabel, ExportStaleReason, FeasibilityExceptionView,
    FeasibilityFinding, FindingCheck, FindingDisposition, FindingState, ProductKind, ProductRevisionState, QuotationState, ResultState, RuleLimit,
    Sourcing, SupportingMeasurement, ToolingOwner, ToolingTreatment, TubeEnd, Unit, UnresolvedReason, YearRange,
};
use uuid::Uuid;

use super::model::{ExportModel, ExportOperation, ExportPart, ExportProduct};

// ---------------------------------------------------------------------------
// XML constants
// ---------------------------------------------------------------------------

/// The XML declaration every part starts with.
const XML_DECLARATION: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";
/// The spreadsheet main namespace.
const MAIN_NAMESPACE: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
/// The package relationship namespace.
const RELATIONSHIP_NAMESPACE: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
/// The document relationship namespace used in workbook relationship types.
const DOCUMENT_RELATIONSHIP_NAMESPACE: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
/// The content-types namespace.
const CONTENT_TYPES_NAMESPACE: &str = "http://schemas.openxmlformats.org/package/2006/content-types";

// ---------------------------------------------------------------------------
// Cell styles (indices into `xl/styles.xml` `cellXfs`)
// ---------------------------------------------------------------------------

const STYLE_NORMAL: u16 = 0;
const STYLE_HEADER: u16 = 1;
const STYLE_MONEY: u16 = 2;
const STYLE_QUANTITY: u16 = 3;
const STYLE_PERCENT: u16 = 4;
const STYLE_WRAP: u16 = 5;
const STYLE_SECTION: u16 = 6;
const STYLE_DECIMAL: u16 = 7;
const STYLE_INTEGER: u16 = 8;
const STYLE_BOLD: u16 = 9;

// ---------------------------------------------------------------------------
// Workbook rendering
// ---------------------------------------------------------------------------

/// Render the detailed internal workbook for one resolved export model.
pub(super) fn render(model: &ExportModel) -> Vec<u8> {
    let sheets = build_sheets(model);
    let mut files: Vec<(String, String)> = Vec::with_capacity(7 + sheets.len());
    files.push(("[Content_Types].xml".to_string(), content_types(sheets.len())));
    files.push(("_rels/.rels".to_string(), root_relationships()));
    files.push(("docProps/core.xml".to_string(), core_properties(model)));
    files.push(("docProps/app.xml".to_string(), application_properties()));
    files.push(("xl/workbook.xml".to_string(), workbook(sheets.as_slice())));
    files.push(("xl/_rels/workbook.xml.rels".to_string(), workbook_relationships(sheets.len())));
    files.push(("xl/styles.xml".to_string(), STYLES.to_string()));
    for (index, sheet) in sheets.iter().enumerate() {
        files.push((format!("xl/worksheets/sheet{}.xml", index + 1), sheet.serialise(index == 0)));
    }
    let entries: Vec<(&str, &[u8])> = files.iter().map(|(name, data)| (name.as_str(), data.as_bytes())).collect();
    super::zip::archive(&entries)
}

/// The package content types: one override per worksheet plus the workbook, styles and properties.
fn content_types(sheet_count: usize) -> String {
    let mut xml = String::from(XML_DECLARATION);
    let _ = write!(xml, "<Types xmlns=\"{CONTENT_TYPES_NAMESPACE}\">");
    xml.push_str("<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>");
    xml.push_str("<Default Extension=\"xml\" ContentType=\"application/xml\"/>");
    xml.push_str(
        "<Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/>",
    );
    xml.push_str("<Override PartName=\"/xl/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml\"/>");
    xml.push_str("<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>");
    xml.push_str("<Override PartName=\"/docProps/app.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.extended-properties+xml\"/>");
    for index in 1..=sheet_count {
        let _ = write!(
            xml,
            "<Override PartName=\"/xl/worksheets/sheet{index}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/>"
        );
    }
    xml.push_str("</Types>");
    xml
}

/// The package root relationships: the workbook and the two property parts.
fn root_relationships() -> String {
    let mut xml = String::from(XML_DECLARATION);
    let _ = write!(xml, "<Relationships xmlns=\"{RELATIONSHIP_NAMESPACE}\">");
    let _ = write!(xml, "<Relationship Id=\"rId1\" Type=\"{DOCUMENT_RELATIONSHIP_NAMESPACE}/officeDocument\" Target=\"xl/workbook.xml\"/>");
    let _ = write!(
        xml,
        "<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>"
    );
    let _ = write!(xml, "<Relationship Id=\"rId3\" Type=\"{DOCUMENT_RELATIONSHIP_NAMESPACE}/extended-properties\" Target=\"docProps/app.xml\"/>");
    xml.push_str("</Relationships>");
    xml
}

/// The document properties: title, creator and the model's recorded export timestamp.
fn core_properties(model: &ExportModel) -> String {
    let mut xml = String::from(XML_DECLARATION);
    xml.push_str(
        "<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
         xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" \
         xmlns:dcmitype=\"http://purl.org/dc/dcmitype/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">",
    );
    let _ = write!(xml, "<dc:title>{}</dc:title>", escape_xml(&model.metadata.project_name));
    xml.push_str("<dc:creator>SolidBOM</dc:creator>");
    xml.push_str("<cp:lastModifiedBy>SolidBOM</cp:lastModifiedBy>");
    let timestamp = escape_xml(model.metadata.exported_at.trim());
    if !timestamp.is_empty() {
        let _ = write!(xml, "<dcterms:created xsi:type=\"dcterms:W3CDTF\">{timestamp}</dcterms:created>");
        let _ = write!(xml, "<dcterms:modified xsi:type=\"dcterms:W3CDTF\">{timestamp}</dcterms:modified>");
    }
    xml.push_str("</cp:coreProperties>");
    xml
}

/// The extended application properties Excel expects beside a workbook.
fn application_properties() -> String {
    let mut xml = String::from(XML_DECLARATION);
    xml.push_str("<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\">");
    xml.push_str("<Application>SolidBOM</Application>");
    xml.push_str("</Properties>");
    xml
}

/// The workbook part: sheet order, the selected first tab and the repeated header rows.
fn workbook(sheets: &[Sheet]) -> String {
    let mut xml = String::from(XML_DECLARATION);
    let _ = write!(xml, "<workbook xmlns=\"{MAIN_NAMESPACE}\" xmlns:r=\"{DOCUMENT_RELATIONSHIP_NAMESPACE}\">");
    xml.push_str("<bookViews><workbookView xWindow=\"0\" yWindow=\"0\" windowWidth=\"24000\" windowHeight=\"12000\"/></bookViews>");
    xml.push_str("<sheets>");
    for (index, sheet) in sheets.iter().enumerate() {
        let _ = write!(xml, "<sheet name=\"{}\" sheetId=\"{}\" r:id=\"rId{}\"/>", escape_xml(sheet.name), index + 1, index + 1);
    }
    xml.push_str("</sheets>");
    let print_titles: Vec<(usize, &Sheet)> = sheets.iter().enumerate().filter(|(_, sheet)| sheet.print_titles).collect();
    if !print_titles.is_empty() {
        xml.push_str("<definedNames>");
        for (index, sheet) in print_titles {
            let _ = write!(xml, "<definedName name=\"_xlnm.Print_Titles\" localSheetId=\"{index}\">'{}'!$1:$1</definedName>", escape_xml(sheet.name));
        }
        xml.push_str("</definedNames>");
    }
    xml.push_str("<calcPr calcId=\"191029\"/>");
    xml.push_str("</workbook>");
    xml
}

/// The workbook relationships: one worksheet per sheet and the styles part last.
fn workbook_relationships(sheet_count: usize) -> String {
    let mut xml = String::from(XML_DECLARATION);
    let _ = write!(xml, "<Relationships xmlns=\"{RELATIONSHIP_NAMESPACE}\">");
    for index in 1..=sheet_count {
        let _ = write!(
            xml,
            "<Relationship Id=\"rId{index}\" Type=\"{DOCUMENT_RELATIONSHIP_NAMESPACE}/worksheet\" Target=\"worksheets/sheet{index}.xml\"/>"
        );
    }
    let _ = write!(xml, "<Relationship Id=\"rId{}\" Type=\"{DOCUMENT_RELATIONSHIP_NAMESPACE}/styles\" Target=\"styles.xml\"/>", sheet_count + 1);
    xml.push_str("</Relationships>");
    xml
}

/// The style sheet: the header band, the number formats and the wrapping formats.
const STYLES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>
<styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">
<numFmts count=\"4\">
<numFmt numFmtId=\"164\" formatCode=\"#,##0.00\"/>
<numFmt numFmtId=\"165\" formatCode=\"#,##0\"/>
<numFmt numFmtId=\"166\" formatCode=\"0.0000\"/>
<numFmt numFmtId=\"167\" formatCode=\"#,##0.0000\"/>
</numFmts>
<fonts count=\"2\">
<font><sz val=\"11\"/><color theme=\"1\"/><name val=\"Calibri\"/><family val=\"2\"/></font>
<font><b/><sz val=\"11\"/><color theme=\"1\"/><name val=\"Calibri\"/><family val=\"2\"/></font>
</fonts>
<fills count=\"4\">
<fill><patternFill patternType=\"none\"/></fill>
<fill><patternFill patternType=\"gray125\"/></fill>
<fill><patternFill patternType=\"solid\"><fgColor rgb=\"FFD9E1F2\"/><bgColor indexed=\"64\"/></patternFill></fill>
<fill><patternFill patternType=\"solid\"><fgColor rgb=\"FFF2F2F2\"/><bgColor indexed=\"64\"/></patternFill></fill>
</fills>
<borders count=\"2\">
<border><left/><right/><top/><bottom/><diagonal/></border>
<border><left style=\"thin\"><color rgb=\"FF808080\"/></left><right style=\"thin\"><color rgb=\"FF808080\"/></right><top style=\"thin\"><color rgb=\"FF808080\"/></top><bottom style=\"thin\"><color rgb=\"FF808080\"/></bottom><diagonal/></border>
</borders>
<cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>
<cellXfs count=\"10\">
<xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\"/>
<xf numFmtId=\"0\" fontId=\"1\" fillId=\"2\" borderId=\"1\" xfId=\"0\" applyFont=\"1\" applyFill=\"1\" applyBorder=\"1\" applyAlignment=\"1\"><alignment horizontal=\"center\" vertical=\"center\" wrapText=\"1\"/></xf>
<xf numFmtId=\"164\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\" applyAlignment=\"1\"><alignment horizontal=\"right\" vertical=\"top\"/></xf>
<xf numFmtId=\"165\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\" applyAlignment=\"1\"><alignment horizontal=\"right\" vertical=\"top\"/></xf>
<xf numFmtId=\"166\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\" applyAlignment=\"1\"><alignment horizontal=\"right\" vertical=\"top\"/></xf>
<xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyAlignment=\"1\"><alignment horizontal=\"left\" vertical=\"top\" wrapText=\"1\"/></xf>
<xf numFmtId=\"0\" fontId=\"1\" fillId=\"3\" borderId=\"1\" xfId=\"0\" applyFont=\"1\" applyFill=\"1\" applyBorder=\"1\" applyAlignment=\"1\"><alignment horizontal=\"left\" vertical=\"center\"/></xf>
<xf numFmtId=\"167\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\" applyAlignment=\"1\"><alignment horizontal=\"right\" vertical=\"top\"/></xf>
<xf numFmtId=\"1\" fontId=\"0\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyNumberFormat=\"1\" applyAlignment=\"1\"><alignment horizontal=\"right\" vertical=\"top\"/></xf>
<xf numFmtId=\"0\" fontId=\"1\" fillId=\"0\" borderId=\"0\" xfId=\"0\" applyFont=\"1\" applyAlignment=\"1\"><alignment horizontal=\"left\" vertical=\"top\"/></xf>
</cellXfs>
<cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles>
</styleSheet>";

// ---------------------------------------------------------------------------
// XML escaping
// ---------------------------------------------------------------------------

/// Escape text for XML text nodes and attribute values, replacing characters XML 1.0 forbids.
///
/// All five markup characters are escaped (also inside text nodes), so a reader never has to
/// distinguish context. Characters outside the XML 1.0 character range, including the C0 control
/// characters a stored text should never carry, become U+FFFD instead of producing an invalid part.
fn escape_xml(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            character if is_xml_character(character) => escaped.push(character),
            _ => escaped.push('\u{fffd}'),
        }
    }
    escaped
}

/// Whether one character is allowed everywhere in an XML 1.0 document.
fn is_xml_character(character: char) -> bool {
    matches!(character, '\u{9}' | '\u{a}' | '\u{d}')
        || ('\u{20}'..='\u{d7ff}').contains(&character)
        || ('\u{e000}'..='\u{fffd}').contains(&character)
        || ('\u{10000}'..='\u{10ffff}').contains(&character)
}

/// The spreadsheet column name of a zero-based column index (`0` is `A`, `26` is `AA`).
fn column_name(mut index: usize) -> String {
    let mut name = String::new();
    loop {
        let remainder = (index % 26) as u8;
        name.insert(0, char::from(b'A' + remainder));
        if index < 26 {
            return name;
        }
        index = index / 26 - 1;
    }
}

// ---------------------------------------------------------------------------
// Cells, rows and sheets
// ---------------------------------------------------------------------------

/// One cell's value: an inline string or a numeric literal.
enum CellValue {
    Text(String),
    Number(String),
}

/// One cell: its zero-based column, its style index and its value.
struct Cell {
    column: usize,
    style: u16,
    value: CellValue,
}

impl Cell {
    /// The cell part in row `number` (one-based).
    fn serialise(&self, number: usize) -> String {
        let reference = format!("{}{number}", column_name(self.column));
        match &self.value {
            CellValue::Text(text) => {
                format!(
                    "<c r=\"{reference}\" s=\"{style}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{text}</t></is></c>",
                    style = self.style,
                    text = escape_xml(text),
                )
            }
            CellValue::Number(value) => format!("<c r=\"{reference}\" s=\"{}\"><v>{value}</v></c>", self.style),
        }
    }
}

/// One worksheet row under construction; each push advances one column.
struct Row {
    cells: Vec<Cell>,
    next_column: usize,
    height: Option<u16>,
}

impl Row {
    fn new() -> Self {
        Self { cells: Vec::new(), next_column: 0, height: None }
    }

    /// A blank spacer row.
    fn blank() -> Self {
        Self::new()
    }

    /// A row with a fixed height in points.
    fn with_height(mut self, height: u16) -> Self {
        self.height = Some(height);
        self
    }

    /// Leave the current column blank and advance to the next one.
    fn skip(&mut self) {
        self.next_column += 1;
    }

    fn push(&mut self, style: u16, value: CellValue) {
        self.cells.push(Cell { column: self.next_column, style, value });
        self.next_column += 1;
    }

    fn text(&mut self, text: impl Into<String>) {
        self.push(STYLE_NORMAL, CellValue::Text(text.into()));
    }

    fn text_opt(&mut self, text: Option<&str>) {
        match text {
            Some(text) => self.text(text),
            None => self.skip(),
        }
    }

    fn wrap(&mut self, text: impl Into<String>) {
        self.push(STYLE_WRAP, CellValue::Text(text.into()));
    }

    fn wrap_opt(&mut self, text: Option<&str>) {
        match text {
            Some(text) => self.wrap(text),
            None => self.skip(),
        }
    }

    fn header(&mut self, text: &str) {
        self.push(STYLE_HEADER, CellValue::Text(text.to_string()));
    }

    fn bold(&mut self, text: impl Into<String>) {
        self.push(STYLE_BOLD, CellValue::Text(text.into()));
    }

    fn money(&mut self, value: Option<Decimal>) {
        match value {
            Some(value) => self.push(STYLE_MONEY, CellValue::Number(decimal_text(value))),
            None => self.skip(),
        }
    }

    fn decimal(&mut self, value: Option<Decimal>) {
        match value {
            Some(value) => self.push(STYLE_DECIMAL, CellValue::Number(decimal_text(value))),
            None => self.skip(),
        }
    }

    fn percent(&mut self, value: Option<Decimal>) {
        match value {
            Some(value) => self.push(STYLE_PERCENT, CellValue::Number(decimal_text(value))),
            None => self.skip(),
        }
    }

    fn quantity(&mut self, value: Option<u64>) {
        match value {
            Some(value) => self.push(STYLE_QUANTITY, CellValue::Number(value.to_string())),
            None => self.skip(),
        }
    }

    fn integer(&mut self, value: u64) {
        self.push(STYLE_INTEGER, CellValue::Number(value.to_string()));
    }

    fn integer_opt(&mut self, value: Option<u64>) {
        match value {
            Some(value) => self.integer(value),
            None => self.skip(),
        }
    }
}

/// The decimal string Excel reads in a numeric cell, without grouping or exponent formatting.
fn decimal_text(value: Decimal) -> String {
    value.to_string()
}

/// One worksheet with its rows, column widths and table behaviour.
struct Sheet {
    name: &'static str,
    rows: Vec<Row>,
    widths: Vec<u16>,
    freeze_rows: usize,
    autofilter: bool,
    print_titles: bool,
}

impl Sheet {
    fn new(name: &'static str, widths: Vec<u16>, freeze_rows: usize, autofilter: bool, print_titles: bool) -> Self {
        Self { name, rows: Vec::new(), widths, freeze_rows, autofilter, print_titles }
    }

    fn push(&mut self, row: Row) {
        self.rows.push(row);
    }

    /// A key column text row of the metadata-style sheets.
    fn property_text(&mut self, key: &str, value: impl Into<String>) {
        let mut row = Row::new();
        row.bold(key.to_string());
        row.wrap(value.into());
        self.push(row);
    }

    /// A key column integer row: a version, a sequence or a count.
    fn property_integer(&mut self, key: &str, value: u64) {
        let mut row = Row::new();
        row.bold(key.to_string());
        row.integer(value);
        self.push(row);
    }

    /// Remove the blank spacer rows a block sheet left at its end.
    fn finish(mut self) -> Self {
        while self.rows.last().is_some_and(|row| row.cells.is_empty()) {
            self.rows.pop();
        }
        self
    }

    /// The number of columns that hold a cell, at least one.
    fn column_count(&self) -> usize {
        self.rows.iter().map(|row| row.cells.iter().map(|cell| cell.column + 1).max().unwrap_or(0)).max().unwrap_or(0).max(1)
    }

    /// The estimated row height for wrapped text, so a long message is readable when it opens.
    ///
    /// Excel sizes rows from their content only when it is asked to; a wrapped cell in a row with
    /// the default height is clipped. This is a display estimate, not a value calculation: one
    /// character per column-width unit, capped at twelve lines and 160 points.
    fn wrapped_height(&self, row: &Row) -> Option<u16> {
        let mut lines = 1usize;
        for cell in &row.cells {
            if cell.style != STYLE_WRAP {
                continue;
            }
            let CellValue::Text(text) = &cell.value else { continue };
            let width = usize::from(self.widths.get(cell.column).copied().unwrap_or(10));
            if width == 0 {
                continue;
            }
            lines = lines.max(text.chars().count().div_ceil(width));
        }
        if lines <= 1 {
            return None;
        }
        let extra_lines = u16::try_from(lines - 1).unwrap_or(u16::MAX).min(12);
        Some(15u16.saturating_add(14u16.saturating_mul(extra_lines)).min(160))
    }

    /// Serialise the worksheet part.
    fn serialise(&self, selected: bool) -> String {
        let columns = self.column_count();
        let rows = self.rows.len();
        let mut xml = String::with_capacity(1024 + rows * 80);
        xml.push_str(XML_DECLARATION);
        let _ = write!(xml, "<worksheet xmlns=\"{MAIN_NAMESPACE}\">");
        xml.push_str("<sheetPr><pageSetUpPr fitToPage=\"1\"/></sheetPr>");
        let _ = write!(xml, "<dimension ref=\"A1:{}{}\"/>", column_name(columns - 1), rows);
        xml.push_str("<sheetViews><sheetView workbookViewId=\"0\"");
        if selected {
            xml.push_str(" tabSelected=\"1\"");
        }
        xml.push('>');
        if self.freeze_rows > 0 && rows > self.freeze_rows {
            let frozen = self.freeze_rows;
            let _ = write!(xml, "<pane ySplit=\"{frozen}\" topLeftCell=\"A{}\" activePane=\"bottomLeft\" state=\"frozen\"/>", frozen + 1);
            let _ = write!(xml, "<selection pane=\"bottomLeft\" activeCell=\"A{}\" sqref=\"A{}\"/>", frozen + 1, frozen + 1);
        }
        xml.push_str("</sheetView></sheetViews>");
        xml.push_str("<sheetFormatPr defaultRowHeight=\"15\"/>");
        if !self.widths.is_empty() {
            xml.push_str("<cols>");
            for (index, width) in self.widths.iter().enumerate() {
                let column = index + 1;
                let _ = write!(xml, "<col min=\"{column}\" max=\"{column}\" width=\"{width}\" customWidth=\"1\"/>");
            }
            xml.push_str("</cols>");
        }
        xml.push_str("<sheetData>");
        for (index, row) in self.rows.iter().enumerate() {
            let number = index + 1;
            if row.cells.is_empty() {
                let _ = write!(xml, "<row r=\"{number}\"/>");
                continue;
            }
            let _ = write!(xml, "<row r=\"{number}\"");
            if let Some(height) = row.height.or_else(|| self.wrapped_height(row)) {
                let _ = write!(xml, " ht=\"{height}\" customHeight=\"1\"");
            }
            xml.push('>');
            for cell in &row.cells {
                xml.push_str(&cell.serialise(number));
            }
            xml.push_str("</row>");
        }
        xml.push_str("</sheetData>");
        if self.autofilter && rows > 1 {
            let _ = write!(xml, "<autoFilter ref=\"A1:{}{}\"/>", column_name(columns - 1), rows);
        }
        xml.push_str("<pageMargins left=\"0.4\" right=\"0.4\" top=\"0.5\" bottom=\"0.5\" header=\"0.2\" footer=\"0.2\"/>");
        xml.push_str("<pageSetup orientation=\"landscape\" fitToWidth=\"1\" fitToHeight=\"0\" paperSize=\"9\"/>");
        xml.push_str("</worksheet>");
        xml
    }
}

// ---------------------------------------------------------------------------
// Text helpers
// ---------------------------------------------------------------------------

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn product_kind_text(kind: ProductKind) -> &'static str {
    match kind {
        ProductKind::Part => "part",
        ProductKind::Assembly => "assembly",
    }
}

fn quotation_state_text(state: QuotationState) -> &'static str {
    match state {
        QuotationState::Included => "included",
        QuotationState::Excluded => "excluded",
    }
}

fn revision_state_text(state: ProductRevisionState) -> &'static str {
    match state {
        ProductRevisionState::Draft => "draft",
        ProductRevisionState::Active => "active",
        ProductRevisionState::Historical => "historical",
    }
}

fn sourcing_text(sourcing: Sourcing) -> &'static str {
    match sourcing {
        Sourcing::Purchased => "purchased",
        Sourcing::Manufactured => "manufactured",
    }
}

fn result_state_text(state: ResultState) -> &'static str {
    match state {
        ResultState::Incomplete => "incomplete",
        ResultState::Complete => "complete",
    }
}

fn cycle_basis_text(basis: CycleBasis) -> &'static str {
    match basis {
        CycleBasis::PerPiece => "per_piece",
        CycleBasis::PerCycle => "per_cycle",
    }
}

fn tooling_treatment_text(treatment: Option<ToolingTreatment>) -> Option<&'static str> {
    match treatment {
        Some(ToolingTreatment::Separate) => Some("separate"),
        Some(ToolingTreatment::Amortized) => Some("amortized"),
        None => None,
    }
}

fn tube_end_text(end: TubeEnd) -> &'static str {
    match end {
        TubeEnd::Start => "start",
        TubeEnd::End => "end",
    }
}

fn finding_state_text(state: FindingState) -> &'static str {
    match state {
        FindingState::Pass => "pass",
        FindingState::Fail => "fail",
        FindingState::Unresolved => "unresolved",
        FindingState::Conflict => "conflict",
    }
}

fn unresolved_reason_text(reason: UnresolvedReason) -> &'static str {
    match reason {
        UnresolvedReason::MissingMeasurement => "missing_measurement",
        UnresolvedReason::MissingConnectorAssociation => "missing_connector_association",
        UnresolvedReason::NoApplicableLimit => "no_applicable_limit",
        UnresolvedReason::LimitNotComputable => "limit_not_computable",
    }
}

fn exception_state_text(state: ExceptionState) -> &'static str {
    match state {
        ExceptionState::Active => "active",
        ExceptionState::ReviewRequired => "review_required",
        ExceptionState::Closed => "closed",
    }
}

/// The selected saved result as the summary states it.
fn selected_result_text(baseline: &ExportBaselineSelection) -> String {
    match baseline {
        ExportBaselineSelection::AcceptedBaseline { accepted_baseline_id, sequence } => {
            format!("accepted baseline #{sequence} ({accepted_baseline_id})")
        }
        ExportBaselineSelection::DraftCalculation { calculation_id } => format!("draft calculation ({calculation_id})"),
    }
}

/// The freshness labels in contract order, as their stable names.
fn labels_text(labels: &[ExportLabel]) -> String {
    if labels.is_empty() {
        return "none".to_string();
    }
    labels.iter().map(|label| label.as_str()).collect::<Vec<_>>().join(", ")
}

/// The named stale reasons in contract order, as their stable names.
fn stale_reasons_text(reasons: &[ExportStaleReason]) -> String {
    if reasons.is_empty() {
        return "none".to_string();
    }
    reasons.iter().map(|reason| reason.as_str()).collect::<Vec<_>>().join(", ")
}

/// The scenario's named stale reasons with their stored messages.
fn scenario_stale_reasons_text(reasons: &[CommercialStaleReason]) -> String {
    reasons.iter().map(|reason| format!("{}: {}", reason.code, reason.message)).collect::<Vec<_>>().join("\n")
}

/// The finding check as a readable stable name.
fn finding_check_text(check: &FindingCheck) -> String {
    match check {
        FindingCheck::WholeTube => "whole_tube".to_string(),
        FindingCheck::BendRadius { bend_index } => format!("bend_radius {bend_index}"),
        FindingCheck::BendSpacing { first_bend_index } => format!("bend_spacing {first_bend_index}"),
        FindingCheck::ConnectorCompatibility { end } => format!("connector_compatibility {}", tube_end_text(*end)),
        FindingCheck::EndAssemblyLength { end } => format!("end_assembly_length {}", tube_end_text(*end)),
    }
}

/// The required limit of a finding as one readable cell.
fn rule_limit_text(limit: &RuleLimit) -> String {
    match limit {
        RuleLimit::MinimumRadiusMm { radius_mm } => format!("minimum radius {radius_mm} mm"),
        RuleLimit::MinimumRadiusMultipleOfOuterDiameter { multiple } => format!("minimum radius {multiple} x outer diameter"),
        RuleLimit::MinimumLengthMm { length_mm } => format!("minimum length {length_mm} mm"),
        RuleLimit::CompatibleTubeRange { outer_diameter_mm, inner_diameter_mm } => format!(
            "outer diameter {}..{} mm; inner diameter {}..{} mm",
            outer_diameter_mm.min_mm, outer_diameter_mm.max_mm, inner_diameter_mm.min_mm, inner_diameter_mm.max_mm
        ),
    }
}

/// The measured values of one finding, one per line.
fn measurements_text(measurements: &[SupportingMeasurement]) -> String {
    measurements.iter().map(|measurement| format!("{} = {} mm", measurement.field_path, measurement.value_mm)).collect::<Vec<_>>().join("\n")
}

/// The applied rules of one finding with their source and limit, one per line.
fn applied_rules_text(rules: &[AppliedRule]) -> String {
    rules
        .iter()
        .map(|rule| {
            format!(
                "rule {} ({}) table version {} v{}: {}",
                rule.rule_id,
                rule.source_reference,
                rule.rule_table_version_id,
                rule.rule_table_version,
                rule_limit_text(&rule.limit)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A wrapped cell, or a blank one when the text is empty.
fn push_wrapped(row: &mut Row, text: String) {
    if text.is_empty() {
        row.skip();
    } else {
        row.wrap(text);
    }
}

/// The contract spelling of a physical unit.
fn unit_text(unit: Unit) -> &'static str {
    match unit {
        Unit::MillimetreCubed => "mm3",
        Unit::CentimetreCubed => "cm3",
        Unit::GramPerCentimetreCubed => "g/cm3",
        Unit::CurrencyPerKilogram => "currency/kg",
        Unit::Second => "s",
        Unit::Hour => "h",
        Unit::CurrencyPerHour => "currency/h",
        Unit::Currency => "currency",
        Unit::CurrencyPerPiece => "currency/piece",
    }
}

/// The selected purchase price source as one readable cell.
fn purchase_source_text(source: &BaselinePurchasePriceSource) -> String {
    match source {
        BaselinePurchasePriceSource::CatalogComponent { component_version_id } => {
            format!("catalog component {component_version_id}")
        }
        BaselinePurchasePriceSource::Tier { minimum_annual_quantity } => {
            format!("tier from {minimum_annual_quantity} per year")
        }
    }
}

/// The owner kind and identity of one tool entry.
fn tooling_owner(owner: &ToolingOwner) -> (&'static str, Uuid) {
    match owner {
        ToolingOwner::PartVersion { part_version_id } => ("part version", *part_version_id),
        ToolingOwner::Product { product_id } => ("product", *product_id),
    }
}

// ---------------------------------------------------------------------------
// Sheet construction
// ---------------------------------------------------------------------------

fn build_sheets(model: &ExportModel) -> Vec<Sheet> {
    vec![
        summary_sheet(model),
        products_sheet(model),
        bom_sheet(model),
        routings_sheet(model),
        feasibility_sheet(model),
        costs_sheet(model),
        tooling_sheet(model),
        scenarios_sheet(model),
        issues_sheet(model),
    ]
}

/// The metadata summary as key/value rows.
fn summary_sheet(model: &ExportModel) -> Sheet {
    let mut sheet = Sheet::new("Summary", vec![26, 110], 0, false, false);
    let mut title = Row::new().with_height(22);
    title.push(STYLE_SECTION, CellValue::Text("Internal export".to_string()));
    sheet.push(title);
    let metadata = &model.metadata;
    let scope = &model.scope;
    sheet.property_text("Project", metadata.project_name.clone());
    sheet.property_text("Project ID", metadata.project_id.to_string());
    let customer = if scope.customer.trim().is_empty() { metadata.customer.clone() } else { scope.customer.clone() };
    sheet.property_text("Customer", customer);
    sheet.property_text("Internal team", scope.internal_team.clone().unwrap_or_else(|| "none".to_string()));
    sheet.property_text("Platform", scope.platform.clone().unwrap_or_else(|| "none".to_string()));
    sheet.property_text("Application", scope.application.clone().unwrap_or_else(|| "none".to_string()));
    sheet.property_text("Currency", scope.currency.clone());
    sheet.property_text("Default schedule", format!("SOP {} to EOP {}", scope.default_schedule.sop_year, scope.default_schedule.eop_year));
    sheet.property_text("Exported at", metadata.exported_at.clone());
    sheet.property_text("Format", metadata.format.as_str());
    sheet.property_text("Selected result", selected_result_text(&metadata.baseline));
    sheet.property_integer("Project version", metadata.versions.project_version);
    sheet.property_integer("Engineering version", metadata.versions.engineering_version);
    sheet.property_integer("Manufacturing version", metadata.versions.manufacturing_version);
    sheet.property_integer("Feasibility version", metadata.versions.feasibility_version);
    sheet.property_text(
        "Feasibility record",
        if metadata.versions.feasibility_pinned { "pinned to the selected result" } else { "read at export time" },
    );
    sheet.property_text("Calculation ID", metadata.calculation.calculation_id.to_string());
    sheet.property_text("Calculator version", metadata.calculation.calculator_version.clone());
    sheet.property_text("Input hash", metadata.calculation.input_hash.clone());
    sheet.property_text("Result state", result_state_text(metadata.calculation.state));
    sheet.property_text("Labels", labels_text(&metadata.labels));
    sheet.property_text("Stale", yes_no(metadata.stale));
    sheet.property_text("Stale reasons", stale_reasons_text(&metadata.stale_reasons));
    sheet.property_integer("Issue count", u64::try_from(model.issues.len()).unwrap_or(u64::MAX));
    sheet.finish()
}

/// One product row of the Products sheet; the cost values come from the costed product line.
#[allow(clippy::too_many_arguments)]
fn push_product_row(
    sheet: &mut Sheet,
    product_id: Uuid,
    name: &str,
    kind: ProductKind,
    customer_part_number: Option<&str>,
    quotation_state: QuotationState,
    product_revision_id: Option<Uuid>,
    revision_state: Option<ProductRevisionState>,
    schedule: YearRange,
    forecasts: &[AnnualForecast],
    cost: Option<&ExportProduct>,
    years: &[u16],
) {
    let mut row = Row::new();
    row.text(product_id.to_string());
    row.wrap(name);
    row.text(product_kind_text(kind));
    row.wrap_opt(customer_part_number.filter(|text| !text.trim().is_empty()));
    row.text(quotation_state_text(quotation_state));
    row.text_opt(product_revision_id.map(|id| id.to_string()).as_deref());
    row.text_opt(revision_state.map(revision_state_text));
    row.integer(u64::from(schedule.sop_year));
    row.integer(u64::from(schedule.eop_year));
    row.quantity(cost.and_then(|product| product.lifetime_demand));
    row.money(cost.and_then(|product| product.material_cost));
    row.money(cost.and_then(|product| product.production_cost));
    row.money(cost.and_then(|product| product.attached_cost));
    row.money(cost.and_then(|product| product.unit_total));
    row.money(cost.and_then(|product| product.recurring_unit_cost));
    for year in years {
        let quantity = forecast_quantity(forecasts, *year).or_else(|| cost.and_then(|product| forecast_quantity(&product.forecasts, *year)));
        row.quantity(quantity);
    }
    sheet.push(row);
}

/// The costed product rows with their dynamic annual forecast columns.
fn products_sheet(model: &ExportModel) -> Sheet {
    let cost_lines: BTreeMap<Uuid, &ExportProduct> = model.products.iter().map(|product| (product.product_id, product)).collect();
    let mut year_set = BTreeSet::new();
    for product in &model.scope.products {
        for forecast in &product.forecasts {
            year_set.insert(forecast.year);
        }
    }
    for product in &model.products {
        for forecast in &product.forecasts {
            year_set.insert(forecast.year);
        }
    }
    let years: Vec<u16> = year_set.into_iter().collect();
    let mut widths = vec![38, 30, 10, 22, 14, 38, 14, 8, 8, 14, 16, 16, 16, 16, 16];
    widths.extend(years.iter().map(|_| 12));
    let mut sheet = Sheet::new("Products", widths, 1, true, true);
    let headers = [
        "Product ID",
        "Product Name",
        "Kind",
        "Customer Part Number",
        "Quotation State",
        "Costed Revision",
        "Revision State",
        "SOP",
        "EOP",
        "Lifetime Demand",
        "Material Cost",
        "Production Cost",
        "Attached Cost",
        "Unit Total",
        "Recurring Unit Cost",
    ];
    let mut header = Row::new().with_height(30);
    for label in headers {
        header.header(label);
    }
    for year in &years {
        header.header(&format!("Forecast {year}"));
    }
    sheet.push(header);

    let scope_ids: BTreeSet<Uuid> = model.scope.products.iter().map(|product| product.product_id).collect();
    for product in &model.scope.products {
        push_product_row(
            &mut sheet,
            product.product_id,
            &product.display_name(),
            product.kind,
            product.customer_part_number.as_deref(),
            product.quotation_state,
            product.product_revision_id,
            product.revision_state,
            product.schedule,
            &product.forecasts,
            cost_lines.get(&product.product_id).copied(),
            &years,
        );
    }
    // A product line the scope did not list (a stale scope row) still reaches the reader.
    for product in &model.products {
        if scope_ids.contains(&product.product_id) {
            continue;
        }
        push_product_row(
            &mut sheet,
            product.product_id,
            &product.display_name(),
            product.kind,
            product.customer_part_number.as_deref(),
            product.quotation_state,
            product.product_revision_id,
            product.revision_state,
            product.schedule,
            &product.forecasts,
            Some(product),
            &years,
        );
    }
    sheet.finish()
}

/// The annual forecast quantity of one product, when the model stores one.
fn forecast_quantity(forecasts: &[AnnualForecast], year: u16) -> Option<u64> {
    forecasts.iter().find(|forecast| forecast.year == year).map(|forecast| forecast.quantity)
}

/// The annual demand quantity of one part, when the model stores one.
fn demand_quantity(demand: &[BaselineAnnualDemand], year: u16) -> Option<u64> {
    demand.iter().find(|line| line.year == year).and_then(|line| line.quantity)
}

/// One BOM row per product × part version, with the part's dynamic annual demand columns.
fn bom_sheet(model: &ExportModel) -> Sheet {
    let parts: BTreeMap<Uuid, &ExportPart> = model.parts.iter().map(|part| (part.part_version_id, part)).collect();
    let mut year_set = BTreeSet::new();
    for part in &model.parts {
        for demand in &part.annual_demand {
            year_set.insert(demand.year);
        }
    }
    let years: Vec<u16> = year_set.into_iter().collect();
    let mut widths = vec![38, 30, 38, 22, 12, 24, 12, 14, 10, 14];
    widths.extend(years.iter().map(|_| 12));
    widths.push(16);
    let mut sheet = Sheet::new("BOM", widths, 1, true, true);
    let headers = [
        "Product ID",
        "Product Name",
        "Part Version ID",
        "Customer Part Number",
        "Customer Revision",
        "Classification",
        "Classification Confirmed",
        "Sourcing",
        "Quantity in Product",
        "Lifetime Demand",
    ];
    let mut header = Row::new().with_height(30);
    for label in headers {
        header.header(label);
    }
    for year in &years {
        header.header(&format!("Demand {year}"));
    }
    header.header("Unit Cost");
    sheet.push(header);

    for product in &model.products {
        for line in &product.parts {
            let part = parts.get(&line.part_version_id).copied();
            let mut row = Row::new();
            row.text(product.product_id.to_string());
            row.wrap(product.display_name());
            row.text(line.part_version_id.to_string());
            let customer_part_number = part.map(ExportPart::display_name);
            row.wrap_opt(customer_part_number.as_deref());
            let revision = part.map(|part| part.customer_revision.as_str()).filter(|text| !text.trim().is_empty());
            row.text_opt(revision);
            let classification = part.and_then(|part| part.classification.as_deref()).filter(|text| !text.trim().is_empty());
            row.wrap_opt(classification);
            row.text(yes_no(part.is_some_and(|part| part.classification_confirmed)));
            row.text_opt(part.and_then(|part| part.sourcing).map(sourcing_text));
            row.quantity(Some(line.quantity));
            row.quantity(part.and_then(|part| part.lifetime_demand));
            for year in &years {
                row.quantity(part.and_then(|part| demand_quantity(&part.annual_demand, *year)));
            }
            row.money(part.and_then(|part| part.unit_cost));
            sheet.push(row);
        }
    }
    sheet.finish()
}

/// One routing row for a part-version or product-root operation.
fn push_routing_row(sheet: &mut Sheet, owner_kind: &str, owner_id: Uuid, owner_name: &str, operation: &ExportOperation) {
    let mut row = Row::new();
    row.text(owner_kind);
    row.text(owner_id.to_string());
    row.wrap(owner_name.to_string());
    row.integer(u64::from(operation.sequence));
    row.text(operation.operation_id.to_string());
    match &operation.cycle {
        Some(cycle) => {
            row.decimal(cycle.seconds);
            row.text(cycle_basis_text(cycle.basis));
            row.integer_opt(cycle.cavities.map(u64::from));
            row.money(cycle.rate_per_hour);
            row.text(cycle.rate_currency.clone());
            row.text_opt(cycle.production_rate_version_id.map(|id| id.to_string()).as_deref());
        }
        None => {
            for _ in 0..6 {
                row.skip();
            }
        }
    }
    match &operation.setup {
        Some(setup) => {
            row.decimal(setup.hours);
            row.money(setup.rate_per_hour);
            row.text(setup.rate_currency.clone());
            row.text_opt(setup.setup_rate_version_id.map(|id| id.to_string()).as_deref());
        }
        None => {
            for _ in 0..4 {
                row.skip();
            }
        }
    }
    sheet.push(row);
}

/// The part and product-root routing operations with their pinned rates.
fn routings_sheet(model: &ExportModel) -> Sheet {
    let widths = vec![14, 38, 30, 10, 38, 12, 12, 10, 16, 10, 38, 12, 16, 10, 38];
    let mut sheet = Sheet::new("Routings", widths, 1, true, true);
    let headers = [
        "Owner Kind",
        "Owner ID",
        "Owner Name",
        "Sequence",
        "Operation ID",
        "Cycle Seconds",
        "Cycle Basis",
        "Cavities",
        "Production Rate per Hour",
        "Rate Currency",
        "Rate Version ID",
        "Setup Hours",
        "Setup Rate per Hour",
        "Setup Rate Currency",
        "Setup Rate Version ID",
    ];
    let mut header = Row::new().with_height(30);
    for label in headers {
        header.header(label);
    }
    sheet.push(header);
    for part in &model.parts {
        for operation in &part.operations {
            push_routing_row(&mut sheet, "part version", part.part_version_id, &part.display_name(), operation);
        }
    }
    for product in &model.products {
        for operation in &product.root_operations {
            push_routing_row(&mut sheet, "product", product.product_id, &product.display_name(), operation);
        }
    }
    sheet.finish()
}

/// One feasibility row: a finding of the latest assessment, with its exception when one covers it.
fn push_feasibility_row(sheet: &mut Sheet, finding: &FeasibilityFinding, exception: Option<&FeasibilityExceptionView>) {
    let mut row = Row::new();
    row.text(finding.key.component_id.to_string());
    row.text(finding.key.product_revision_id.to_string());
    row.text(finding.key.kind.as_str());
    row.text(finding_check_text(&finding.key.check));
    let state = match (finding.state, finding.unresolved_reason) {
        (FindingState::Unresolved, Some(reason)) => format!("unresolved ({})", unresolved_reason_text(reason)),
        (state, _) => finding_state_text(state).to_string(),
    };
    row.text(state);
    let limit = finding.required_limit.as_ref().map(rule_limit_text);
    row.wrap_opt(limit.as_deref());
    push_wrapped(&mut row, measurements_text(&finding.measurements));
    push_wrapped(&mut row, applied_rules_text(&finding.applied_rules));
    match exception {
        Some(view) => {
            row.wrap(view.exception.justification.clone());
            row.text(view.exception.accepted_by.clone());
            row.text(view.exception.accepted_at.clone());
            row.text(exception_state_text(view.state));
        }
        None => {
            for _ in 0..4 {
                row.skip();
            }
        }
    }
    sheet.push(row);
}

/// The feasibility findings, their limits, measurements, rules and recorded exceptions.
fn feasibility_sheet(model: &ExportModel) -> Sheet {
    let widths = vec![38, 38, 26, 22, 12, 28, 44, 54, 40, 24, 24, 16];
    let mut sheet = Sheet::new("Feasibility", widths, 1, true, true);
    let headers = [
        "Component ID",
        "Product Revision ID",
        "Rule Kind",
        "Check",
        "State",
        "Required Limit",
        "Measurements",
        "Applied Rules",
        "Exception Justification",
        "Accepted By",
        "Accepted At",
        "Exception State",
    ];
    let mut header = Row::new().with_height(30);
    for label in headers {
        header.header(label);
    }
    sheet.push(header);
    if let Some(feasibility) = &model.feasibility {
        let view = &feasibility.view;
        let mut referenced: BTreeSet<Uuid> = BTreeSet::new();
        if let Some(assessment) = &view.latest_assessment {
            for finding_view in &assessment.findings {
                let exception_id = match &finding_view.disposition {
                    FindingDisposition::Open => None,
                    FindingDisposition::ExceptionAccepted { exception_id } => Some(*exception_id),
                    FindingDisposition::ExceptionReviewRequired { exception_id } => Some(*exception_id),
                };
                let exception = exception_id.and_then(|id| view.exceptions.iter().find(|view| view.exception.exception_id == id));
                if let Some(view) = exception {
                    referenced.insert(view.exception.exception_id);
                }
                push_feasibility_row(&mut sheet, &finding_view.finding, exception);
            }
        }
        // A recorded exception whose finding is no longer in the latest assessment (a closed
        // decision, or an assessment changed the finding key) stays visible with its own row.
        for exception in &view.exceptions {
            if referenced.contains(&exception.exception.exception_id) {
                continue;
            }
            push_feasibility_row(&mut sheet, &exception.exception.finding, Some(exception));
        }
    }
    sheet.finish()
}

/// The detailed cost blocks: part lines, their source records, setup lines, categories, annual cost
/// and the project totals.
fn costs_sheet(model: &ExportModel) -> Sheet {
    let widths = vec![24, 38, 14, 14, 14, 14, 16, 16, 16, 16, 16, 18, 38];
    let mut sheet = Sheet::new("Costs", widths, 0, false, false);
    sheet.push(section_row("Part cost lines"));
    let part_headers = [
        "Customer Part Number",
        "Part Version ID",
        "Sourcing",
        "Lifetime Demand",
        "Peak Demand",
        "Costing Volume",
        "Material Cost",
        "Production Cost",
        "Attached Cost",
        "Unit Cost",
        "Setup Charge",
        "Reviewed Volume (mm3)",
        "Created In Package",
    ];
    sheet.push(header_row(&part_headers));
    for part in &model.parts {
        let mut row = Row::new();
        row.wrap(part.display_name());
        row.text(part.part_version_id.to_string());
        row.text_opt(part.sourcing.map(sourcing_text));
        row.quantity(part.lifetime_demand);
        row.quantity(part.peak_demand);
        row.decimal(part.costing_volume);
        row.money(part.material_cost);
        row.money(part.production_cost);
        row.money(part.attached_cost);
        row.money(part.unit_cost);
        row.money(part.setup_charge);
        row.decimal(part.reviewed_volume_mm3);
        row.text(part.created_in_package_id.to_string());
        sheet.push(row);
    }

    sheet.push(Row::blank());
    sheet.push(section_row("Product cost lines"));
    sheet.push(header_row(&[
        "Product ID",
        "Product Name",
        "Quotation State",
        "Counted in Result",
        "Material Cost",
        "Production Cost",
        "Attached Cost",
        "Root Production Cost",
        "Root Attached Cost",
        "Unit Total",
        "Recurring Unit Cost",
    ]));
    for product in &model.products {
        let mut row = Row::new();
        row.text(product.product_id.to_string());
        row.wrap(product.display_name());
        row.text(quotation_state_text(product.quotation_state));
        row.text(yes_no(product.quoted));
        row.money(product.material_cost);
        row.money(product.production_cost);
        row.money(product.attached_cost);
        row.money(product.root_production_cost);
        row.money(product.root_attached_cost);
        row.money(product.unit_total);
        row.money(product.recurring_unit_cost);
        sheet.push(row);
    }

    sheet.push(Row::blank());
    sheet.push(section_row("Product cost categories"));
    sheet.push(header_row(&["Product ID", "Category Code", "Category Name", "Amount"]));
    for product in &model.products {
        for category in &product.categories {
            let mut row = Row::new();
            row.text(product.product_id.to_string());
            row.text(category.code.clone());
            row.wrap(category.name.clone());
            row.money(category.amount);
            sheet.push(row);
        }
    }

    sheet.push(Row::blank());
    sheet.push(section_row("Product root attached cost lines"));
    sheet.push(header_row(&["Product ID", "Category", "Quantity", "Unit Cost"]));
    for product in &model.products {
        for line in &product.root_attached_costs {
            let mut row = Row::new();
            row.text(product.product_id.to_string());
            row.wrap(line.category.clone());
            row.decimal(line.quantity);
            row.money(line.unit_cost);
            sheet.push(row);
        }
    }

    sheet.push(Row::blank());
    sheet.push(section_row("Part material records"));
    sheet.push(header_row(&[
        "Part Version ID",
        "Net Volume",
        "Unit",
        "Net Volume (mm3)",
        "Net Mass (kg)",
        "Net Material Cost",
        "Allowance Rate",
        "Price per kg",
        "Price Currency",
        "FX Rate",
        "Allowed Material Cost",
    ]));
    for part in &model.parts {
        let Some(material) = &part.material else { continue };
        let mut row = Row::new();
        row.text(part.part_version_id.to_string());
        row.decimal(material.net_volume);
        row.text(unit_text(material.net_volume_unit));
        row.decimal(material.net_volume_mm3);
        row.decimal(material.net_mass_kg);
        row.money(material.net_material_cost);
        row.percent(material.allowance_rate);
        row.money(material.price_per_kg);
        row.text(material.price_currency.clone());
        row.decimal(material.fx_rate);
        row.money(material.allowed_material_cost);
        sheet.push(row);
    }

    sheet.push(Row::blank());
    sheet.push(section_row("Part purchase prices"));
    sheet.push(header_row(&["Part Version ID", "Source", "Currency", "Source Unit Price", "Project Unit Price", "FX Rate"]));
    for part in &model.parts {
        let Some(purchase) = &part.purchase else { continue };
        let mut row = Row::new();
        row.text(part.part_version_id.to_string());
        row.wrap(purchase_source_text(&purchase.source));
        row.text(purchase.currency.clone());
        row.money(Some(purchase.source_unit_price));
        row.money(Some(purchase.project_unit_price));
        row.decimal(purchase.fx_rate);
        sheet.push(row);
    }

    sheet.push(Row::blank());
    sheet.push(section_row("Part attached cost lines"));
    sheet.push(header_row(&["Part Version ID", "Category", "Quantity", "Unit Cost"]));
    for part in &model.parts {
        for line in &part.attached_costs {
            let mut row = Row::new();
            row.text(part.part_version_id.to_string());
            row.wrap(line.category.clone());
            row.decimal(line.quantity);
            row.money(line.unit_cost);
            sheet.push(row);
        }
    }

    sheet.push(Row::blank());
    sheet.push(section_row("Separate setup lines"));
    sheet.push(header_row(&["Owner Kind", "Owner ID", "Setup Charge"]));
    for line in &model.setup {
        push_setup_line(&mut sheet, line);
    }

    sheet.push(Row::blank());
    sheet.push(section_row("Setup categories"));
    sheet.push(header_row(&["Category Code", "Category Name", "Amount"]));
    for category in &model.setup_categories {
        push_category_line(&mut sheet, category);
    }

    sheet.push(Row::blank());
    sheet.push(section_row("Annual project manufacturing cost"));
    sheet.push(header_row(&["Year", "Manufacturing Cost"]));
    for annual in &model.annual {
        let mut row = Row::new();
        row.integer(u64::from(annual.year));
        row.money(annual.manufacturing_cost);
        sheet.push(row);
    }

    sheet.push(Row::blank());
    sheet.push(section_row("Project totals"));
    push_total_row(&mut sheet, "Lifetime manufacturing cost including separate setup", model.project_total_manufacturing_cost);
    push_total_row(&mut sheet, "Separate setup total", model.setup_total);
    sheet.finish()
}

/// A section title row of a block sheet.
fn section_row(title: &str) -> Row {
    let mut row = Row::new().with_height(20);
    row.push(STYLE_SECTION, CellValue::Text(title.to_string()));
    row
}

/// A header row of a block sheet's table.
fn header_row(labels: &[&str]) -> Row {
    let mut row = Row::new().with_height(30);
    for label in labels {
        row.header(label);
    }
    row
}

/// One separate setup line row.
fn push_setup_line(sheet: &mut Sheet, line: &BaselineSetupLine) {
    let (kind, owner_id) = match &line.owner {
        BaselineSetupOwner::PartVersion { part_version_id } => ("part version", *part_version_id),
        BaselineSetupOwner::Product { product_id } => ("product", *product_id),
    };
    let mut row = Row::new();
    row.text(kind);
    row.text(owner_id.to_string());
    row.money(line.setup_charge);
    sheet.push(row);
}

/// One setup-scope category row.
fn push_category_line(sheet: &mut Sheet, category: &BaselineCategoryLine) {
    let mut row = Row::new();
    row.text(category.code.clone());
    row.wrap(category.name.clone());
    row.money(category.amount);
    sheet.push(row);
}

/// One project-total row: the label in column A, the amount in column D so the label may spill.
fn push_total_row(sheet: &mut Sheet, label: &str, amount: Option<Decimal>) {
    let mut row = Row::new();
    row.bold(label.to_string());
    row.skip();
    row.skip();
    row.money(amount);
    sheet.push(row);
}

/// The tooling project totals, one row per tool and the per-product allocation rows.
fn tooling_sheet(model: &ExportModel) -> Sheet {
    let mut year_set = BTreeSet::new();
    if let Some(tooling) = &model.tooling {
        for tool in &tooling.tools {
            for recovery in &tool.annual_recovery {
                year_set.insert(recovery.year);
            }
        }
    }
    let years: Vec<u16> = year_set.into_iter().collect();
    let mut widths = vec![38, 34, 16, 16, 16, 14, 16, 16, 18, 18, 18, 18];
    widths.extend(years.iter().map(|_| 14));
    let mut sheet = Sheet::new("Tooling", widths, 0, false, false);

    sheet.push(section_row("Tooling project totals"));
    match &model.tooling {
        Some(tooling) => {
            sheet.property_text("Treatment", tooling_treatment_text(tooling.treatment).unwrap_or("not selected"));
            sheet.property_text("State", result_state_text(tooling.state));
            push_tooling_total(&mut sheet, "Project investment", tooling.project_investment);
            push_tooling_total(&mut sheet, "Project amortized recovered", tooling.project_amortized_recovered);
            push_tooling_total(&mut sheet, "Project under/over recovery", tooling.project_under_over_recovery);
            push_tooling_total(&mut sheet, "Project separate charge", tooling.project_separate_charge);
        }
        None => {
            sheet.property_text("Treatment", "not selected");
            sheet.property_text("State", "no tooling answer is recorded for this selected result");
            push_tooling_total(&mut sheet, "Project investment", None);
            push_tooling_total(&mut sheet, "Project amortized recovered", None);
            push_tooling_total(&mut sheet, "Project under/over recovery", None);
            push_tooling_total(&mut sheet, "Project separate charge", None);
        }
    }

    sheet.push(Row::blank());
    sheet.push(section_row("Tools"));
    let tool_headers = [
        "Tool ID",
        "Owner ID",
        "Owner Kind",
        "Tool Count",
        "Cost per Tool",
        "Currency",
        "Investment",
        "Recovery Quantity",
        "Contribution per Owner Unit",
        "Projected Recovery",
        "Under/Over Recovery",
        "Separate Charge",
    ];
    let mut tool_header = header_row(&tool_headers);
    for year in &years {
        tool_header.header(&format!("Recovery {year}"));
    }
    sheet.push(tool_header);
    if let Some(tooling) = &model.tooling {
        for tool in &tooling.tools {
            let (kind, owner_id) = tooling_owner(&tool.owner);
            let mut row = Row::new();
            row.text(tool.tool_id.to_string());
            row.text(owner_id.to_string());
            row.text(kind);
            row.quantity(Some(tool.tool_count));
            row.money(Some(tool.cost_per_tool));
            row.text(tool.currency.clone());
            row.money(tool.investment);
            row.decimal(tool.recovery_quantity);
            row.money(tool.contribution_per_owner_unit);
            row.money(tool.projected_recovery);
            row.money(tool.under_over_recovery);
            row.money(tool.separate_charge);
            for year in &years {
                let recovered = tool.annual_recovery.iter().find(|recovery| recovery.year == *year).and_then(|recovery| recovery.recovered);
                row.money(recovered);
            }
            sheet.push(row);
        }
    }

    sheet.push(Row::blank());
    sheet.push(section_row("Allocations"));
    sheet.push(header_row(&["Tool ID", "Product ID", "Allocated Investment", "Product Contribution", "Allocation Share"]));
    if let Some(tooling) = &model.tooling {
        for tool in &tooling.tools {
            for allocation in &tool.allocation {
                let mut row = Row::new();
                row.text(tool.tool_id.to_string());
                row.text(allocation.product_id.to_string());
                row.money(allocation.allocated_investment);
                row.money(allocation.product_contribution);
                row.percent(allocation.allocation_share);
                sheet.push(row);
            }
        }
    }
    sheet.finish()
}

/// One amount row of the tooling totals block: the label in column A, the amount in column C.
fn push_tooling_total(sheet: &mut Sheet, label: &str, amount: Option<Decimal>) {
    let mut row = Row::new();
    row.bold(label.to_string());
    row.skip();
    row.money(amount);
    sheet.push(row);
}

/// The scenario summary and the per-product, per-year commercial detail.
fn scenarios_sheet(model: &ExportModel) -> Sheet {
    let widths = vec![38, 12, 38, 30, 38, 16, 14, 16, 16, 16, 16, 16, 16, 16, 14, 16, 14, 16, 16, 16];
    let mut sheet = Sheet::new("Scenarios", widths, 0, false, false);
    let scope_products: BTreeMap<Uuid, &super::model::ExportScopedProduct> =
        model.scope.products.iter().map(|product| (product.product_id, product)).collect();

    sheet.push(section_row("Scenario summary"));
    let summary_headers = [
        "Scenario ID",
        "Version",
        "Name",
        "Stale",
        "Stale Reasons",
        "Lifetime Revenue",
        "Lifetime EBIT",
        "Tooling Investment",
        "Tooling Separate Charge",
        "Tooling Projected Recovery",
        "Tooling Under/Over Recovery",
        "Setup Total",
        "State",
    ];
    sheet.push(header_row(&summary_headers));
    for scenario in &model.scenarios {
        let outcome = &scenario.outcome;
        let mut row = Row::new();
        row.text(scenario.scenario_id.to_string());
        row.integer(scenario.version);
        row.wrap(scenario.name.clone());
        row.text(yes_no(scenario.stale));
        row.wrap(scenario_stale_reasons_text(&scenario.stale_reasons));
        row.money(outcome.project.lifetime_revenue);
        row.money(outcome.project.lifetime_ebit_amount);
        row.money(outcome.project.tooling_investment);
        row.money(outcome.project.tooling_separate_charge);
        row.money(outcome.project.tooling_projected_recovery);
        row.money(outcome.project.tooling_under_over_recovery);
        row.money(outcome.setup_total);
        row.text(result_state_text(outcome.state));
        sheet.push(row);
    }

    sheet.push(Row::blank());
    sheet.push(section_row("Scenario details"));
    let detail_headers = [
        "Scenario Name",
        "Scenario Version",
        "Product ID",
        "Product Name",
        "Product Revision ID",
        "Year",
        "Forecast",
        "Recurring Unit Cost",
        "Suggested Base Price",
        "Manual Base Price",
        "Tooling Contribution",
        "Suggested Total Price",
        "Manual Total Price",
        "Suggested EBIT Amount",
        "Suggested EBIT Rate",
        "Manual EBIT Amount",
        "Manual EBIT Rate",
        "Revenue",
        "EBIT Amount",
        "Tooling Recovery Revenue",
    ];
    sheet.push(header_row(&detail_headers));
    for scenario in &model.scenarios {
        for product in &scenario.outcome.products {
            let product_name = scope_products.get(&product.product_id).map(|product| product.display_name()).unwrap_or_default();
            for line in &product.years {
                let mut row = Row::new();
                row.wrap(scenario.name.clone());
                row.integer(scenario.version);
                row.text(product.product_id.to_string());
                row.wrap(product_name.clone());
                row.text_opt(product.product_revision_id.map(|id| id.to_string()).as_deref());
                row.integer(u64::from(line.year));
                row.quantity(line.forecast);
                row.money(line.recurring_unit_cost);
                row.money(line.suggested_base_price);
                row.money(line.manual_base_price);
                row.money(line.tooling_contribution);
                row.money(line.suggested_total_price);
                row.money(line.manual_total_price);
                row.money(line.suggested_unit_ebit_amount);
                row.percent(line.suggested_unit_ebit_rate);
                row.money(line.manual_unit_ebit_amount);
                row.percent(line.manual_unit_ebit_rate);
                row.money(line.revenue);
                row.money(line.ebit_amount);
                row.money(line.tooling_recovery_revenue);
                sheet.push(row);
            }
        }
    }
    sheet.finish()
}

/// Every listed issue with its source, stable code, scope and stored message.
fn issues_sheet(model: &ExportModel) -> Sheet {
    let widths = vec![14, 34, 38, 100];
    let mut sheet = Sheet::new("Issues", widths, 1, true, true);
    let headers = ["Source", "Code", "Scope ID", "Message"];
    let mut header = Row::new().with_height(30);
    for label in headers {
        header.header(label);
    }
    sheet.push(header);
    for issue in &model.issues {
        let mut row = Row::new();
        row.text(issue.source.as_str());
        row.wrap(issue.code.clone());
        row.text_opt(issue.scope.map(|id| id.to_string()).as_deref());
        row.wrap(issue.message.clone());
        sheet.push(row);
    }
    sheet.finish()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use solidbom_contract::*;

    use super::*;
    use crate::export::model::{ExportFeasibility, ExportIssue, ExportIssueSource, ExportScenario, ExportScope, ExportScopedProduct};

    const YEARS: [u16; 3] = [2027, 2028, 2029];

    fn id(value: u128) -> Uuid {
        Uuid::from_u128(value)
    }

    fn dec(text: &str) -> Decimal {
        text.parse().expect("a decimal literal in the test fixture")
    }

    fn schedule(years: &[u16]) -> YearRange {
        YearRange { sop_year: *years.first().unwrap_or(&2027), eop_year: *years.last().unwrap_or(&2029) }
    }

    fn forecasts(years: &[u16], base: u64) -> Vec<AnnualForecast> {
        years.iter().enumerate().map(|(index, year)| AnnualForecast { year: *year, quantity: base + index as u64 * 100 }).collect()
    }

    fn metadata(baseline: ExportBaselineSelection) -> ExportMetadata {
        ExportMetadata {
            project_id: id(1),
            project_name: "Three boxes & brackets".to_string(),
            customer: "Internal \"pilot\"".to_string(),
            currency: "CNY".to_string(),
            exported_at: "2026-11-11T13:00:00Z".to_string(),
            format: ExportFormat::Xlsx,
            baseline,
            versions: ExportRecordVersions {
                project_version: 4,
                engineering_version: 3,
                manufacturing_version: 2,
                feasibility_version: 1,
                feasibility_pinned: true,
            },
            calculation: ExportCalculation {
                calculation_id: id(3),
                calculator_version: "solidbom-manufacturing-baseline/1".to_string(),
                input_hash: "a".repeat(64),
                state: ResultState::Complete,
            },
            labels: vec![ExportLabel::OutdatedCosts, ExportLabel::SupersededAcceptedBaseline],
            stale: false,
            stale_reasons: Vec::new(),
        }
    }

    fn cycle(operation: u128) -> BaselineCycleInput {
        BaselineCycleInput {
            production_rate_version_id: Some(id(8000 + operation)),
            seconds: Some(dec("12.5")),
            basis: CycleBasis::PerPiece,
            cavities: None,
            rate_per_hour: Some(dec("45.00")),
            rate_currency: "CNY".to_string(),
        }
    }

    fn setup(operation: u128) -> BaselineSetupInput {
        BaselineSetupInput {
            setup_rate_version_id: Some(id(8100 + operation)),
            hours: Some(dec("1.5")),
            rate_per_hour: Some(dec("60.00")),
            rate_currency: "CNY".to_string(),
        }
    }

    fn operation(value: u128) -> ExportOperation {
        ExportOperation { sequence: 10, operation_id: id(5000 + value), cycle: Some(cycle(value)), setup: Some(setup(value)) }
    }

    fn attached_line(value: u128) -> BaselineAttachedCostInput {
        BaselineAttachedCostInput {
            line_id: id(6000 + value),
            category: "Packaging".to_string(),
            quantity: Some(dec("1")),
            unit_cost: Some(dec("2.50")),
        }
    }

    fn material() -> BaselineMaterialCost {
        BaselineMaterialCost {
            net_volume: Some(dec("1200")),
            net_volume_unit: Unit::MillimetreCubed,
            net_volume_mm3: Some(dec("1200")),
            net_mass_kg: Some(dec("3.24")),
            net_material_cost: Some(dec("18.50")),
            allowance_rate: Some(dec("0.05")),
            price_per_kg: Some(dec("5.71")),
            price_currency: "CNY".to_string(),
            fx_rate: None,
            allowed_material_cost: Some(dec("19.43")),
        }
    }

    fn purchase() -> BaselineSelectedPurchasePrice {
        BaselineSelectedPurchasePrice {
            source: BaselinePurchasePriceSource::CatalogComponent { component_version_id: id(7000) },
            currency: "CNY".to_string(),
            source_unit_price: dec("21.00"),
            project_unit_price: dec("21.00"),
            fx_rate: None,
        }
    }

    fn fixture_part(index: u128, years: &[u16]) -> ExportPart {
        let manufactured = index.is_multiple_of(2);
        ExportPart {
            part_version_id: id(index),
            customer_part_number: format!("PART-{index:04}"),
            customer_revision: format!("R{}", index % 3),
            created_in_package_id: id(9000),
            classification: Some(if index.is_multiple_of(3) { "Bracket & plate".to_string() } else { "Tube".to_string() }),
            classification_confirmed: index.is_multiple_of(2),
            reviewed_volume_mm3: Some(dec("1200.5")),
            sourcing: Some(if manufactured { Sourcing::Manufactured } else { Sourcing::Purchased }),
            material: manufactured.then(material),
            purchase: (!manufactured).then(purchase),
            attached_costs: vec![attached_line(index)],
            operations: if index.is_multiple_of(5) { vec![operation(index)] } else { Vec::new() },
            annual_demand: years
                .iter()
                .map(|year| BaselineAnnualDemand { year: *year, quantity: Some((*year as u64) * 10 + index as u64) })
                .collect(),
            lifetime_demand: Some(12_000 + index as u64),
            peak_demand: Some(5_000 + index as u64),
            costing_volume: Some(dec("12000.5")),
            material_cost: Some(dec("12.34")),
            production_cost: Some(dec("7.89")),
            attached_cost: Some(dec("2.50")),
            unit_cost: Some(dec("22.73")),
            setup_charge: Some(dec("50.00")),
        }
    }

    fn fixture_products(years: &[u16], parts: &[ExportPart]) -> Vec<ExportProduct> {
        let mut housing_parts = parts
            .iter()
            .filter(|part| part.part_version_id <= id(101))
            .map(|part| BaselineProductPartLine { part_version_id: part.part_version_id, quantity: 1 })
            .collect::<Vec<_>>();
        // One shared part is used twice, so the occurrence quantity is not always one.
        if let Some(line) = housing_parts.first_mut() {
            line.quantity = 2;
        }
        let bracket_parts = parts
            .iter()
            .filter(|part| part.part_version_id > id(101))
            .map(|part| BaselineProductPartLine { part_version_id: part.part_version_id, quantity: 1 })
            .collect();
        vec![
            ExportProduct {
                product_id: id(1000),
                name: "Housing & cover".to_string(),
                kind: ProductKind::Assembly,
                customer_part_number: Some("CPN-1000".to_string()),
                quotation_state: QuotationState::Included,
                product_revision_id: Some(id(2000)),
                revision_state: Some(ProductRevisionState::Active),
                schedule: schedule(years),
                forecasts: forecasts(years, 1_000),
                quoted: true,
                root_operations: vec![operation(1000)],
                root_attached_costs: vec![attached_line(1000)],
                parts: housing_parts,
                lifetime_demand: Some(123_456),
                material_cost: Some(dec("234.56")),
                production_cost: Some(dec("120.00")),
                attached_cost: Some(dec("12.50")),
                root_production_cost: Some(dec("10.00")),
                root_attached_cost: Some(dec("5.00")),
                categories: vec![
                    BaselineCategoryLine { code: "OH".to_string(), name: "Overhead".to_string(), amount: Some(dec("9.00")) },
                    BaselineCategoryLine { code: "SG".to_string(), name: "Selling".to_string(), amount: Some(dec("3.00")) },
                ],
                unit_total: Some(dec("123.45")),
                recurring_unit_cost: Some(dec("120.00")),
            },
            ExportProduct {
                product_id: id(1001),
                name: "Bracket".to_string(),
                kind: ProductKind::Part,
                customer_part_number: Some("CPN-1001".to_string()),
                quotation_state: QuotationState::Excluded,
                product_revision_id: None,
                revision_state: None,
                schedule: schedule(years),
                forecasts: Vec::new(),
                quoted: false,
                root_operations: Vec::new(),
                root_attached_costs: Vec::new(),
                parts: bracket_parts,
                lifetime_demand: None,
                material_cost: None,
                production_cost: None,
                attached_cost: None,
                root_production_cost: None,
                root_attached_cost: None,
                categories: Vec::new(),
                unit_total: None,
                recurring_unit_cost: None,
            },
        ]
    }

    fn fixture_scoped_product(years: &[u16]) -> ExportScopedProduct {
        ExportScopedProduct {
            product_id: id(1000),
            name: "Housing & cover".to_string(),
            kind: ProductKind::Assembly,
            customer_part_number: Some("CPN-1000".to_string()),
            quotation_state: QuotationState::Included,
            product_revision_id: Some(id(2000)),
            revision_state: Some(ProductRevisionState::Active),
            schedule: schedule(years),
            forecasts: forecasts(years, 1_000),
        }
    }

    fn tooling_outcome(years: &[u16]) -> ToolingOutcome {
        let allocation = |product: u128, share: &str, investment: &str| ToolingAllocationLine {
            product_id: id(product),
            product_lifetime_demand: Some(12_345),
            expanded_quantity: 1,
            product_owner_demand: Some(12_345),
            allocation_share: Some(dec(share)),
            allocated_investment: Some(dec(investment)),
            product_contribution: Some(dec("0.25")),
        };
        ToolingOutcome {
            state: ResultState::Complete,
            treatment: Some(ToolingTreatment::Amortized),
            tools: vec![ToolingToolLine {
                tool_id: id(400),
                owner: ToolingOwner::PartVersion { part_version_id: id(1) },
                tool_count: 2,
                cost_per_tool: dec("2500.00"),
                currency: "CNY".to_string(),
                source_investment: Some(dec("5000.00")),
                fx_rate: None,
                investment: Some(dec("5000.00")),
                recovery_quantity: Some(dec("20000")),
                contribution_per_owner_unit: Some(dec("0.25")),
                allocation: vec![allocation(1000, "0.75", "3750.00"), allocation(1001, "0.25", "1250.00")],
                annual_recovery: years.iter().map(|year| ToolingAnnualRecovery { year: *year, recovered: Some(dec("1000.00")) }).collect(),
                projected_recovery: Some(dec("5000.00")),
                under_over_recovery: Some(dec("0.00")),
                separate_charge: None,
                explanations: Vec::new(),
            }],
            products: vec![ToolingProductLine {
                product_id: id(1000),
                product_revision_id: Some(id(2000)),
                lifetime_demand: Some(123_456),
                allocated_investment: Some(dec("3750.00")),
                amortized_contribution: Some(dec("0.25")),
                explanations: Vec::new(),
            }],
            project_investment: Some(dec("5000.00")),
            project_amortized_recovered: Some(dec("5000.00")),
            project_under_over_recovery: Some(dec("0.00")),
            project_separate_charge: None,
            annual_recovery: years.iter().map(|year| ToolingAnnualRecovery { year: *year, recovered: Some(dec("1000.00")) }).collect(),
            explanations: Vec::new(),
            issues: Vec::new(),
        }
    }

    fn commercial_outcome(years: &[u16]) -> CommercialOutcome {
        CommercialOutcome {
            state: ResultState::Complete,
            ebit_target_rate: Some(dec("0.2")),
            annual_productivity_rate: Some(dec("0.03")),
            productivity_applies_to_tooling: false,
            products: vec![CommercialProductLine {
                product_id: id(1000),
                product_revision_id: Some(id(2000)),
                schedule: schedule(years),
                manual_price: Some(dec("150.00")),
                suggested_first_year_price: Some(dec("188.00")),
                manual_first_year_ebit_amount: Some(dec("30.00")),
                manual_first_year_ebit_rate: Some(dec("0.2")),
                tooling_initial_contribution: Some(dec("5.00")),
                allocated_investment: Some(dec("500.00")),
                years: years
                    .iter()
                    .map(|year| CommercialYearLine {
                        year: *year,
                        forecast: Some(1_000),
                        recurring_unit_cost: Some(dec("120.00")),
                        suggested_base_price: Some(dec("150.00")),
                        manual_base_price: Some(dec("150.00")),
                        tooling_contribution: Some(dec("5.00")),
                        suggested_total_price: Some(dec("155.00")),
                        manual_total_price: Some(dec("155.00")),
                        suggested_unit_ebit_amount: Some(dec("30.00")),
                        suggested_unit_ebit_rate: Some(dec("0.2")),
                        manual_unit_ebit_amount: Some(dec("30.00")),
                        manual_unit_ebit_rate: Some(dec("0.2")),
                        revenue: Some(dec("150000.00")),
                        ebit_amount: Some(dec("30000.00")),
                        tooling_recovery_revenue: Some(dec("5000.00")),
                    })
                    .collect(),
                explanations: Vec::new(),
            }],
            project: CommercialProjectTotals {
                lifetime_revenue: Some(dec("1000000.00")),
                lifetime_ebit_amount: Some(dec("200000.00")),
                years: years
                    .iter()
                    .map(|year| CommercialYearTotal {
                        year: *year,
                        revenue: Some(dec("150000.00")),
                        ebit_amount: Some(dec("30000.00")),
                        tooling_recovery_revenue: Some(dec("5000.00")),
                    })
                    .collect(),
                tooling_investment: Some(dec("5000.00")),
                tooling_separate_charge: None,
                tooling_projected_recovery: Some(dec("5000.00")),
                tooling_under_over_recovery: Some(dec("0.00")),
            },
            tooling: tooling_outcome(years),
            setup_total: Some(dec("126.00")),
            explanations: Vec::new(),
            issues: Vec::new(),
        }
    }

    fn assessed_component(component_id: Uuid, product_revision_id: Uuid) -> AssessedComponent {
        AssessedComponent {
            evidence: FeasibilityComponentEvidence {
                component_id,
                product_revision_id,
                analysis_key: AnalysisKey {
                    source_sha256: "b".repeat(64),
                    engine_build: "solidcore/1".to_string(),
                    extractor_version: "feasibility/1".to_string(),
                },
                extraction_version: "feasibility/1".to_string(),
                attributes: FeasibilityTubeAttributes::default(),
                measurements: None,
                ends: Vec::new(),
            },
            part_version_id: Some(id(1)),
            reviewed_geometry: vec![ReviewedGeometry { source_path: vec![1, 2], source_body: 3 }],
        }
    }

    fn feasibility_finding(component_id: Uuid, product_revision_id: Uuid, kind: FeasibilityRuleKind, check: FindingCheck) -> FeasibilityFinding {
        FeasibilityFinding {
            key: FindingKey { component_id, product_revision_id, kind, check },
            state: FindingState::Fail,
            unresolved_reason: None,
            measurements: vec![SupportingMeasurement { field_path: "bends.1.centerline_radius_mm".to_string(), value_mm: dec("2.5") }],
            applied_rules: vec![AppliedRule {
                rule_id: id(701),
                rule_table_id: id(702),
                rule_table_version_id: id(700),
                rule_table_version: 4,
                source_reference: "owner.smooth-tube.minimum-radius".to_string(),
                limit: RuleLimit::MinimumRadiusMm { radius_mm: dec("3.0") },
            }],
            required_limit: Some(RuleLimit::MinimumRadiusMm { radius_mm: dec("3.0") }),
        }
    }

    fn feasibility(_years: &[u16]) -> ExportFeasibility {
        let component_id = id(600);
        let product_revision_id = id(2000);
        let finding = feasibility_finding(component_id, product_revision_id, FeasibilityRuleKind::MinimumBendRadius, FindingCheck::WholeTube);
        let exception = FeasibilityException {
            exception_id: id(601),
            finding_key: finding.key.clone(),
            justification: "Radius is below the table limit, but the bend is formed in a controlled fixture.".to_string(),
            accepted_by: "engineer@example.test".to_string(),
            accepted_at: "2026-11-10T09:00:00Z".to_string(),
            assessment_version: 3,
            rule_table_version_id: id(700),
            component: assessed_component(component_id, product_revision_id),
            finding: finding.clone(),
            closed: None,
        };
        let open_finding = FeasibilityFinding {
            key: FindingKey {
                component_id: id(602),
                product_revision_id,
                kind: FeasibilityRuleKind::BendSpacing,
                check: FindingCheck::BendSpacing { first_bend_index: 2 },
            },
            state: FindingState::Unresolved,
            unresolved_reason: Some(UnresolvedReason::MissingMeasurement),
            measurements: Vec::new(),
            applied_rules: Vec::new(),
            required_limit: None,
        };
        let assessment = FeasibilityAssessment {
            version: 3,
            rule_table_version_id: id(700),
            engineering_record_version: 5,
            created_by: "engineer@example.test".to_string(),
            created_at: "2026-11-10T09:00:00Z".to_string(),
            components: vec![assessed_component(component_id, product_revision_id)],
            findings: vec![finding.clone(), open_finding.clone()],
        };
        ExportFeasibility {
            version: 3,
            pinned: true,
            view: FeasibilityRecordView {
                project_id: id(1),
                version: 3,
                latest_assessment: Some(FeasibilityAssessmentView {
                    assessment,
                    findings: vec![
                        FeasibilityFindingView { finding, disposition: FindingDisposition::ExceptionAccepted { exception_id: id(601) } },
                        FeasibilityFindingView { finding: open_finding, disposition: FindingDisposition::Open },
                    ],
                }),
                exceptions: vec![FeasibilityExceptionView { exception, state: ExceptionState::Active, review_required_reasons: Vec::new() }],
                issues: Vec::new(),
            },
        }
    }

    fn issue(index: usize) -> ExportIssue {
        let sources = [
            ExportIssueSource::Costing,
            ExportIssueSource::Tooling,
            ExportIssueSource::Commercial,
            ExportIssueSource::Feasibility,
            ExportIssueSource::Staleness,
        ];
        let message = match index {
            7 => "bad\u{1}control\u{0}value".to_string(),
            9 => "A stored explanation that is deliberately long to exercise wrapping and row height growth in the workbook. ".repeat(3),
            _ => format!("Missing input number {index}: “quoted” text with ünïcode and a & ampersand"),
        };
        ExportIssue { source: sources[index % sources.len()], code: format!("ISSUE_{index:03}"), scope: Some(id(index as u128 + 10)), message }
    }

    fn fixture(years: &[u16]) -> ExportModel {
        let parts: Vec<ExportPart> = (1..=103).map(|index| fixture_part(index, years)).collect();
        let products = fixture_products(years, &parts);
        ExportModel {
            metadata: metadata(ExportBaselineSelection::AcceptedBaseline { accepted_baseline_id: id(2), sequence: 7 }),
            scope: ExportScope {
                project_name: "Three boxes & brackets".to_string(),
                customer: "Internal \"pilot\"".to_string(),
                internal_team: Some("Pilot team".to_string()),
                platform: None,
                application: Some("Demo".to_string()),
                currency: "CNY".to_string(),
                default_schedule: schedule(years),
                products: vec![fixture_scoped_product(years)],
            },
            parts,
            products,
            setup: vec![
                BaselineSetupLine {
                    owner: BaselineSetupOwner::PartVersion { part_version_id: id(1) },
                    setup_charge: Some(dec("42.00")),
                    explanations: Vec::new(),
                },
                BaselineSetupLine {
                    owner: BaselineSetupOwner::Product { product_id: id(1000) },
                    setup_charge: Some(dec("84.00")),
                    explanations: Vec::new(),
                },
            ],
            setup_categories: vec![BaselineCategoryLine {
                code: "setup-tool".to_string(),
                name: "Tool setup".to_string(),
                amount: Some(dec("126.00")),
            }],
            setup_total: Some(dec("126.00")),
            annual: years.iter().map(|year| BaselineAnnualProjectCost { year: *year, manufacturing_cost: Some(dec("1000.00")) }).collect(),
            project_total_manufacturing_cost: Some(dec("3000.00")),
            tooling: Some(tooling_outcome(years)),
            scenarios: vec![ExportScenario {
                scenario_id: id(300),
                version: 3,
                name: "Base case".to_string(),
                stale: true,
                stale_reasons: vec![CommercialStaleReason::new("scenario_baseline_outdated", "the saved scenario pinned an older baseline")],
                outcome: commercial_outcome(years),
            }],
            feasibility: Some(feasibility(years)),
            issues: (0..150).map(issue).collect(),
        }
    }

    fn empty_model() -> ExportModel {
        ExportModel {
            metadata: metadata(ExportBaselineSelection::DraftCalculation { calculation_id: id(3) }),
            scope: ExportScope {
                project_name: "Empty project".to_string(),
                customer: String::new(),
                internal_team: None,
                platform: None,
                application: None,
                currency: "EUR".to_string(),
                default_schedule: schedule(&[2027]),
                products: Vec::new(),
            },
            parts: Vec::new(),
            products: Vec::new(),
            setup: Vec::new(),
            setup_categories: Vec::new(),
            setup_total: None,
            annual: Vec::new(),
            project_total_manufacturing_cost: None,
            tooling: None,
            scenarios: Vec::new(),
            feasibility: None,
            issues: Vec::new(),
        }
    }

    /// A minimal independent read of the stored archive entries: name and uncompressed bytes.
    fn read_entries(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
        let eocd = bytes.len().checked_sub(22).expect("the archive has an end record");
        assert_eq!(&bytes[eocd..eocd + 4], &[0x50, 0x4b, 0x05, 0x06], "end of central directory signature");
        let count = usize::from(u16::from_le_bytes(bytes[eocd + 10..eocd + 12].try_into().unwrap()));
        let mut position = usize::try_from(u32::from_le_bytes(bytes[eocd + 16..eocd + 20].try_into().unwrap())).unwrap();
        let mut entries = Vec::new();
        for _ in 0..count {
            assert_eq!(&bytes[position..position + 4], &[0x50, 0x4b, 0x01, 0x02], "central header signature");
            let size = usize::try_from(u32::from_le_bytes(bytes[position + 24..position + 28].try_into().unwrap())).unwrap();
            let name_len = usize::from(u16::from_le_bytes(bytes[position + 28..position + 30].try_into().unwrap()));
            let extra_len = usize::from(u16::from_le_bytes(bytes[position + 30..position + 32].try_into().unwrap()));
            let comment_len = usize::from(u16::from_le_bytes(bytes[position + 32..position + 34].try_into().unwrap()));
            let local = usize::try_from(u32::from_le_bytes(bytes[position + 42..position + 46].try_into().unwrap())).unwrap();
            let name = std::str::from_utf8(&bytes[position + 46..position + 46 + name_len]).unwrap().to_string();
            let local_name_len = usize::from(u16::from_le_bytes(bytes[local + 26..local + 28].try_into().unwrap()));
            let local_extra_len = usize::from(u16::from_le_bytes(bytes[local + 28..local + 30].try_into().unwrap()));
            let data_start = local + 30 + local_name_len + local_extra_len;
            entries.push((name, bytes[data_start..data_start + size].to_vec()));
            position += 46 + name_len + extra_len + comment_len;
        }
        entries
    }

    fn text_entry<'a>(entries: &'a [(String, Vec<u8>)], name: &str) -> &'a str {
        let (_, data) = entries.iter().find(|(entry_name, _)| entry_name == name).expect("the archive holds the entry");
        std::str::from_utf8(data).expect("the part is UTF-8 text")
    }

    /// Read the sheet names in workbook order from the workbook part.
    fn sheet_names(workbook: &str) -> Vec<String> {
        workbook
            .split("<sheet ")
            .skip(1)
            .filter_map(|chunk| {
                let start = chunk.find("name=\"")? + 6;
                let rest = &chunk[start..];
                let end = rest.find('"')?;
                Some(rest[..end].to_string())
            })
            .collect()
    }

    fn sheet(entries: &[(String, Vec<u8>)], index: usize) -> &str {
        let name = format!("xl/worksheets/sheet{index}.xml");
        text_entry(entries, &name)
    }

    #[test]
    fn the_workbook_has_the_expected_sheets_styles_and_a_known_value() {
        let model = fixture(&YEARS);
        let entries = read_entries(&render(&model));
        let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "[Content_Types].xml",
                "_rels/.rels",
                "docProps/core.xml",
                "docProps/app.xml",
                "xl/workbook.xml",
                "xl/_rels/workbook.xml.rels",
                "xl/styles.xml",
                "xl/worksheets/sheet1.xml",
                "xl/worksheets/sheet2.xml",
                "xl/worksheets/sheet3.xml",
                "xl/worksheets/sheet4.xml",
                "xl/worksheets/sheet5.xml",
                "xl/worksheets/sheet6.xml",
                "xl/worksheets/sheet7.xml",
                "xl/worksheets/sheet8.xml",
                "xl/worksheets/sheet9.xml",
            ]
        );
        assert_eq!(
            sheet_names(text_entry(&entries, "xl/workbook.xml")),
            ["Summary", "Products", "BOM", "Routings", "Feasibility", "Costs", "Tooling", "Scenarios", "Issues"]
        );
        let content_types = text_entry(&entries, "[Content_Types].xml");
        for index in 1..=9 {
            assert!(content_types.contains(&format!("/xl/worksheets/sheet{index}.xml")), "content type for sheet {index}");
        }
        assert_eq!(content_types.matches("spreadsheetml.worksheet+xml").count(), 9);

        let styles = text_entry(&entries, "xl/styles.xml");
        for format in ["#,##0.00", "#,##0", "0.0000", "0.0000"] {
            assert!(styles.contains(format), "the style sheet carries {format}");
        }
        assert!(styles.contains("wrapText=\"1\""));

        // Summary states the exact selected result, versions, pinned feasibility record and labels.
        let summary = sheet(&entries, 1);
        assert!(summary.contains("accepted baseline #7"));
        assert!(summary.contains("pinned to the selected result"));
        assert!(summary.contains("outdated_costs, superseded_accepted_baseline"));
        assert!(summary.contains(">4</v>"), "the project version is a numeric cell");
        assert!(summary.contains(">150</v>"), "the issue count is a numeric cell");

        // Products: a known money cell parses back from its reference and a forecast column exists.
        let products = sheet(&entries, 2);
        assert!(products.contains("<c r=\"N2\" s=\"2\"><v>123.45</v></c>"), "the first product's unit total is numeric money");
        assert!(products.contains("<t xml:space=\"preserve\">Housing &amp; cover</t>"), "a product name round-trips as an inline string");
        assert!(products.contains("Forecast 2027") && products.contains("Forecast 2028") && products.contains("Forecast 2029"));
        assert!(products.contains("<autoFilter ref=\"A1:R3\"/>"), "the product table has an autofilter");
        assert!(products.contains("<pane ySplit=\"1\""), "the product header row is frozen");
        assert!(products.contains("orientation=\"landscape\"") && products.contains("fitToWidth=\"1\""));

        // The repeated print titles are workbook-level defined names.
        let workbook = text_entry(&entries, "xl/workbook.xml");
        assert!(workbook.contains("<definedName name=\"_xlnm.Print_Titles\" localSheetId=\"1\">'Products'!$1:$1</definedName>"));

        // The BOM has one row per product × part version and demand columns after the year set.
        let bom = sheet(&entries, 3);
        assert!(bom.contains("<dimension ref=\"A1:N104\"/>"), "103 BOM rows plus the header, 14 columns");
        assert!(bom.contains("<row r=\"104\""));
        assert!(bom.contains("Demand 2029"));
        // Routings carry both owner kinds and the pinned rate identity.
        let routings = sheet(&entries, 4);
        assert!(routings.contains("part version") && routings.contains("product"));
        assert!(routings.contains(">12.5</v>"));
        // Feasibility carries the finding and its accepted exception.
        let feasibility = sheet(&entries, 5);
        assert!(feasibility.contains("minimum_bend_radius"));
        assert!(feasibility.contains("minimum radius 3.0 mm"));
        assert!(feasibility.contains("engineer@example.test"));
        assert!(feasibility.contains(">active<"));
        // Costs holds its labelled blocks.
        let costs = sheet(&entries, 6);
        for title in [
            "Part cost lines",
            "Product cost lines",
            "Product cost categories",
            "Product root attached cost lines",
            "Part material records",
            "Part purchase prices",
            "Part attached cost lines",
            "Separate setup lines",
            "Setup categories",
            "Annual project manufacturing cost",
            "Project totals",
        ] {
            assert!(costs.contains(title), "the costs sheet has the {title} block");
        }
        // Tooling has the totals, the tool row and an allocation row.
        let tooling = sheet(&entries, 7);
        assert!(tooling.contains("Project investment") && tooling.contains("Recovery 2028"));
        assert!(tooling.contains("<t xml:space=\"preserve\">amortized</t>"));
        // Scenarios has the summary and the per-year detail.
        let scenarios = sheet(&entries, 8);
        assert!(scenarios.contains("Scenario summary") && scenarios.contains("Scenario details"));
        assert!(scenarios.contains("Base case"));
        assert!(scenarios.contains("scenario_baseline_outdated"));
        // Issues lists every issue.
        let issues = sheet(&entries, 9);
        assert!(issues.contains("<dimension ref=\"A1:D151\"/>"));
        assert!(issues.contains("ISSUE_149"));
    }

    #[test]
    fn year_columns_follow_the_data() {
        let model = fixture(&[2030, 2031]);
        let entries = read_entries(&render(&model));
        let products = sheet(&entries, 2);
        assert!(products.contains("Forecast 2030") && products.contains("Forecast 2031"));
        assert!(!products.contains("Forecast 2027"));
        // Products has 15 fixed columns plus two forecast columns.
        assert!(products.contains("<autoFilter ref=\"A1:Q3\"/>"));
        let bom = sheet(&entries, 3);
        assert!(bom.contains("Demand 2030") && !bom.contains("Demand 2027"));
    }

    #[test]
    fn the_same_model_renders_the_same_bytes() {
        let model = fixture(&YEARS);
        let first = render(&model);
        let second = render(&model);
        assert_eq!(first, second, "the renderer must be a pure function of the model");
        // A fixed DOS timestamp: the first local header carries 1980-01-01T00:00:00.
        assert_eq!(&first[10..12], &0u16.to_le_bytes(), "fixed DOS time");
        assert_eq!(&first[12..14], &0x0021u16.to_le_bytes(), "fixed DOS date");
        // A different model with other years changes the bytes.
        assert_ne!(first, render(&fixture(&[2030, 2031])));
    }

    #[test]
    fn text_is_xml_escaped_and_invalid_control_characters_are_replaced() {
        let mut model = fixture(&YEARS);
        model.metadata.project_name = "A&B <C> \"D\" 'E'".to_string();
        let entries = read_entries(&render(&model));
        let summary = sheet(&entries, 1);
        assert!(summary.contains("A&amp;B &lt;C&gt; &quot;D&quot; &apos;E&apos;"), "every markup character is escaped");
        assert!(!summary.contains("<C>"), "the raw markup is not written");
        let core = text_entry(&entries, "docProps/core.xml");
        assert!(core.contains("A&amp;B &lt;C&gt; &quot;D&quot; &apos;E&apos;"));
        // The fixture's eighth issue holds a C0 control character that XML 1.0 forbids.
        let issues = sheet(&entries, 9);
        assert!(issues.contains("bad\u{fffd}control\u{fffd}value"));
        assert!(!issues.contains('\u{1}') && !issues.contains('\u{0}'), "no invalid control character reaches the part");
    }

    #[test]
    fn the_large_fixture_writes_every_row_and_sheet() {
        let model = fixture(&YEARS);
        assert!(model.issues.len() >= 150);
        let bom_rows: usize = model.products.iter().map(|product| product.parts.len()).sum();
        assert!(bom_rows >= 100);
        let entries = read_entries(&render(&model));
        let issues = sheet(&entries, 9);
        assert_eq!(issues.matches("<row ").count(), 151, "one header row and 150 issue rows");
        assert!(issues.contains("<row r=\"11\" ht="), "a wrapped long message gets a taller row");
        let routings = sheet(&entries, 4);
        assert!(routings.matches("<row ").count() >= 22, "part and product-root routing rows are written");
        let costs = sheet(&entries, 6);
        assert!(costs.matches("<row ").count() >= 103, "every part cost line is written");
    }

    #[test]
    fn an_empty_or_sparse_model_still_renders_a_valid_workbook() {
        let model = empty_model();
        let bytes = render(&model);
        let entries = read_entries(&bytes);
        assert_eq!(entries.len(), 16, "seven package parts and nine worksheets");
        let names = sheet_names(text_entry(&entries, "xl/workbook.xml"));
        assert_eq!(names, ["Summary", "Products", "BOM", "Routings", "Feasibility", "Costs", "Tooling", "Scenarios", "Issues"]);
        // Every table sheet still has its header row and no data rows.
        assert_eq!(sheet(&entries, 2), sheet(&entries, 2));
        assert!(sheet(&entries, 3).contains("<dimension ref=\"A1:K1\"/>"));
        assert!(sheet(&entries, 4).contains("<dimension ref=\"A1:O1\"/>"));
        assert!(sheet(&entries, 5).contains("<dimension ref=\"A1:L1\"/>"));
        assert!(sheet(&entries, 9).contains("<dimension ref=\"A1:D1\"/>"));
        assert!(!sheet(&entries, 2).contains("<autoFilter"), "a header-only table has no autofilter");
        // The block sheets still carry their titles even without data.
        for title in ["Part cost lines", "Separate setup lines", "Project totals"] {
            assert!(sheet(&entries, 6).contains(title));
        }
        assert!(sheet(&entries, 7).contains("not selected"));
        assert!(sheet(&entries, 8).contains("Scenario summary"));
        // Re-rendering the empty model stays deterministic.
        assert_eq!(bytes, render(&model));

        // A None-heavy model with one part and one product exercises the blank cycle/setup columns
        // and the product rows a scope does not list.
        let mut sparse = empty_model();
        let mut part = fixture_part(200, &[]);
        part.operations = vec![ExportOperation { sequence: 1, operation_id: id(77), cycle: None, setup: None }];
        part.annual_demand = Vec::new();
        sparse.parts.push(part);
        let mut product = fixture_products(&[], &[]).remove(1);
        product.root_operations = vec![ExportOperation { sequence: 2, operation_id: id(78), cycle: None, setup: None }];
        product.parts = Vec::new();
        sparse.products.push(product);
        let sparse_bytes = render(&sparse);
        let sparse_entries = read_entries(&sparse_bytes);
        let routings = sheet(&sparse_entries, 4);
        assert!(routings.contains("<dimension ref=\"A1:O3\"/>"), "a part row and a product row plus the header");
        assert_eq!(routings.matches("<c r=").count(), 25, "each None routing row keeps only its five owner cells");
        assert!(routings.contains("part version") && routings.contains("product"));
        // The product without a scope row still reaches the Products sheet, from its costed line.
        let products = sheet(&sparse_entries, 2);
        assert!(products.contains("<dimension ref=\"A1:O2\"/>"));
        assert!(products.contains("Bracket"));
        assert_eq!(sparse_bytes, render(&sparse));
    }
    /// Write a rendered fixture workbook so an independent parser can open it: the SB-22 evidence
    /// run uses openpyxl on this file. Ignored by default because it writes an evidence artifact.
    #[test]
    #[ignore = "writes an evidence workbook into target/ on demand"]
    fn write_evidence_workbook_for_an_independent_parser() {
        let model = fixture(&YEARS);
        let bytes = render(&model);
        let path = std::path::Path::new("../../target/solidbom-export-xlsx-evidence.xlsx");
        assert!(std::fs::write(path, &bytes).is_ok(), "the evidence workbook is written");
    }
}
