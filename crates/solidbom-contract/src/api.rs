//! The version 1 HTTP vocabulary: stable error bodies and the compact receipts a caller can keep
//! without the full answer.
//!
//! The wire routes and status codes are documented in `docs/solidbom/contracts/v1/http-api.md`;
//! this module holds the shapes both sides serialize. An error body is exactly
//! `{"error":{"code":"…","message":"…"}}` and never carries a database error, path or token.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{ContractError, Result};
use crate::types::CalculationSnapshot;

/// The HTTP API version this build speaks. Routes live under `/v1`.
/// The HTTP API version. Version 2 adds the shared-data and permission routes (SB-09): the
/// `/v1/me` and `/v1/shared` routes, publish-version routes, the `forbidden` error code and the
/// versioned reference records with optional money and explicit units.
pub const API_VERSION: u32 = 2;

/// The stable error codes the service returns.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApiErrorCode {
    Unauthorized,
    /// The identity is known but holds neither permission set the route requires.
    Forbidden,
    Invalid,
    NotFound,
    Conflict,
    Internal,
}

impl ApiErrorCode {
    /// Whether sending the same request again can succeed without a person changing anything.
    pub fn retryable(self) -> bool {
        matches!(self, Self::Internal)
    }
}

/// The detail inside an [`ApiErrorBody`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiErrorDetail {
    pub code: ApiErrorCode,
    pub message: String,
}

/// The exact JSON shape of a failed HTTP request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiErrorBody {
    pub error: ApiErrorDetail,
}

impl ApiErrorBody {
    /// Build the exact error body the service and the client share.
    pub fn new(code: ApiErrorCode, message: impl Into<String>) -> Self {
        Self { error: ApiErrorDetail { code, message: message.into() } }
    }
}

/// One immutable saved project version.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectVersionRef {
    pub project_id: Uuid,
    pub version: u64,
}

impl ProjectVersionRef {
    /// A saved version starts at 1; 0 means never saved.
    pub fn validate(&self) -> Result<()> {
        if self.version == 0 {
            return Err(ContractError::invalid("project_version.version", "a saved version starts at 1"));
        }
        Ok(())
    }
}

/// The compact identity of a recorded calculation, kept for idempotent retries and history rows.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CalculationReceipt {
    pub request_id: String,
    pub calculation_id: Uuid,
    pub project_id: Uuid,
    pub saved_version: u64,
}

impl CalculationReceipt {
    /// The receipt for `snapshot`, submitted under `request_id`.
    pub fn of(request_id: impl Into<String>, snapshot: &CalculationSnapshot) -> Self {
        Self {
            request_id: request_id.into(),
            calculation_id: snapshot.calculation_id,
            project_id: snapshot.project_id,
            saved_version: snapshot.saved_version,
        }
    }

    /// The receipt needs its request id and a saved version.
    pub fn validate(&self) -> Result<()> {
        if self.request_id.trim().is_empty() {
            return Err(ContractError::invalid("calculation.request_id", "a receipt needs its request id"));
        }
        if self.saved_version == 0 {
            return Err(ContractError::invalid("calculation.saved_version", "a calculation needs a saved version"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CalculationInputs, CalculationOutcome, ResultState};
    use rust_decimal::Decimal;

    #[test]
    fn an_error_body_has_the_exact_wire_shape() {
        let value = serde_json::to_value(ApiErrorBody::new(ApiErrorCode::Conflict, "another save won")).unwrap_or_default();
        assert_eq!(value, serde_json::json!({"error": {"code": "conflict", "message": "another save won"}}));
        assert_eq!(serde_json::from_value::<ApiErrorBody>(value).ok(), Some(ApiErrorBody::new(ApiErrorCode::Conflict, "another save won")));
    }

    #[test]
    fn only_an_internal_failure_is_retryable_without_changing_anything() {
        assert!(ApiErrorCode::Internal.retryable());
        for code in [ApiErrorCode::Unauthorized, ApiErrorCode::Forbidden, ApiErrorCode::Invalid, ApiErrorCode::NotFound, ApiErrorCode::Conflict] {
            assert!(!code.retryable());
        }
    }

    #[test]
    fn a_project_version_starts_at_one() {
        let version = ProjectVersionRef { project_id: Uuid::nil(), version: 1 };
        assert!(version.validate().is_ok());
        assert!(ProjectVersionRef { version: 0, ..version }.validate().is_err());
    }

    #[test]
    fn a_receipt_keeps_the_calculation_identity() {
        let snapshot = CalculationSnapshot {
            calculation_id: Uuid::from_u128(7),
            project_id: Uuid::from_u128(9),
            saved_version: 3,
            input_hash: "a".repeat(64),
            calculator_version: "solidbom-direct-cost/1".into(),
            inputs: CalculationInputs { currency: "CNY".into(), material_overhead_rate: Some(Decimal::ZERO), parts: Vec::new() },
            outputs: CalculationOutcome {
                state: ResultState::Incomplete,
                parts: Vec::new(),
                assembly_subtotal: None,
                setup_total: None,
                issues: Vec::new(),
            },
            issues: Vec::new(),
        };
        let receipt = CalculationReceipt::of("request-1", &snapshot);
        assert_eq!(receipt.calculation_id, Uuid::from_u128(7));
        assert_eq!(receipt.saved_version, 3);
        assert!(receipt.validate().is_ok());
        assert!(CalculationReceipt { request_id: " ".into(), ..receipt }.validate().is_err());
    }
}
