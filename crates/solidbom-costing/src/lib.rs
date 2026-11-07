//! SolidBOM's direct-cost calculator: pure arithmetic over the portable v1 contract, used by the
//! service only. The desktop displays the service's answer and never reaches this crate.
//!
//! # Published contract
//!
//! Inputs are [`solidbom_contract::CalculationInputs`] and
//! [`solidbom_contract::ResolvedPartInput`] (the SB-02 brief's prose name
//! `ResolvedCalculationInput` is this published `CalculationInputs`); the answer is
//! [`solidbom_contract::CalculationOutcome`] with its part lines, `assembly_subtotal`, a separate
//! `setup_total`, named [`solidbom_contract::Issue`]s and a `ResultState`. Money and rates are
//! [`rust_decimal::Decimal`], serialized as strings. Every business value is an `Option`: `None`
//! is missing, `Some(Decimal::ZERO)` is an explicit zero.
//!
//! # Arithmetic in version `solidbom-direct-cost/1`
//!
//! ```text
//! net_mass_kg       = volume_mm3 × density_g_cm3 / 1_000_000
//! allowed_material  = net_mass_kg × material_price_per_kg × (1 + allowance_rate)
//! material_overhead = allowed_material × material_overhead_rate
//! production_cost   = (cycle_seconds / cavities) × production_rate_per_hour / 3_600
//! per_part_subtotal = allowed_material + material_overhead + production_cost
//! line_total        = per_part_subtotal × quantity
//! assembly_subtotal = Σ line_total
//! setup_total       = Σ setup_hours × setup_rate_per_hour
//! ```
//!
//! The allowance is applied once, to material only. A part's subtotal is expanded by its occurrence
//! quantity before it enters the assembly total, and the assembly total sums those unrounded line
//! totals. Setup is counted once per shared part routing and stays outside the unit direct cost.
//! Arithmetic is checked decimal arithmetic: overflow becomes an `arithmetic_overflow` issue and
//! an unavailable amount, never a panic and never an invented value.
//!
//! # Missing, zero and invalid inputs
//!
//! A missing value stays `None`, makes its affected amounts unavailable and produces a named
//! `missing_*` issue; it never becomes zero. `cycle_seconds = 0` is a valid explicit value (zero
//! production; the production rate may then be omitted), and `setup_hours = 0` is a valid explicit
//! value (the setup rate may then be omitted). A zero volume is a valid reviewed measurement; zero
//! price, allowance, overhead rate, cycle time and setup hours are valid explicit values. An
//! omitted cavity count is a single-cavity tool. Negative cycle or setup time, zero cavities, zero
//! density, zero or negative production or setup rates (a setup rate is required only when setup
//! time is positive), and negative volume, price or allowance are invalid inputs; a zero quantity
//! is invalid and withholds the quantity line total. The supported shape is one costing currency
//! (CNY; FX is outside this prototype), one entry per `part_version_id` whose quantity is the
//! expanded occurrence count, and one manufactured operation per part. An empty part list withholds
//! the assembly and setup totals rather than reporting a zero cost.
//!
//! All arithmetic is exact while the value is representable in `rust_decimal`'s 96-bit mantissa
//! (up to 28–29 significant digits). `checked_add`/`checked_mul` never wrap; an unrepresentable
//! result becomes an `arithmetic_overflow` issue and an unavailable amount. Division of a
//! non-terminating decimal is rounded by `rust_decimal` at its last representable digit; every
//! approved prototype fixture divides exactly, and this crate never applies display rounding.
//!
//! # Issues
//!
//! | Code | Field | Meaning |
//! |---|---|---|
//! | `invalid_currency` | `currency` | the project has no currency |
//! | `unsupported_currency` | `currency` | a currency other than CNY needs FX, which is outside this prototype; all amounts are unavailable |
//! | `missing_overhead_rate` / `invalid_overhead_rate` | `material_overhead_rate` | no overhead rate was entered, or it is negative |
//! | `no_parts` | `parts` | there is no reviewed part to calculate |
//! | `duplicate_part_version` | `parts[i].part_version_id` | the same part version appears twice; assembly and setup totals are withheld |
//! | `missing_volume` / `invalid_volume` | `parts[i].volume_mm3` | no reviewed volume, or a negative one |
//! | `missing_density` / `invalid_density` | `parts[i].density_g_cm3` | no selected material density, or a non-positive one |
//! | `missing_material_price` / `invalid_material_price` | `parts[i].material_price_per_kg` | the material has no price, or a negative one |
//! | `missing_allowance_rate` / `invalid_allowance_rate` | `parts[i].allowance_rate` | no allowance was entered, or a negative one |
//! | `missing_cycle_time` / `invalid_cycle_time` | `parts[i].cycle_seconds` | no cycle time, or a negative one |
//! | `invalid_cavities` | `parts[i].cavities` | the cavity count is zero; an omitted count is a single-cavity tool |
//! | `missing_production_rate` / `invalid_production_rate` | `parts[i].production_rate_per_hour` | no production rate, or a non-positive one |
//! | `missing_setup_hours` / `invalid_setup_hours` | `parts[i].setup_hours` | no setup duration, or a negative one |
//! | `missing_setup_rate` / `invalid_setup_rate` | `parts[i].setup_rate_per_hour` | setup hours are positive but no positive setup rate was selected |
//! | `invalid_quantity` | `parts[i].quantity` | a part must occur at least once |
//! | `arithmetic_overflow` | the affected line, `assembly_subtotal` or `setup_total` | the checked decimal operation exceeded the supported range |
//!
//! A part issue carries the resolved `part_version_id` in `solidbom_contract::Issue::definition_id`
//! because the resolved input has no CAD definition id; SB-03 owns mapping it to the reviewed
//! definition. An unsupported currency keeps one row per part with its identity and the standard
//! explanation lines, but every amount is `None`.
//!
//! # Scope
//!
//! Only the approved direct-cost lines are reported: net mass, allowed material, material
//! overhead, production, the per-part subtotal, the quantity line total and the separate setup
//! charge. Tooling, SG&A and other full-manufacturing categories, pricing/EBIT, annual
//! productivity, FX and shared-demand policy are outside this calculator. It performs no catalog
//! lookup and applies no company defaults.
//!
//! # Version policy
//!
//! [`calculator_version`] names the arithmetic an immutable snapshot used. Any change to the
//! formulas, rounding, units or supported scope needs a new version and a contract review within
//! SB-01's published contract; adding a named issue for an input that already had no usable result
//! does not change a valid input's arithmetic.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

pub mod calculate;

pub use calculate::{calculate, calculator_version};
