//! Small, human-readable learned policies.
//!
//! A [`Brain`] is a *council* of tiny perceptrons. Every module has a name, a
//! one-sentence responsibility, labelled inputs, and labelled outputs. A fixed,
//! hand-written formula (see [`tactical`]) combines the module outputs into a
//! command, so nothing learned is hidden in the wiring. Weight files are
//! self-describing JSON, and [`explain`] turns one into a report.
//!
//! Training is gradient-free evolution strategies ([`es`]) scored by an
//! explicitly declared objective ([`objective`]); see [`train`].

pub mod air;
pub mod baselines;
pub mod brain;
pub mod campaign;
pub mod campaign_eval;
pub mod campaign_frontier;
pub mod campaign_io;
pub mod campaign_promotion;
pub mod campaign_sampling;
pub mod campaign_work;
pub mod content_hash;
pub mod es;
pub mod explain;
pub mod matchups;
pub mod mlp;
pub mod objective;
pub mod rng;
pub mod sensor_support;
pub mod strategic;
pub mod strategic_reward;
pub mod strategic_sensors;
pub mod strategic_telemetry;
pub mod structural_search;
pub mod structure;
pub mod tactical;
pub mod train;
pub mod watch;

pub use brain::{Brain, BrainError, Module, ModuleSpec};
pub use es::{EsConfig, EvolutionStrategy};
pub use mlp::Mlp;
pub use objective::{BattleAssessment, ObjectiveWeights};
pub use tactical::TacticalPolicy;
pub use train::{TrainConfig, TrainError, TrainSummary};
