//! Shared fixtures and an isolated PostgreSQL schema for the service's integration tests.
//! Each test binary uses a different part of it, so unused items are expected here.
#![allow(dead_code, clippy::expect_used, clippy::panic, clippy::unwrap_used)]

use std::str::FromStr;

use rust_decimal::Decimal;
use sha2::{Digest, Sha256};
use solidbom_contract::{
    AnalysisKey, AnalysisManifest, CatalogAttribute, CategoryBasis, CategoryCalculation, CostCategory, CostCenterPurpose, CreateAppConfigVersion,
    CreateComponentVersion, CreateMachineRateVersion, CreateMaterialVersion, CreateRateVersion, CycleBasis, DefinitionRef, DirectCostInput,
    DraftProject, GeometryBinding, MachineIdentity, MachineRateInput, Measurement, MeasurementSource, OccurrenceRef, OperationInputs, PartInputs,
    ProcessType, PublishMachineRateVersion, RateScenario, ReviewState, SCHEMA_VERSION, SaveDraft, SourceRef, Sourcing, Unit,
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

/// The development tokens the API tests configure. The legacy token lives in `tests/api.rs`.
pub const ENGINEER_TOKEN: &str = "solidbom-engineer-token-0123456789";
pub const ADMIN_TOKEN: &str = "solidbom-administrator-token-0123456789";

/// The seeded material version id; selecting it is what an existing project version stored.
pub fn seeded_material_version_id() -> Uuid {
    material_id()
}

/// The seeded rate version id; selecting it is what an existing project version stored.
pub fn seeded_rate_version_id() -> Uuid {
    rate_id()
}

/// A material publish request for the shared-data tests.
pub fn material_version(request_id: &str, expected_version: u64, material_id: Uuid, density: &str, price: Option<&str>) -> CreateMaterialVersion {
    CreateMaterialVersion {
        request_id: request_id.into(),
        expected_version,
        material_id,
        name: "PA12 test fixture".into(),
        density_g_cm3: d(density),
        price_per_kg: price.map(d),
        currency: "CNY".into(),
        source_reference: Some("supplier sheet 2026-11".into()),
    }
}

/// A cost-center publish request for the shared-data tests.
pub fn rate_version(
    request_id: &str,
    expected_version: u64,
    cost_center_id: Uuid,
    rate: Option<&str>,
    purpose: CostCenterPurpose,
) -> CreateRateVersion {
    CreateRateVersion {
        request_id: request_id.into(),
        expected_version,
        cost_center_id,
        name: "Machining test fixture".into(),
        purpose,
        rate_per_hour: rate.map(d),
        currency: "CNY".into(),
        source_reference: Some("shop rate card 2026-11".into()),
    }
}

/// A catalog-component publish request for the shared-data tests.
pub fn component_version(request_id: &str, expected_version: u64, component_id: Uuid, price: Option<&str>) -> CreateComponentVersion {
    CreateComponentVersion {
        request_id: request_id.into(),
        expected_version,
        component_id,
        name: "M6 bolt".into(),
        specification: "DIN 912 M6 x 20, 8.8".into(),
        price: price.map(d),
        currency: "CNY".into(),
        source_reference: Some("catalog page 12".into()),
        attributes: Vec::new(),
    }
}

/// A catalog-component publish request whose declared matching attributes are set; catalog matching
/// reads these administrator-declared values, never the published name alone.
pub fn component_version_with_attributes(
    request_id: &str,
    expected_version: u64,
    component_id: Uuid,
    name: &str,
    specification: &str,
    price: Option<&str>,
    attributes: &[(&str, &str)],
) -> CreateComponentVersion {
    CreateComponentVersion {
        request_id: request_id.into(),
        expected_version,
        component_id,
        name: name.into(),
        specification: specification.into(),
        price: price.map(d),
        currency: "CNY".into(),
        source_reference: Some("catalog page 12".into()),
        attributes: attributes.iter().map(|(key, value)| CatalogAttribute { key: (*key).into(), value: (*value).into() }).collect(),
    }
}

/// An app-config publish request with the given categories.
pub fn app_config_version(request_id: &str, expected_version: u64, config_id: Uuid, categories: Vec<CostCategory>) -> CreateAppConfigVersion {
    CreateAppConfigVersion {
        request_id: request_id.into(),
        expected_version,
        config_id,
        currency: "CNY".into(),
        categories,
        source_reference: Some("app configuration sheet".into()),
    }
}

/// A calculated direct-cost category.
pub fn direct_category(code: &str, input: DirectCostInput) -> CostCategory {
    CostCategory { code: code.into(), name: code.into(), calculation: CategoryCalculation::Direct { input }, included_in_ebit: false }
}

/// A fixed-amount category.
pub fn fixed_category(code: &str, amount: &str) -> CostCategory {
    CostCategory { code: code.into(), name: code.into(), calculation: CategoryCalculation::Fixed { amount: d(amount) }, included_in_ebit: true }
}

/// A percentage category over an explicit basis: the direct-cost subtotal and/or named codes.
pub fn percentage_category(code: &str, direct_cost: bool, basis_codes: &[&str], rate: &str) -> CostCategory {
    let basis = CategoryBasis { direct_cost, categories: basis_codes.iter().map(|code| (*code).to_string()).collect() };
    CostCategory {
        code: code.into(),
        name: code.into(),
        calculation: CategoryCalculation::Percentage { basis, rate: d(rate) },
        included_in_ebit: true,
    }
}

/// The stable machine id of the machine-rate fixture. It is also the SB-09 cost-center id the
/// publication appends to.
pub fn machine_rate_machine_id() -> Uuid {
    Uuid::from_u128(0x5b01_0001)
}

/// The complete machine-rate input of the service tests: one extrusion line whose recommended
/// arithmetic is exact in every field (4 000 scheduled hours, 2 400 running hours, installed CAPEX
/// 130 000, annual depreciation 12 000, annual fixed operating cost 129 000, variable cost
/// excluding standby 84 per running hour and total 144.75 CNY per running hour). Every
/// authoritative numeric input is explicit; nothing is defaulted.
pub fn machine_rate_input() -> MachineRateInput {
    machine_rate_input_for(machine_rate_machine_id())
}

/// The same complete input on another machine id, for inventory and chain-order tests.
pub fn machine_rate_input_for(machine_id: Uuid) -> MachineRateInput {
    MachineRateInput {
        machine: MachineIdentity {
            machine_id,
            code: Some("CC-1001".into()),
            name: "Extrusion Line 1".into(),
            process_type: ProcessType::Extrusion,
            process_description: Some("Sheet extrusion line".into()),
            brand: Some("Example".into()),
            model: Some("EX-1".into()),
            acquisition_year: Some(2020),
            active: true,
        },
        financial_year: Some(2026),
        scenario: RateScenario::Base,
        working_days_per_year: Some(d("250")),
        shifts_per_day: Some(d("2")),
        hours_per_shift: Some(d("8")),
        loading: Some(d("0.8")),
        availability: Some(d("0.75")),
        machine_purchase_capex: Some(d("100000")),
        installation_capex: Some(d("20000")),
        dedicated_auxiliary_capex: Some(d("10000")),
        residual_value: Some(d("10000")),
        useful_life_years: Some(d("10")),
        remaining_depreciation_years: None,
        annual_maintenance: Some(d("6000")),
        annual_insurance: Some(d("1200")),
        floor_space_m2: Some(d("200")),
        annual_premises_rate_per_m2: Some(d("6")),
        other_fixed_machine_overhead: Some(d("600")),
        dedicated_annual_labor: Some(d("0")),
        committed_operators: Some(d("1")),
        committed_labor_base_hourly_pay: Some(d("30")),
        committed_labor_burden_rate: Some(d("0.25")),
        installed_running_power_kw: Some(d("50")),
        average_running_power_factor: Some(d("0.8")),
        electricity_tariff: Some(d("1")),
        standby_power_kw: Some(d("6")),
        compressed_air_nm3_per_running_hour: Some(d("5")),
        compressed_air_tariff: Some(d("0.4")),
        cooling_water_m3_per_running_hour: Some(d("2")),
        cooling_water_tariff: Some(d("1")),
        other_utilities_per_running_hour: Some(d("1")),
        wear_parts_per_running_hour: Some(d("2")),
        operating_consumables_per_running_hour: Some(d("1")),
        run_only_operators: Some(d("1")),
        run_only_labor_base_hourly_pay: Some(d("30")),
        run_only_labor_burden_rate: Some(d("0.2")),
        other_variable_cost_per_running_hour: Some(d("0")),
        source_reference: Some("SB-X01 service fixture".into()),
        assumption_notes: None,
    }
}

/// A create request for one machine rate version.
pub fn machine_rate_create(request_id: &str, expected_version: u64, input: MachineRateInput) -> CreateMachineRateVersion {
    CreateMachineRateVersion { request_id: request_id.into(), expected_version, input }
}

/// A publish request for one machine rate version.
pub fn machine_rate_publish(request_id: &str, expected_version: u64) -> PublishMachineRateVersion {
    PublishMachineRateVersion { request_id: request_id.into(), expected_version }
}
