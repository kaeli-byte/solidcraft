//! Machine hourly-rate versions and publication (gated package SB-X01, machine hourly-rate contract
//! version 1 additive to API version 2).
//!
//! One row of `machine_rate_versions` is one immutable version of one (machine, financial year,
//! scenario) chain. Creating a version validates the portable [`solidbom_contract::MachineRateInput`]
//! wire shape, locks the chain, checks the caller's `expected_version`, calculates the recommended
//! outcome with the pure engine from `solidbom_rates` and stores the input snapshot and the draft.
//! A draft is not an approved rate: it may be invalid (a missing financial year is recorded, never
//! invented) and it may be recalculated for display, but it is never published in parity mode.
//!
//! Publishing locks the version row and recalculates the recommended outcome. It refuses an invalid
//! or zero-running-hour outcome and an incomplete machine identity (no cost-center code), then, in
//! the same transaction, appends the next immutable SB-09 cost-center `rate_versions` row with
//! `purpose = production`, unit `cny_h`, currency `CNY`, the fully absorbed total rate as
//! `rate_per_hour` and `ACTUAL_RUNNING_HOUR` as the denominator in its provenance, appends the
//! cost-center [`solidbom_contract::ReferenceChange`] fact, and freezes the approved outcome snapshot
//! and calculator version onto the machine rate version. A published version is never edited: a
//! later version supersedes it and appends a newer SB-09 rate version.
//!
//! Reads of a draft calculate with the current engine; reads of a published version answer from the
//! stored approval snapshot, so a later calculator change cannot rewrite an approved result. A
//! corrupt stored input or snapshot is the internal codec error, never a silent recalculation.
//!
//! Loading sensitivity is a pure read: it copies the stored input snapshot, changes only `loading`,
//! calculates one outcome per requested loading and writes nothing. Sensitivity at the input's base
//! loading equals the base outcome exactly, and a loading of zero is a valid `no_run_hours` scenario.

use solidbom_contract::{
    CalculationMode, CreateMachineRateVersion, DiagnosticSeverity, MAX_RATE_VERSIONS, MachineRateInput, MachineRateOutcome, MachineRateSensitivity,
    MachineRateSensitivityRequest, MachineRateVersion, MachineRateVersionList, MachineRateVersionPublished, MachineRateVersionView,
    PublishMachineRateVersion, RateDiagnosticCode, RateScenario, RateVersion, RateVersionState, ReferenceChange, SharedRecordKind, Unit,
};
use solidbom_rates::{CALCULATOR_VERSION, calculate, sensitivity, validate_for_publication};
use sqlx::Row;
use sqlx::postgres::PgConnection;
use uuid::Uuid;

use crate::Result;
use crate::store::{
    RFC3339_UTC, Store, StoreError, append_change, i64_version, lock_logical_record, lock_request, next_version, parse_optional_decimal,
    payload_hash, price_summary, record_reply, recorded_reply,
};

/// The column list every machine rate version read shares. `created_at` and `published_at` are
/// formatted once by the service, so a client never parses a locale timestamp.
const VERSION_COLUMNS: &str = "machine_rate_version_id, version, supersedes, state, input::text AS input, outcome_snapshot::text AS outcome_snapshot, \
     calculator_version, published_rate_version_id, created_by, \
     to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS created_at, \
     to_char(published_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"') AS published_at";

/// The SB-09 provenance of a published machine rate: the machine rate version it came from, the
/// calculator version and the fully absorbed denominator. It stays below the reference bound.
fn publication_source_reference(machine_rate_version_id: Uuid) -> String {
    format!("machine rate version {machine_rate_version_id} ({CALCULATOR_VERSION}); fully absorbed cost per ACTUAL_RUNNING_HOUR")
}

impl Store {
    /// Create the next draft version of one (machine, financial year, scenario) chain.
    ///
    /// `expected_version` is the chain's current latest version, `0` for a new chain. An identical
    /// retry of one request id returns its recorded view before the version check, so a committed
    /// create whose reply was lost is safe to retry; a stale expectation or a changed payload under
    /// the same request id is a conflict. The draft outcome is calculated in the recommended mode
    /// and may itself be invalid: storing the incomplete input is the point, publishing it is not.
    pub async fn create_machine_rate_version(&self, request: &CreateMachineRateVersion, created_by: &str) -> Result<MachineRateVersionView> {
        request.validate().map_err(invalid)?;
        check_created_by(created_by)?;
        let input = request.input.clone();
        let machine_id = input.machine.machine_id;
        let financial_year = input.financial_year;
        let scenario = input.scenario;
        let operation = "create_machine_rate_version";
        let payload = payload_hash(operation, request)?;
        let mut tx = self.pool().begin().await?;
        lock_request(&mut tx, &request.request_id).await?;
        if let Some(view) = recorded_reply::<MachineRateVersionView>(&mut tx, &request.request_id, &payload).await? {
            tx.commit().await?;
            return Ok(view);
        }
        lock_machine_chain(&mut tx, machine_id, financial_year, scenario).await?;
        let latest = latest_machine_rate_version(&mut tx, machine_id, financial_year, scenario).await?;
        let (version, supersedes) = next_version("machine rate version", request.expected_version, latest)?;
        let machine_rate_version_id = Uuid::new_v4();
        let outcome = calculate(&input, CalculationMode::Recommended);
        outcome.validate().map_err(StoreError::codec)?;
        let created_at: String = sqlx::query_scalar(&format!(
            "INSERT INTO machine_rate_versions \
             (machine_rate_version_id, machine_id, financial_year, scenario, version, supersedes, state, input, request_id, created_by) \
             VALUES ($1, $2, $3, $4, $5, $6, 'draft', $7::jsonb, $8, $9) RETURNING to_char(created_at AT TIME ZONE 'UTC', {RFC3339_UTC})"
        ))
        .bind(machine_rate_version_id)
        .bind(machine_id)
        .bind(financial_year)
        .bind(scenario.as_str())
        .bind(i64_version(version)?)
        .bind(supersedes)
        .bind(serde_json::to_string(&input).map_err(StoreError::codec)?)
        .bind(&request.request_id)
        .bind(created_by)
        .fetch_one(&mut *tx)
        .await?;
        let view = MachineRateVersionView {
            version: MachineRateVersion {
                machine_rate_version_id,
                version,
                supersedes,
                state: RateVersionState::Draft,
                input,
                published_rate_version_id: None,
                calculator_version: None,
                created_at: Some(created_at),
                published_at: None,
                created_by: Some(created_by.to_string()),
            },
            outcome,
        };
        view.validate().map_err(StoreError::codec)?;
        record_reply(&mut tx, &request.request_id, operation, &payload, &view).await?;
        tx.commit().await?;
        Ok(view)
    }

    /// The latest version of every machine across all its chains, ordered by machine id and bounded
    /// by the contract. "Latest" is the newest recorded version, so a machine whose newest chain is
    /// another scenario or year is not hidden by an older chain.
    pub async fn machine_rate_inventory(&self) -> Result<MachineRateVersionList> {
        let limit = i64::try_from(MAX_RATE_VERSIONS).map_err(StoreError::codec)?;
        let rows = sqlx::query(&format!(
            "SELECT DISTINCT ON (machine_id) {VERSION_COLUMNS} FROM machine_rate_versions \
             ORDER BY machine_id, created_at DESC, version DESC, machine_rate_version_id DESC LIMIT $1"
        ))
        .bind(limit)
        .fetch_all(self.pool())
        .await?;
        views_from_rows(&rows)
    }

    /// One machine's rate-version chains, newest first, with the optional financial-year and
    /// scenario filters. A machine with no version is an empty list, not a missing machine: this
    /// route lists versions, it does not own a machine master record.
    pub async fn machine_rate_versions_of(
        &self,
        machine_id: Uuid,
        financial_year: Option<i32>,
        scenario: Option<RateScenario>,
    ) -> Result<MachineRateVersionList> {
        let limit = i64::try_from(MAX_RATE_VERSIONS).map_err(StoreError::codec)?;
        let rows = sqlx::query(&format!(
            "SELECT {VERSION_COLUMNS} FROM machine_rate_versions \
             WHERE machine_id = $1 \
               AND ($2::int IS NULL OR coalesce(financial_year, 0) = $2) \
               AND ($3::text IS NULL OR scenario = $3) \
             ORDER BY created_at DESC, version DESC, machine_rate_version_id DESC LIMIT $4"
        ))
        .bind(machine_id)
        .bind(financial_year)
        .bind(scenario.map(RateScenario::as_str))
        .bind(limit)
        .fetch_all(self.pool())
        .await?;
        views_from_rows(&rows)
    }

    /// One machine rate version. A draft is calculated fresh with the current engine; a published
    /// version answers from its immutable approval snapshot. Anything else is not found.
    pub async fn machine_rate_version(&self, machine_rate_version_id: Uuid) -> Result<MachineRateVersionView> {
        let mut connection = self.pool().acquire().await?;
        let stored = load_machine_rate_version(&mut connection, machine_rate_version_id).await?;
        view_from_stored(stored)
    }

    /// Calculate alternative loadings from one stored input snapshot without writing anything: no
    /// request row, no version update and no SB-09 reference change. The engine copies the input and
    /// changes only `loading`, so the stored draft or published version stays byte-identical.
    pub async fn machine_rate_sensitivity(
        &self,
        machine_rate_version_id: Uuid,
        request: &MachineRateSensitivityRequest,
    ) -> Result<MachineRateSensitivity> {
        request.validate().map_err(invalid)?;
        let mut connection = self.pool().acquire().await?;
        let stored = load_machine_rate_version(&mut connection, machine_rate_version_id).await?;
        // Validate the stored approval record even though the scenarios are recalculated: a corrupt
        // published snapshot stays an internal failure instead of an apparently usable answer.
        outcome_from_stored(&stored)?;
        let result = sensitivity(&stored.input, &request.loadings, CalculationMode::Recommended).map_err(invalid)?;
        result.validate().map_err(StoreError::codec)?;
        Ok(result)
    }

    /// Publish one draft as an immutable SB-09 cost-center production rate.
    ///
    /// The transaction loads the version `FOR UPDATE`, refuses a published version or a stale
    /// `expected_version` as a conflict, recalculates the recommended outcome and refuses an
    /// invalid, zero-running-hour or unidentified outcome without writing anything. It then locks
    /// the cost-center chain, appends the next `rate_versions` row and its `cost_center`
    /// reference-change fact, marks this version published with the approved outcome snapshot and
    /// calculator version, and records the reply for an identical retry.
    pub async fn publish_machine_rate_version(
        &self,
        machine_rate_version_id: Uuid,
        request: &PublishMachineRateVersion,
        changed_by: &str,
    ) -> Result<MachineRateVersionPublished> {
        request.validate().map_err(invalid)?;
        check_created_by(changed_by)?;
        let operation = "publish_machine_rate_version";
        let payload = payload_hash(operation, &(machine_rate_version_id, request))?;
        let mut tx = self.pool().begin().await?;
        lock_request(&mut tx, &request.request_id).await?;
        if let Some(published) = recorded_reply::<MachineRateVersionPublished>(&mut tx, &request.request_id, &payload).await? {
            tx.commit().await?;
            return Ok(published);
        }
        let stored = load_machine_rate_version_for_update(&mut tx, machine_rate_version_id).await?;
        if stored.state != RateVersionState::Draft {
            return Err(StoreError::Conflict("the machine rate version is already published".into()));
        }
        if request.expected_version != stored.version {
            return Err(StoreError::Conflict(format!("the machine rate version is at version {}, not {}", stored.version, request.expected_version)));
        }
        let input = stored.input.clone();
        let outcome = calculate(&input, CalculationMode::Recommended);
        outcome.validate().map_err(StoreError::codec)?;
        let blocking = blocking_diagnostics(&input, &outcome);
        if !blocking.is_empty() {
            let codes: Vec<&str> = blocking.iter().map(|code| diagnostic_code_name(*code)).collect();
            return Err(StoreError::Invalid(format!("the machine rate version cannot be published: {}", codes.join(", "))));
        }
        let Some(total_rate) = outcome.result.as_ref().and_then(|result| result.total_machine_hourly_rate) else {
            // `is_publishable` already guarantees the rate; this is the defensive internal answer.
            return Err(StoreError::Codec("a publishable machine rate outcome carries its total rate".into()));
        };
        // Lock the SB-09 logical cost center before reading its latest version, exactly like the
        // ordinary rate publish, so two publications cannot claim one version number.
        lock_logical_record(&mut tx, "cost_center", input.machine.machine_id).await?;
        let (published_rate, change) = append_cost_center_rate(
            &mut tx,
            input.machine.machine_id,
            &input.machine.name,
            total_rate,
            publication_source_reference(machine_rate_version_id),
            changed_by,
        )
        .await?;
        let outcome_snapshot = serde_json::to_string(&outcome).map_err(StoreError::codec)?;
        let published_at: String = sqlx::query_scalar(&format!(
            "UPDATE machine_rate_versions \
             SET state = 'published', outcome_snapshot = $2::jsonb, calculator_version = $3, published_rate_version_id = $4, published_at = now() \
             WHERE machine_rate_version_id = $1 RETURNING to_char(published_at AT TIME ZONE 'UTC', {RFC3339_UTC})"
        ))
        .bind(machine_rate_version_id)
        .bind(outcome_snapshot)
        .bind(CALCULATOR_VERSION)
        .bind(published_rate.rate_version_id)
        .fetch_one(&mut *tx)
        .await?;
        let published = MachineRateVersionPublished {
            version: MachineRateVersion {
                machine_rate_version_id,
                version: stored.version,
                supersedes: stored.supersedes,
                state: RateVersionState::Published,
                input,
                published_rate_version_id: Some(published_rate.rate_version_id),
                calculator_version: Some(CALCULATOR_VERSION.to_string()),
                created_at: stored.created_at,
                published_at: Some(published_at),
                created_by: stored.created_by,
            },
            outcome,
            published_rate,
            change,
        };
        published.validate().map_err(StoreError::codec)?;
        record_reply(&mut tx, &request.request_id, operation, &payload, &published).await?;
        tx.commit().await?;
        Ok(published)
    }
}

/// One stored machine rate version with its immutable input snapshot and, once published, its
/// approval snapshot. Input and snapshot stay JSON text until the caller needs a typed value, so a
/// corrupt stored value becomes a codec failure instead of a reconstruction.
struct StoredMachineRateVersion {
    machine_rate_version_id: Uuid,
    version: u64,
    supersedes: Option<Uuid>,
    state: RateVersionState,
    input: MachineRateInput,
    outcome_snapshot: Option<String>,
    calculator_version: Option<String>,
    published_rate_version_id: Option<Uuid>,
    created_at: Option<String>,
    published_at: Option<String>,
    created_by: Option<String>,
}

impl StoredMachineRateVersion {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self> {
        let state = match row.try_get::<String, _>("state")?.as_str() {
            "draft" => RateVersionState::Draft,
            "published" => RateVersionState::Published,
            other => return Err(StoreError::Codec(format!("the stored machine rate state `{other}` is unknown"))),
        };
        let input_text: String = row.try_get("input")?;
        let input: MachineRateInput = serde_json::from_str(&input_text).map_err(StoreError::codec)?;
        input.validate().map_err(StoreError::codec)?;
        Ok(Self {
            machine_rate_version_id: row.try_get("machine_rate_version_id")?,
            version: u64::try_from(row.try_get::<i64, _>("version")?).map_err(StoreError::codec)?,
            supersedes: row.try_get("supersedes")?,
            state,
            input,
            outcome_snapshot: row.try_get("outcome_snapshot")?,
            calculator_version: row.try_get("calculator_version")?,
            published_rate_version_id: row.try_get("published_rate_version_id")?,
            created_at: row.try_get("created_at")?,
            published_at: row.try_get("published_at")?,
            created_by: row.try_get("created_by")?,
        })
    }

    fn into_version(self) -> MachineRateVersion {
        MachineRateVersion {
            machine_rate_version_id: self.machine_rate_version_id,
            version: self.version,
            supersedes: self.supersedes,
            state: self.state,
            input: self.input,
            published_rate_version_id: self.published_rate_version_id,
            calculator_version: self.calculator_version,
            created_at: self.created_at,
            published_at: self.published_at,
            created_by: self.created_by,
        }
    }
}

/// The outcome a read answers with: a draft is calculated fresh, a published version is read from
/// its stored approval snapshot. Both are validated against the contract before they leave.
fn outcome_from_stored(stored: &StoredMachineRateVersion) -> Result<MachineRateOutcome> {
    let outcome = match stored.state {
        RateVersionState::Draft => calculate(&stored.input, CalculationMode::Recommended),
        RateVersionState::Published => {
            let snapshot = stored
                .outcome_snapshot
                .as_ref()
                .ok_or_else(|| StoreError::Codec("a published machine rate version has no outcome snapshot".into()))?;
            serde_json::from_str::<MachineRateOutcome>(snapshot)
                .map_err(|error| StoreError::Codec(format!("the stored machine rate outcome snapshot: {error}")))?
        }
    };
    outcome.validate().map_err(|error| StoreError::Codec(format!("the stored machine rate outcome: {error}")))?;
    Ok(outcome)
}

fn view_from_stored(stored: StoredMachineRateVersion) -> Result<MachineRateVersionView> {
    let outcome = outcome_from_stored(&stored)?;
    let view = MachineRateVersionView { version: stored.into_version(), outcome };
    view.validate().map_err(|error| StoreError::Codec(format!("the machine rate version view: {error}")))?;
    Ok(view)
}

fn views_from_rows(rows: &[sqlx::postgres::PgRow]) -> Result<MachineRateVersionList> {
    let mut versions = Vec::with_capacity(rows.len());
    for row in rows {
        versions.push(view_from_stored(StoredMachineRateVersion::from_row(row)?)?);
    }
    let list = MachineRateVersionList { versions };
    list.validate().map_err(|error| StoreError::Codec(format!("the machine rate version list: {error}")))?;
    Ok(list)
}

async fn load_machine_rate_version(connection: &mut PgConnection, machine_rate_version_id: Uuid) -> Result<StoredMachineRateVersion> {
    let row = sqlx::query(&format!("SELECT {VERSION_COLUMNS} FROM machine_rate_versions WHERE machine_rate_version_id = $1"))
        .bind(machine_rate_version_id)
        .fetch_optional(connection)
        .await?
        .ok_or(StoreError::NotFound)?;
    StoredMachineRateVersion::from_row(&row)
}

async fn load_machine_rate_version_for_update(tx: &mut PgConnection, machine_rate_version_id: Uuid) -> Result<StoredMachineRateVersion> {
    let row = sqlx::query(&format!("SELECT {VERSION_COLUMNS} FROM machine_rate_versions WHERE machine_rate_version_id = $1 FOR UPDATE"))
        .bind(machine_rate_version_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(StoreError::NotFound)?;
    StoredMachineRateVersion::from_row(&row)
}

/// The latest version of one exact chain, used for the optimistic-concurrency check. A missing
/// financial year is the incomplete-draft chain, which the unique index coalesces to year 0.
async fn latest_machine_rate_version(
    tx: &mut PgConnection,
    machine_id: Uuid,
    financial_year: Option<i32>,
    scenario: RateScenario,
) -> Result<Option<(Uuid, u64)>> {
    let year = financial_year.unwrap_or(0);
    let row = sqlx::query(
        "SELECT machine_rate_version_id, version FROM machine_rate_versions \
         WHERE machine_id = $1 AND coalesce(financial_year, 0) = $2 AND scenario = $3 ORDER BY version DESC LIMIT 1",
    )
    .bind(machine_id)
    .bind(year)
    .bind(scenario.as_str())
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else { return Ok(None) };
    Ok(Some((row.try_get("machine_rate_version_id")?, u64::try_from(row.try_get::<i64, _>("version")?).map_err(StoreError::codec)?)))
}

/// Serialise creates of one chain so two writers cannot append the same next version number.
async fn lock_machine_chain(tx: &mut PgConnection, machine_id: Uuid, financial_year: Option<i32>, scenario: RateScenario) -> Result<()> {
    let year = financial_year.map_or_else(|| "none".to_string(), |year| year.to_string());
    let key = format!("machine_rate_chain:{machine_id}:{year}:{}", scenario.as_str());
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))").bind(key).execute(&mut *tx).await?;
    Ok(())
}

/// Why a draft cannot become an approved production rate: the recommended outcome must be
/// publishable and the identity must carry a stable machine id and a real cost-center code. The
/// diagnostic codes are named in the error message, including the zero-running-hour case that has
/// no error diagnostic of its own.
fn blocking_diagnostics(input: &MachineRateInput, outcome: &MachineRateOutcome) -> Vec<RateDiagnosticCode> {
    let mut blocking: Vec<RateDiagnosticCode> = Vec::new();
    for diagnostic in &outcome.diagnostics {
        if diagnostic.severity == DiagnosticSeverity::Error && !blocking.contains(&diagnostic.code) {
            blocking.push(diagnostic.code);
        }
    }
    for diagnostic in validate_for_publication(input) {
        if diagnostic.severity == DiagnosticSeverity::Error && !blocking.contains(&diagnostic.code) {
            blocking.push(diagnostic.code);
        }
    }
    if !outcome.is_publishable() && blocking.is_empty() {
        blocking.push(RateDiagnosticCode::ZeroRunHours);
    }
    blocking
}

/// The stable wire name of a diagnostic code, so a client-safe message can name the blocking codes
/// without a second hand-written table.
fn diagnostic_code_name(code: RateDiagnosticCode) -> &'static str {
    match code {
        RateDiagnosticCode::MissingFinancialYear => "missing_financial_year",
        RateDiagnosticCode::MissingAcquisitionYear => "missing_acquisition_year",
        RateDiagnosticCode::FinancialYearBeforeAcquisitionYear => "financial_year_before_acquisition_year",
        RateDiagnosticCode::InvalidYear => "invalid_year",
        RateDiagnosticCode::MissingInput => "missing_input",
        RateDiagnosticCode::NegativeInput => "negative_input",
        RateDiagnosticCode::InvalidCapacity => "invalid_capacity",
        RateDiagnosticCode::InvalidLoading => "invalid_loading",
        RateDiagnosticCode::InvalidAvailability => "invalid_availability",
        RateDiagnosticCode::InvalidUsefulLife => "invalid_useful_life",
        RateDiagnosticCode::InvalidRemainingLife => "invalid_remaining_life",
        RateDiagnosticCode::RemainingLifeExceedsUsefulLife => "remaining_life_exceeds_useful_life",
        RateDiagnosticCode::ResidualExceedsInstalledCapex => "residual_exceeds_installed_capex",
        RateDiagnosticCode::InvalidPowerFactor => "invalid_power_factor",
        RateDiagnosticCode::InvalidBurden => "invalid_burden",
        RateDiagnosticCode::ArithmeticOverflow => "arithmetic_overflow",
        RateDiagnosticCode::ZeroRunHours => "zero_run_hours",
        RateDiagnosticCode::BlankTreatedAsZero => "blank_treated_as_zero",
        RateDiagnosticCode::ResidualDefaultedToTwentyPercent => "residual_defaulted_to_twenty_percent",
        RateDiagnosticCode::MissingMachineCode => "missing_machine_code",
        RateDiagnosticCode::MissingMachineIdentity => "missing_machine_identity",
        RateDiagnosticCode::ResidualEqualsInstalledCapex => "residual_equals_installed_capex",
        RateDiagnosticCode::PowerWithoutTariff => "power_without_tariff",
        RateDiagnosticCode::TariffWithoutPower => "tariff_without_power",
        RateDiagnosticCode::UtilityConsumptionWithoutTariff => "utility_consumption_without_tariff",
        RateDiagnosticCode::UtilityTariffWithoutConsumption => "utility_tariff_without_consumption",
        RateDiagnosticCode::CommittedAndDedicatedLabor => "committed_and_dedicated_labor",
        RateDiagnosticCode::PremisesAllocationUnconfirmed => "premises_allocation_unconfirmed",
        RateDiagnosticCode::CompressedAirBoundaryUnconfirmed => "compressed_air_boundary_unconfirmed",
        RateDiagnosticCode::CoolingWaterBasisUnconfirmed => "cooling_water_basis_unconfirmed",
        RateDiagnosticCode::WorkbookInputsFlagged => "workbook_inputs_flagged",
    }
}

/// The latest published SB-09 rate version of one cost center, as the appending transaction needs
/// it: the version id and number to supersede and the previous rate for the change summary.
async fn latest_cost_center_rate(tx: &mut PgConnection, cost_center_id: Uuid) -> Result<Option<(Uuid, u64, Option<rust_decimal::Decimal>)>> {
    let row = sqlx::query(
        "SELECT rate_version_id, version, rate_per_hour::text AS rate FROM rate_versions WHERE cost_center_id = $1 ORDER BY version DESC LIMIT 1",
    )
    .bind(cost_center_id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else { return Ok(None) };
    Ok(Some((
        row.try_get("rate_version_id")?,
        u64::try_from(row.try_get::<i64, _>("version")?).map_err(StoreError::codec)?,
        parse_optional_decimal(row.try_get("rate")?, "rate")?,
    )))
}

/// Append one immutable SB-09 cost-center rate version and its reference-change fact, exactly like
/// the ordinary administrator publish: `production` purpose, `cny_h`, `CNY`, the machine's name as
/// the display name and the machine rate provenance as the source reference. The caller holds the
/// cost-center chain lock.
async fn append_cost_center_rate(
    tx: &mut PgConnection,
    cost_center_id: Uuid,
    name: &str,
    rate_per_hour: rust_decimal::Decimal,
    source_reference: String,
    changed_by: &str,
) -> Result<(RateVersion, ReferenceChange)> {
    let latest = latest_cost_center_rate(&mut *tx, cost_center_id).await?;
    let expected = latest.as_ref().map_or(0, |(_, version, _)| *version);
    let (version, supersedes) = next_version("cost center", expected, latest.as_ref().map(|(id, version, _)| (*id, *version)))?;
    let rate_version_id = Uuid::new_v4();
    let created_at: String = sqlx::query_scalar(&format!(
        "INSERT INTO rate_versions \
         (rate_version_id, cost_center_id, version, supersedes, name, purpose, rate_per_hour, rate_unit, currency, source_reference) \
         VALUES ($1, $2, $3, $4, $5, 'production', $6::numeric, 'cny_h', 'CNY', $7) \
         RETURNING to_char(created_at AT TIME ZONE 'UTC', {RFC3339_UTC})"
    ))
    .bind(rate_version_id)
    .bind(cost_center_id)
    .bind(i64_version(version)?)
    .bind(supersedes)
    .bind(name)
    .bind(rate_per_hour.to_string())
    .bind(&source_reference)
    .fetch_one(&mut *tx)
    .await?;
    let summary = price_summary(latest.as_ref().and_then(|(_, _, rate)| *rate), Some(rate_per_hour), "CNY", "h");
    let change = append_change(tx, SharedRecordKind::CostCenter, cost_center_id, rate_version_id, version, supersedes, summary, changed_by).await?;
    let rate = RateVersion {
        rate_version_id,
        cost_center_id,
        version,
        supersedes,
        name: name.to_string(),
        purpose: solidbom_contract::CostCenterPurpose::Production,
        rate_per_hour: Some(rate_per_hour),
        rate_unit: Unit::CurrencyPerHour,
        currency: "CNY".to_string(),
        source_reference: Some(source_reference),
        created_at: Some(created_at),
    };
    Ok((rate, change))
}

fn check_created_by(changed_by: &str) -> Result<()> {
    if changed_by.trim().is_empty() {
        return Err(StoreError::Invalid("a machine rate write needs the identity that made it".into()));
    }
    Ok(())
}

fn invalid(error: solidbom_contract::ContractError) -> StoreError {
    StoreError::Invalid(error.to_string())
}
