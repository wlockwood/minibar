//! Deterministic comparison of captured Recoil fixtures with simulator output.
//!
//! This module deliberately stops at the interchange and comparison boundary.
//! Capturing observations from Recoil and producing simulator observations are
//! responsibilities of their respective adapters.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub const CALIBRATION_FIXTURE_VERSION: u16 = 1;

fn fixture_version() -> u16 {
    CALIBRATION_FIXTURE_VERSION
}

fn required_by_default() -> bool {
    true
}

fn minimum_sample_count() -> u64 {
    1
}

/// Provenance retained with a captured Recoil fixture.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FixtureMetadata {
    pub fixture_id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub source_engine: String,
    pub source_engine_version: String,
    pub game_name: String,
    pub game_version: String,
    pub map_name: String,
    #[serde(default)]
    pub captured_at_utc: Option<String>,
    #[serde(default)]
    pub seed: Option<u64>,
    pub tick_rate_hz: f64,
    #[serde(default)]
    pub tags: BTreeMap<String, String>,
}

/// A complete, JSON-importable calibration fixture.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CalibrationFixture {
    #[serde(default = "fixture_version")]
    pub format_version: u16,
    pub metadata: FixtureMetadata,
    #[serde(default)]
    pub checks: Vec<ExpectedCheck>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ExpectedCheck {
    /// Stable identifier unique within the fixture.
    pub id: String,
    /// Stable simulator capability/mechanic name, such as `weapon.burst`.
    pub mechanic: String,
    #[serde(default = "required_by_default")]
    pub required: bool,
    #[serde(flatten)]
    pub expectation: CheckExpectation,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CheckExpectation {
    ExactEvents { events: Vec<EventObservation> },
    ExactSnapshots { snapshots: Vec<SnapshotObservation> },
    Statistic(StatisticalExpectation),
}

/// A normalized event. Adapters can retain mechanic-specific data in `fields`.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EventObservation {
    pub tick: u64,
    #[serde(default)]
    pub sequence: u32,
    pub event: String,
    #[serde(default)]
    pub subject: Option<u64>,
    #[serde(default)]
    pub object: Option<u64>,
    #[serde(default)]
    pub fields: BTreeMap<String, ObservationValue>,
}

/// A normalized state snapshot. Stable string keys avoid coupling fixtures to
/// an engine-specific entity representation.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SnapshotObservation {
    pub tick: u64,
    #[serde(default)]
    pub values: BTreeMap<String, ObservationValue>,
    #[serde(default)]
    pub entities: BTreeMap<String, BTreeMap<String, ObservationValue>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(untagged)]
pub enum ObservationValue {
    Bool(bool),
    Integer(i64),
    Number(f64),
    Text(String),
    Null,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StatisticalExpectation {
    pub metric: String,
    pub expected_mean: f64,
    #[serde(default)]
    pub absolute_tolerance: f64,
    #[serde(default)]
    pub relative_tolerance: f64,
    #[serde(default = "minimum_sample_count")]
    pub minimum_samples: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StatisticalObservation {
    pub metric: String,
    pub samples: u64,
    pub mean: f64,
    #[serde(default)]
    pub standard_deviation: Option<f64>,
}

/// Output produced by a simulator-side fixture runner.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MeasuredObservations {
    pub fixture_id: String,
    pub simulator_version: String,
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub checks: Vec<MeasuredCheck>,
    #[serde(default)]
    pub notices: Vec<CalibrationNotice>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MeasuredCheck {
    pub id: String,
    #[serde(flatten)]
    pub observation: CheckObservation,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CheckObservation {
    ExactEvents { events: Vec<EventObservation> },
    ExactSnapshots { snapshots: Vec<SnapshotObservation> },
    Statistic(StatisticalObservation),
}

/// A limitation reported by the simulator-side fixture runner.
#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct CalibrationNotice {
    pub mechanic: String,
    pub kind: NoticeKind,
    /// Stable adapter- or simulator-defined identifier.
    pub code: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    Unsupported,
    Approximation,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CalibrationReport {
    pub fixture_id: String,
    pub simulator_version: String,
    pub passed: bool,
    pub mechanics: Vec<MechanicReport>,
    pub checks: Vec<CheckReport>,
    pub warnings: Vec<CalibrationWarning>,
    pub diagnostics: Vec<CalibrationDiagnostic>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MechanicReport {
    pub mechanic: String,
    /// True only with at least one required check, every required check passing,
    /// valid fixture/run identity, and no explicit unsupported notice.
    pub supported: bool,
    pub required_checks: usize,
    pub passed_required_checks: usize,
    pub optional_checks: usize,
    pub passed_optional_checks: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CheckReport {
    pub id: String,
    pub mechanic: String,
    pub required: bool,
    pub passed: bool,
    pub diagnostics: Vec<CalibrationDiagnostic>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct CalibrationWarning {
    pub code: WarningCode,
    pub mechanic: String,
    pub detail_code: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningCode {
    UnsupportedMechanic,
    Approximation,
    NoRequiredChecks,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CalibrationDiagnostic {
    pub code: FailureCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub observed: Option<Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode {
    UnsupportedFixtureVersion,
    FixtureIdMismatch,
    DuplicateExpectedCheck,
    DuplicateMeasuredCheck,
    MissingObservation,
    ObservationKindMismatch,
    ExactEventsMismatch,
    ExactSnapshotsMismatch,
    InvalidStatisticalExpectation,
    StatisticMetricMismatch,
    InsufficientSamples,
    NonFiniteStatistic,
    StatisticOutsideTolerance,
    UnexpectedObservation,
}

impl CalibrationDiagnostic {
    fn new(code: FailureCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            expected: None,
            observed: None,
        }
    }

    fn values(mut self, expected: Value, observed: Value) -> Self {
        self.expected = Some(expected);
        self.observed = Some(observed);
        self
    }
}

/// Compare normalized simulator observations against a captured fixture.
///
/// Output order is canonical and independent of input vector ordering:
/// checks are ordered by `(mechanic, id)`, mechanics by name, warnings by their
/// derived ordering, and top-level diagnostics by failure code then message.
pub fn compare_calibration(
    fixture: &CalibrationFixture,
    measured: &MeasuredObservations,
) -> CalibrationReport {
    let mut global_diagnostics = Vec::new();
    if fixture.format_version != CALIBRATION_FIXTURE_VERSION {
        global_diagnostics.push(
            CalibrationDiagnostic::new(
                FailureCode::UnsupportedFixtureVersion,
                "fixture format version is not supported",
            )
            .values(
                Value::from(CALIBRATION_FIXTURE_VERSION),
                Value::from(fixture.format_version),
            ),
        );
    }
    if fixture.metadata.fixture_id != measured.fixture_id {
        global_diagnostics.push(
            CalibrationDiagnostic::new(
                FailureCode::FixtureIdMismatch,
                "measured observations belong to a different fixture",
            )
            .values(
                Value::from(fixture.metadata.fixture_id.clone()),
                Value::from(measured.fixture_id.clone()),
            ),
        );
    }

    let expected_counts = counts(fixture.checks.iter().map(|check| check.id.as_str()));
    let mut measured_by_id: BTreeMap<&str, Vec<&MeasuredCheck>> = BTreeMap::new();
    for observation in &measured.checks {
        measured_by_id
            .entry(observation.id.as_str())
            .or_default()
            .push(observation);
    }

    let mut ordered_expected: Vec<_> = fixture.checks.iter().collect();
    ordered_expected
        .sort_by(|left, right| (&left.mechanic, &left.id).cmp(&(&right.mechanic, &right.id)));

    let mut checks = Vec::with_capacity(ordered_expected.len());
    for expected in ordered_expected {
        let mut diagnostics = Vec::new();
        if expected_counts[expected.id.as_str()] > 1 {
            diagnostics.push(CalibrationDiagnostic::new(
                FailureCode::DuplicateExpectedCheck,
                format!("fixture check id `{}` is not unique", expected.id),
            ));
        }
        match measured_by_id.get(expected.id.as_str()) {
            None => diagnostics.push(CalibrationDiagnostic::new(
                FailureCode::MissingObservation,
                format!("no observation was supplied for check `{}`", expected.id),
            )),
            Some(observations) if observations.len() > 1 => {
                diagnostics.push(CalibrationDiagnostic::new(
                    FailureCode::DuplicateMeasuredCheck,
                    format!("observation check id `{}` is not unique", expected.id),
                ))
            }
            Some(observations) => compare_check(
                &expected.expectation,
                &observations[0].observation,
                &mut diagnostics,
            ),
        }
        diagnostics
            .sort_by(|left, right| (left.code, &left.message).cmp(&(right.code, &right.message)));
        checks.push(CheckReport {
            id: expected.id.clone(),
            mechanic: expected.mechanic.clone(),
            required: expected.required,
            passed: diagnostics.is_empty(),
            diagnostics,
        });
    }

    for (id, observations) in &measured_by_id {
        if !expected_counts.contains_key(id) {
            global_diagnostics.push(CalibrationDiagnostic::new(
                FailureCode::UnexpectedObservation,
                format!(
                    "{} observation(s) supplied for unknown check `{id}`",
                    observations.len()
                ),
            ));
        }
    }

    let mut warnings: Vec<_> = measured
        .notices
        .iter()
        .map(|notice| CalibrationWarning {
            code: match notice.kind {
                NoticeKind::Unsupported => WarningCode::UnsupportedMechanic,
                NoticeKind::Approximation => WarningCode::Approximation,
            },
            mechanic: notice.mechanic.clone(),
            detail_code: notice.code.clone(),
            message: notice.message.clone(),
        })
        .collect();

    let mut mechanic_names: BTreeSet<&str> = fixture
        .checks
        .iter()
        .map(|check| check.mechanic.as_str())
        .collect();
    mechanic_names.extend(
        measured
            .notices
            .iter()
            .map(|notice| notice.mechanic.as_str()),
    );

    let identity_valid = global_diagnostics.iter().all(|diagnostic| {
        !matches!(
            diagnostic.code,
            FailureCode::UnsupportedFixtureVersion | FailureCode::FixtureIdMismatch
        )
    });
    let mut mechanics = Vec::with_capacity(mechanic_names.len());
    for mechanic in mechanic_names {
        let mechanic_checks: Vec<_> = checks
            .iter()
            .filter(|check| check.mechanic == mechanic)
            .collect();
        let required_checks = mechanic_checks
            .iter()
            .filter(|check| check.required)
            .count();
        let passed_required_checks = mechanic_checks
            .iter()
            .filter(|check| check.required && check.passed)
            .count();
        let optional_checks = mechanic_checks
            .iter()
            .filter(|check| !check.required)
            .count();
        let passed_optional_checks = mechanic_checks
            .iter()
            .filter(|check| !check.required && check.passed)
            .count();
        let explicitly_unsupported = measured
            .notices
            .iter()
            .any(|notice| notice.mechanic == mechanic && notice.kind == NoticeKind::Unsupported);
        if required_checks == 0 {
            warnings.push(CalibrationWarning {
                code: WarningCode::NoRequiredChecks,
                mechanic: mechanic.to_owned(),
                detail_code: "no_required_checks".to_owned(),
                message: "mechanic has no required calibration checks".to_owned(),
            });
        }
        mechanics.push(MechanicReport {
            mechanic: mechanic.to_owned(),
            supported: identity_valid
                && required_checks > 0
                && required_checks == passed_required_checks
                && !explicitly_unsupported,
            required_checks,
            passed_required_checks,
            optional_checks,
            passed_optional_checks,
        });
    }

    warnings.sort();
    warnings.dedup();
    global_diagnostics
        .sort_by(|left, right| (left.code, &left.message).cmp(&(right.code, &right.message)));
    let passed = global_diagnostics.is_empty()
        && checks
            .iter()
            .filter(|check| check.required)
            .all(|check| check.passed)
        && mechanics.iter().all(|mechanic| mechanic.supported);

    CalibrationReport {
        fixture_id: fixture.metadata.fixture_id.clone(),
        simulator_version: measured.simulator_version.clone(),
        passed,
        mechanics,
        checks,
        warnings,
        diagnostics: global_diagnostics,
    }
}

fn counts<'a>(ids: impl Iterator<Item = &'a str>) -> BTreeMap<&'a str, usize> {
    let mut counts = BTreeMap::new();
    for id in ids {
        *counts.entry(id).or_insert(0) += 1;
    }
    counts
}

fn compare_check(
    expected: &CheckExpectation,
    observed: &CheckObservation,
    diagnostics: &mut Vec<CalibrationDiagnostic>,
) {
    match (expected, observed) {
        (
            CheckExpectation::ExactEvents { events: expected },
            CheckObservation::ExactEvents { events: observed },
        ) => {
            if expected != observed {
                diagnostics.push(
                    CalibrationDiagnostic::new(
                        FailureCode::ExactEventsMismatch,
                        "observed event sequence did not exactly match the fixture",
                    )
                    .values(json_value(expected), json_value(observed)),
                );
            }
        }
        (
            CheckExpectation::ExactSnapshots {
                snapshots: expected,
            },
            CheckObservation::ExactSnapshots {
                snapshots: observed,
            },
        ) => {
            if expected != observed {
                diagnostics.push(
                    CalibrationDiagnostic::new(
                        FailureCode::ExactSnapshotsMismatch,
                        "observed snapshots did not exactly match the fixture",
                    )
                    .values(json_value(expected), json_value(observed)),
                );
            }
        }
        (CheckExpectation::Statistic(expected), CheckObservation::Statistic(observed)) => {
            compare_statistic(expected, observed, diagnostics);
        }
        _ => diagnostics.push(CalibrationDiagnostic::new(
            FailureCode::ObservationKindMismatch,
            "observation kind does not match the fixture expectation",
        )),
    }
}

fn compare_statistic(
    expected: &StatisticalExpectation,
    observed: &StatisticalObservation,
    diagnostics: &mut Vec<CalibrationDiagnostic>,
) {
    let tolerance_valid = expected.expected_mean.is_finite()
        && expected.absolute_tolerance.is_finite()
        && expected.absolute_tolerance >= 0.0
        && expected.relative_tolerance.is_finite()
        && expected.relative_tolerance >= 0.0
        && expected.minimum_samples > 0;
    if !tolerance_valid {
        diagnostics.push(CalibrationDiagnostic::new(
            FailureCode::InvalidStatisticalExpectation,
            "statistical expectation must be finite, non-negative, and require samples",
        ));
        return;
    }
    if expected.metric != observed.metric {
        diagnostics.push(
            CalibrationDiagnostic::new(
                FailureCode::StatisticMetricMismatch,
                "observed statistic has a different metric name",
            )
            .values(
                Value::from(expected.metric.clone()),
                Value::from(observed.metric.clone()),
            ),
        );
    }
    if observed.samples < expected.minimum_samples {
        diagnostics.push(
            CalibrationDiagnostic::new(
                FailureCode::InsufficientSamples,
                "observed statistic has too few samples",
            )
            .values(
                Value::from(expected.minimum_samples),
                Value::from(observed.samples),
            ),
        );
    }
    if !observed.mean.is_finite()
        || observed
            .standard_deviation
            .is_some_and(|value| !value.is_finite() || value < 0.0)
    {
        diagnostics.push(CalibrationDiagnostic::new(
            FailureCode::NonFiniteStatistic,
            "observed statistic must contain finite, non-negative values",
        ));
        return;
    }
    let allowed_error = expected
        .absolute_tolerance
        .max(expected.relative_tolerance * expected.expected_mean.abs());
    if (observed.mean - expected.expected_mean).abs() > allowed_error {
        diagnostics.push(
            CalibrationDiagnostic::new(
                FailureCode::StatisticOutsideTolerance,
                format!("observed mean exceeds the allowed error of {allowed_error}"),
            )
            .values(
                Value::from(expected.expected_mean),
                Value::from(observed.mean),
            ),
        );
    }
}

fn json_value<T: Serialize>(value: &T) -> Value {
    serde_json::to_value(value).expect("calibration observations serialize to JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> CalibrationFixture {
        CalibrationFixture {
            format_version: CALIBRATION_FIXTURE_VERSION,
            metadata: FixtureMetadata {
                fixture_id: "burst-v1".to_owned(),
                name: "Burst cadence".to_owned(),
                description: String::new(),
                source_engine: "Recoil".to_owned(),
                source_engine_version: "fixture-build".to_owned(),
                game_name: "Beyond All Reason".to_owned(),
                game_version: "test".to_owned(),
                map_name: "Test Map".to_owned(),
                captured_at_utc: None,
                seed: Some(7),
                tick_rate_hz: 30.0,
                tags: BTreeMap::new(),
            },
            checks: vec![
                ExpectedCheck {
                    id: "impacts".to_owned(),
                    mechanic: "weapon.burst".to_owned(),
                    required: true,
                    expectation: CheckExpectation::ExactEvents {
                        events: vec![event(10, "impact")],
                    },
                },
                ExpectedCheck {
                    id: "accuracy".to_owned(),
                    mechanic: "weapon.accuracy".to_owned(),
                    required: true,
                    expectation: CheckExpectation::Statistic(StatisticalExpectation {
                        metric: "hit_fraction".to_owned(),
                        expected_mean: 0.5,
                        absolute_tolerance: 0.02,
                        relative_tolerance: 0.0,
                        minimum_samples: 1_000,
                    }),
                },
            ],
        }
    }

    fn event(tick: u64, event: &str) -> EventObservation {
        EventObservation {
            tick,
            sequence: 0,
            event: event.to_owned(),
            subject: Some(1),
            object: Some(2),
            fields: BTreeMap::new(),
        }
    }

    fn passing_observations() -> MeasuredObservations {
        MeasuredObservations {
            fixture_id: "burst-v1".to_owned(),
            simulator_version: "sim-test".to_owned(),
            seed: Some(7),
            checks: vec![
                MeasuredCheck {
                    id: "accuracy".to_owned(),
                    observation: CheckObservation::Statistic(StatisticalObservation {
                        metric: "hit_fraction".to_owned(),
                        samples: 5_000,
                        mean: 0.515,
                        standard_deviation: Some(0.1),
                    }),
                },
                MeasuredCheck {
                    id: "impacts".to_owned(),
                    observation: CheckObservation::ExactEvents {
                        events: vec![event(10, "impact")],
                    },
                },
            ],
            notices: Vec::new(),
        }
    }

    #[test]
    fn json_fixture_round_trip_accepts_tagged_expectations() {
        let encoded = serde_json::to_string(&fixture()).unwrap();
        let decoded: CalibrationFixture = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, fixture());
        assert!(encoded.contains("\"kind\":\"exact_events\""));
        assert!(encoded.contains("\"kind\":\"statistic\""));
    }

    #[test]
    fn all_required_checks_must_pass_before_mechanic_is_supported() {
        let fixture = fixture();
        let mut observations = passing_observations();
        let report = compare_calibration(&fixture, &observations);
        assert!(report.passed);
        assert!(report.mechanics.iter().all(|mechanic| mechanic.supported));

        if let CheckObservation::Statistic(statistic) = &mut observations.checks[0].observation {
            statistic.mean = 0.53;
        }
        let report = compare_calibration(&fixture, &observations);
        assert!(!report.passed);
        assert!(
            !report
                .mechanics
                .iter()
                .find(|mechanic| mechanic.mechanic == "weapon.accuracy")
                .unwrap()
                .supported
        );
        assert_eq!(
            report.checks[0].diagnostics[0].code,
            FailureCode::StatisticOutsideTolerance
        );
    }

    #[test]
    fn optional_failure_does_not_hide_required_support() {
        let mut fixture = fixture();
        fixture.checks[0].required = false;
        let mut observations = passing_observations();
        observations.checks[1].observation = CheckObservation::ExactEvents { events: Vec::new() };
        let report = compare_calibration(&fixture, &observations);
        let burst = report
            .mechanics
            .iter()
            .find(|mechanic| mechanic.mechanic == "weapon.burst")
            .unwrap();
        assert!(!burst.supported, "a mechanic needs a required check");
        assert_eq!(burst.optional_checks, 1);
        assert_eq!(report.warnings[0].code, WarningCode::NoRequiredChecks);
    }

    #[test]
    fn unsupported_blocks_support_while_approximation_only_warns() {
        let fixture = fixture();
        let mut observations = passing_observations();
        observations.notices = vec![
            CalibrationNotice {
                mechanic: "weapon.burst".to_owned(),
                kind: NoticeKind::Approximation,
                code: "tick_quantized".to_owned(),
                message: "cadence is tick quantized".to_owned(),
            },
            CalibrationNotice {
                mechanic: "weapon.accuracy".to_owned(),
                kind: NoticeKind::Unsupported,
                code: "missing_spray_angle".to_owned(),
                message: "spray angle is not modeled".to_owned(),
            },
        ];
        let report = compare_calibration(&fixture, &observations);
        assert!(!report.passed);
        assert!(report.mechanics[1].supported);
        assert!(!report.mechanics[0].supported);
        assert_eq!(report.warnings.len(), 2);
        assert_eq!(report.warnings[0].code, WarningCode::UnsupportedMechanic);
        assert_eq!(report.warnings[1].code, WarningCode::Approximation);
    }

    #[test]
    fn reports_have_stable_order_despite_input_order() {
        let fixture = fixture();
        let forward = passing_observations();
        let mut reversed = forward.clone();
        reversed.checks.reverse();
        assert_eq!(
            serde_json::to_string(&compare_calibration(&fixture, &forward)).unwrap(),
            serde_json::to_string(&compare_calibration(&fixture, &reversed)).unwrap()
        );
    }

    #[test]
    fn duplicate_missing_and_unexpected_codes_are_stable() {
        let fixture = fixture();
        let mut observations = passing_observations();
        observations.checks.remove(0);
        observations.checks.push(observations.checks[0].clone());
        observations.checks.push(MeasuredCheck {
            id: "extra".to_owned(),
            observation: CheckObservation::ExactEvents { events: Vec::new() },
        });
        let report = compare_calibration(&fixture, &observations);
        assert_eq!(
            report.checks[0].diagnostics[0].code,
            FailureCode::MissingObservation
        );
        assert_eq!(
            report.checks[1].diagnostics[0].code,
            FailureCode::DuplicateMeasuredCheck
        );
        assert_eq!(
            report.diagnostics[0].code,
            FailureCode::UnexpectedObservation
        );
    }

    #[test]
    fn relative_tolerance_scales_with_expected_magnitude() {
        let mut fixture = fixture();
        fixture.checks = vec![ExpectedCheck {
            id: "travel".to_owned(),
            mechanic: "movement.speed".to_owned(),
            required: true,
            expectation: CheckExpectation::Statistic(StatisticalExpectation {
                metric: "travel_seconds".to_owned(),
                expected_mean: 10.0,
                absolute_tolerance: 0.1,
                relative_tolerance: 0.05,
                minimum_samples: 10,
            }),
        }];
        let observations = MeasuredObservations {
            fixture_id: fixture.metadata.fixture_id.clone(),
            simulator_version: "sim-test".to_owned(),
            seed: None,
            checks: vec![MeasuredCheck {
                id: "travel".to_owned(),
                observation: CheckObservation::Statistic(StatisticalObservation {
                    metric: "travel_seconds".to_owned(),
                    samples: 10,
                    mean: 10.5,
                    standard_deviation: None,
                }),
            }],
            notices: Vec::new(),
        };
        assert!(compare_calibration(&fixture, &observations).passed);
    }

    #[test]
    fn wrong_observation_kind_has_a_specific_failure_code() {
        let fixture = fixture();
        let mut observations = passing_observations();
        observations.checks[1].observation = CheckObservation::ExactSnapshots {
            snapshots: Vec::new(),
        };
        let report = compare_calibration(&fixture, &observations);
        assert_eq!(
            report.checks[1].diagnostics[0].code,
            FailureCode::ObservationKindMismatch
        );
    }
}
