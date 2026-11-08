//! Shared fixtures and an isolated PostgreSQL schema for the service's integration tests.
//! Each test binary uses a different part of it, so unused items are expected here.
#![allow(dead_code, clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::str::FromStr;

use rust_decimal::Decimal;
use sha2::{Digest, Sha256};
use solidbom_contract::{
    AnalysisKey, AnalysisManifest, CycleBasis, DefinitionRef, DraftProject, GeometryBinding, Measurement, MeasurementSource, OccurrenceRef,
    OperationInputs, PartInputs, ReviewState, SCHEMA_VERSION, SaveDraft, SourceRef, Sourcing, Unit,
};
use solidbom_server::Store;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use uuid::Uuid;

pub const FIXTURE_SOURCE: &[u8] =
    b"ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('SolidBOM test fixture'),'2;1');\nENDSEC;\nDATA;\nENDSEC;\nEND-ISO-10303-21;\n";
pub const IDENTITY: [[f64; 4]; 4] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];

pub fn d(text: &str) -> Decimal {
    Decimal::from_str(text).expect("a decimal in the fixture")
}

pub fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn material_id() -> Uuid {
    Uuid::parse_str("00000000-0000-4000-8000-000000000001").expect("the seeded material fixture")
}

pub fn rate_id() -> Uuid {
    Uuid::parse_str("00000000-0000-4000-8000-000000000101").expect("the seeded rate fixture")
}

pub fn key() -> AnalysisKey {
    AnalysisKey { source_sha256: digest(FIXTURE_SOURCE), engine_build: "0.40.0-rc.2".into(), extractor_version: "1".into() }
}

pub fn measurement(value: Option<&str>) -> Measurement {
    Measurement {
        value: value.map(d),
        unit: Unit::MillimetreCubed,
        source: MeasurementSource::Extracted,
        review: ReviewState::Confirmed,
        reason: None,
        analysis_key: key(),
    }
}

pub fn part(part_version_id: Uuid, definition_id: Uuid) -> PartInputs {
    PartInputs {
        part_version_id,
        definition_id,
        classification: "clip".into(),
        classification_confirmed: true,
        sourcing: Sourcing::Manufactured,
        volume_mm3: measurement(Some("2400")),
        material_version_id: Some(material_id()),
        allowance_rate: Some(d("0.02")),
        operation: Some(OperationInputs {
            cycle_seconds: Some(d("30")),
            cycle_basis: CycleBasis::PerPiece,
            cavities: Some(1),
            production_rate_version_id: Some(rate_id()),
            setup_hours: Some(d("0.5")),
            setup_rate_version_id: Some(rate_id()),
        }),
    }
}

pub fn one_definition(key: AnalysisKey) -> AnalysisManifest {
    let definition_id = Uuid::from_u128(0xdef1);
    let occurrences: Vec<OccurrenceRef> = (0u64..3)
        .map(|index| OccurrenceRef {
            occurrence_id: Uuid::from_u128(0x0cc0 + u128::from(index)),
            definition_id,
            parent_occurrence_id: None,
            source_path: vec![index],
            transform: IDENTITY,
        })
        .collect();
    let bindings: Vec<GeometryBinding> = occurrences
        .iter()
        .map(|occurrence| GeometryBinding {
            source_path: occurrence.source_path.clone(),
            source_body: 0,
            definition_id,
            occurrence_id: occurrence.occurrence_id,
        })
        .collect();
    AnalysisManifest {
        schema_version: SCHEMA_VERSION,
        key: key.clone(),
        definitions: vec![DefinitionRef {
            definition_id,
            source_key: key,
            customer_part_number: Some("PL-100".into()),
            customer_revision: Some("B".into()),
            volume_mm3: measurement(Some("2400")),
        }],
        occurrences,
        geometry_bindings: bindings,
        issues: Vec::new(),
    }
}

pub fn fixture_draft(version: u64, source: &SourceRef) -> DraftProject {
    DraftProject {
        project_id: Uuid::from_u128(0x9001),
        product_revision_id: Uuid::from_u128(0x9002),
        version,
        source: source.clone(),
        analysis: one_definition(key()),
        material_overhead_rate: Some(d("0.05")),
        parts: vec![part(Uuid::from_u128(0xaa01), Uuid::from_u128(0xdef1))],
        forecast_year: 2027,
        forecast_quantity: 1000,
        currency: "CNY".into(),
    }
}

/// The approved fixture with a two-cavity tool: the calculator divides the 30-second cycle.
pub fn two_cavity_draft(version: u64, source: &SourceRef) -> DraftProject {
    let mut draft = fixture_draft(version, source);
    draft.parts[0].operation.as_mut().expect("operation").cavities = Some(2);
    draft
}

/// A reviewed manual volume (2500 mm³) on the same approved material and operation values.
pub fn manual_volume_draft(version: u64, source: &SourceRef) -> DraftProject {
    let mut draft = fixture_draft(version, source);
    draft.parts[0].volume_mm3 = Measurement {
        value: Some(d("2500")),
        unit: Unit::MillimetreCubed,
        source: MeasurementSource::Manual,
        review: ReviewState::Confirmed,
        reason: Some("open surface; reviewed by an engineer".into()),
        analysis_key: key(),
    };
    draft
}

/// The approved fixture with the production inputs missing: no cycle time, cavities or rate.
pub fn missing_cycle_draft(version: u64, source: &SourceRef) -> DraftProject {
    let mut draft = fixture_draft(version, source);
    let operation = draft.parts[0].operation.as_mut().expect("operation");
    operation.cycle_seconds = None;
    operation.cavities = None;
    operation.production_rate_version_id = None;
    draft
}

/// An explicit zero cycle: valid, distinct from a missing value, and it needs no rate or cavity
/// count because it cannot change the production cost.
pub fn zero_cycle_draft(version: u64, source: &SourceRef) -> DraftProject {
    let mut draft = fixture_draft(version, source);
    let operation = draft.parts[0].operation.as_mut().expect("operation");
    operation.cycle_seconds = Some(d("0"));
    operation.cavities = None;
    operation.production_rate_version_id = None;
    draft
}

pub fn two_revision_draft(version: u64, source: &SourceRef) -> DraftProject {
    let mut draft = fixture_draft(version, source);
    let definition_b = draft.analysis.definitions[0].clone();
    let mut definition_c = definition_b.clone();
    definition_c.definition_id = Uuid::from_u128(0xdef2);
    definition_c.customer_revision = Some("C".into());
    let occurrence_c = OccurrenceRef {
        occurrence_id: Uuid::from_u128(0x0cc9),
        definition_id: definition_c.definition_id,
        parent_occurrence_id: None,
        source_path: vec![3],
        transform: IDENTITY,
    };
    draft.analysis.definitions.push(definition_c);
    draft.analysis.geometry_bindings.push(GeometryBinding {
        source_path: occurrence_c.source_path.clone(),
        source_body: 0,
        definition_id: occurrence_c.definition_id,
        occurrence_id: occurrence_c.occurrence_id,
    });
    draft.analysis.occurrences.push(occurrence_c);
    draft.parts.push(part(Uuid::from_u128(0xaa02), Uuid::from_u128(0xdef2)));
    draft
}

pub struct TestDb {
    pub store: Store,
    pub options: PgConnectOptions,
    pub schema: String,
    pub url: String,
}

impl TestDb {
    pub async fn new() -> Self {
        let url = std::env::var("SOLIDBOM_TEST_DATABASE_URL").expect("set SOLIDBOM_TEST_DATABASE_URL to a disposable test database");
        let schema = format!("t_{}", Uuid::new_v4().simple());
        let admin = PgPoolOptions::new().max_connections(1).connect(&url).await.expect("connect the test database");
        sqlx::query(&format!("CREATE SCHEMA {schema}")).execute(&admin).await.expect("create the test schema");
        let options = PgConnectOptions::from_str(&url).expect("parse the test database URL").options([("search_path", schema.as_str())]);
        let store = Store::connect_with_options(options.clone()).await.expect("migrate the test schema");
        Self { store, options, schema, url }
    }

    pub async fn reconnect(&self) -> Store {
        Store::connect_with_options(self.options.clone()).await.expect("reconnect the test schema")
    }

    pub async fn cleanup(&self) {
        if let Ok(admin) = PgPoolOptions::new().max_connections(1).connect(&self.url).await {
            let _ = sqlx::query(&format!("DROP SCHEMA {} CASCADE", self.schema)).execute(&admin).await;
        }
    }
}

pub async fn upload(store: &Store) -> SourceRef {
    store.put_source(&digest(FIXTURE_SOURCE), "three_boxes.step", FIXTURE_SOURCE).await.expect("store the source")
}

pub async fn create(store: &Store, source: &SourceRef) -> SaveDraft {
    let save = SaveDraft { request_id: "create-1".into(), expected_version: 0, draft: fixture_draft(0, source) };
    let receipt = store.create_draft(&save).await.expect("create the project");
    assert_eq!(receipt.version, 1);
    save
}

/// How many rows a test's private schema holds after an operation. `table` is always a literal in
/// the test source, never client input.
pub async fn row_count(store: &Store, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT count(*) FROM {table}")).fetch_one(store.pool()).await.expect("count the rows")
}

/// One definition whose component grouping is unresolved; the numeric inputs are complete.
pub fn unresolved_draft(version: u64, source: &SourceRef) -> DraftProject {
    let mut draft = fixture_draft(version, source);
    let definition_id = draft.analysis.definitions[0].definition_id;
    draft.analysis.issues.push(
        solidbom_contract::Issue::new("unresolved_grouping", "analysis.definitions", "the grouping is unresolved").for_definition(definition_id),
    );
    draft
}
