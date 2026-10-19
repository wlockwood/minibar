//! Human-readable account of a trained council.
//!
//! Static structure (weights) alone says little, so the report is grounded in
//! behaviour: the council is run through the validation battles, every module
//! input it actually saw is recorded, and each module is then described by
//!
//! * what it outputs in practice (distribution),
//! * how much each input *moves* that output over the range the input really
//!   takes ("swing"), and in which direction,
//! * one-input-at-a-time sweeps with the other inputs held typical,
//! * what each hidden unit reads and what it pushes (single-hidden-layer
//!   modules), and
//! * a few complete worked decisions.

use super::brain::{Brain, Module};
use super::tactical::{DecisionTrace, ModuleTrace, TacticalPolicy};
use super::train::{Evaluation, Opponent, TrainConfig, TrainError, validation_simulators};
use crate::catalog::UnitCatalog;
use crate::model::{Command, SIM_HZ, UnitState, WorldView};
use crate::policy::Policy;
use std::fmt::Write;
use std::sync::Mutex;

/// Decisions are sampled every `SAMPLE_EVERY_TICKS` ticks (0.2 s at 30 Hz).
const SAMPLE_EVERY_TICKS: u64 = 6;
const MAX_TRACES: usize = 6_000;
const MAX_SAMPLES_PER_MODULE: usize = 1_500;
const SWEEP_POINTS: usize = 5;
const EXAMPLES: usize = 3;
const BAR_WIDTH: usize = 20;
const VALIDATION_SEED_BASE: u64 = 1_000_000_000;

#[derive(Clone, Debug, Default)]
pub struct Recording {
    pub traces: Vec<DecisionTrace>,
}

struct Recorder<'a> {
    policy: &'a TacticalPolicy,
    traces: Mutex<Vec<DecisionTrace>>,
}

impl Policy for Recorder<'_> {
    fn command(&self, world: &WorldView<'_>, unit: &UnitState) -> Command {
        let (command, trace) = self.policy.explain(world, unit);
        let tick = (world.time * SIM_HZ).round() as u64;
        if tick % SAMPLE_EVERY_TICKS == 0 {
            if let Some(trace) = trace {
                let mut traces = self.traces.lock().expect("recorder lock");
                if traces.len() < MAX_TRACES {
                    traces.push(trace);
                }
            }
        }
        command
    }
}

/// Plays the council through every validation battle (sequentially, so the
/// recording is reproducible) and keeps sampled decision traces.
pub fn record(
    config: &TrainConfig,
    catalog: &UnitCatalog,
    policy: &TacticalPolicy,
    opponents: &[Opponent],
    seeds: u64,
) -> Result<Recording, TrainError> {
    let recorder = Recorder {
        policy,
        traces: Mutex::new(Vec::new()),
    };
    for (_, learner_side, simulator) in validation_simulators(config, catalog)? {
        for opponent in opponents {
            for seed in 0..seeds.max(1) {
                let policies: [&dyn Policy; 2] = if learner_side == 0 {
                    [&recorder, opponent.policy.as_ref()]
                } else {
                    [opponent.policy.as_ref(), &recorder]
                };
                simulator.run(policies, VALIDATION_SEED_BASE + seed);
            }
        }
    }
    Ok(Recording {
        traces: recorder.traces.into_inner().expect("recorder lock"),
    })
}

// --- Statistics ---------------------------------------------------------------------

fn quantile(sorted: &[f32], q: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let position = q * (sorted.len() - 1) as f32;
    let (low, high) = (position.floor() as usize, position.ceil() as usize);
    sorted[low] + (sorted[high] - sorted[low]) * (position - low as f32)
}

fn sorted(mut values: Vec<f32>) -> Vec<f32> {
    values.sort_by(f32::total_cmp);
    values
}

fn bar(fraction: f32) -> String {
    let filled = (fraction.clamp(0.0, 1.0) * BAR_WIDTH as f32).round() as usize;
    format!("{}{}", "█".repeat(filled), "·".repeat(BAR_WIDTH - filled))
}

fn signed(value: f32) -> String {
    format!("{value:+.2}")
}

struct ModuleSamples<'a> {
    runs: Vec<&'a ModuleTrace>,
}

fn samples_for<'a>(recording: &'a Recording, module: &Module) -> ModuleSamples<'a> {
    let all: Vec<&ModuleTrace> = recording
        .traces
        .iter()
        .flat_map(|trace| trace.modules.iter())
        .filter(|run| run.module == module.spec.name)
        .collect();
    let stride = all.len().div_ceil(MAX_SAMPLES_PER_MODULE).max(1);
    ModuleSamples {
        runs: all.into_iter().step_by(stride).collect(),
    }
}

fn evaluate_module(module: &Module, input: &[f32]) -> Vec<f32> {
    let mut output = vec![0.0; module.net.output_len()];
    module.net.forward(input, &mut output);
    output
}

// --- Report ---------------------------------------------------------------------------

pub fn report(brain: &Brain, recording: &Recording, evaluation: Option<&Evaluation>) -> String {
    let mut out = String::new();
    let w = &mut out;
    let _ = writeln!(w, "# Council report: `{}`\n", brain.kind);
    let _ = writeln!(
        w,
        "{} modules, {} parameters. Each module is a small perceptron with one job; \
         the formula that combines them is fixed code (see `learn/tactical.rs`). \
         Behaviour below is measured from {} sampled decisions.\n",
        brain.modules.len(),
        brain.param_count(),
        recording.traces.len()
    );

    if let Some(evaluation) = evaluation {
        write_results(w, evaluation);
    }

    for module in &brain.modules {
        write_module(w, module, &samples_for(recording, module));
    }

    write_examples(w, recording);
    out
}

fn write_results(w: &mut String, evaluation: &Evaluation) {
    let _ = writeln!(w, "## Results on validation battles\n");
    let _ = writeln!(
        w,
        "| scenario | opponent | W | timeout | wipe-out | L | win rate | 95% CI | mean exchange |"
    );
    let _ = writeln!(w, "|---|---|---|---|---|---|---|---|---|");
    for row in &evaluation.rows {
        let t = &row.tally;
        let [low, high] = t.win_rate_95ci();
        let _ = writeln!(
            w,
            "| {} | {} | {} | {} | {} | {} | {:.0}% | {:.0}–{:.0}% | {} |",
            row.scenario,
            row.opponent,
            t.wins,
            t.timeouts,
            t.draws,
            t.losses,
            t.win_rate() * 100.0,
            low * 100.0,
            high * 100.0,
            signed(t.mean_exchange() as f32),
        );
    }
    let t = &evaluation.total;
    let _ = write!(
        w,
        "\nOverall win rate {:.0}%, mean fitness {}",
        t.win_rate() * 100.0,
        signed(t.mean_fitness() as f32)
    );
    if let Some(rate) = t.underdog_win_rate() {
        let _ = write!(
            w,
            ", **{:.0}% in battles where it was the underdog** ({} battles)",
            rate * 100.0,
            t.underdog_battles
        );
    }
    let _ = writeln!(w, ".\n");
}

fn write_module(w: &mut String, module: &Module, samples: &ModuleSamples<'_>) {
    let structure = module.net.structure();
    let enabled = structure.inputs.iter().filter(|on| **on).count();
    let edges = structure
        .connections
        .iter()
        .flatten()
        .flatten()
        .filter(|on| **on)
        .count();
    let _ = writeln!(
        w,
        "\nStructure: {enabled}/{} inputs enabled, {edges} connections enabled. Hidden responses: {:?}. Outputs use tanh.\n",
        structure.inputs.len(),
        structure.hidden
    );

    let spec = &module.spec;
    let sizes = module.net.sizes();
    let shape = sizes
        .iter()
        .map(usize::to_string)
        .collect::<Vec<_>>()
        .join(" → ");
    let _ = writeln!(w, "## `{}`\n", spec.name);
    let _ = writeln!(w, "{}\n", spec.role);
    let _ = writeln!(
        w,
        "Shape {shape} · {} parameters · {} samples\n",
        module.net.params().len(),
        samples.runs.len()
    );
    if samples.runs.is_empty() {
        let _ = writeln!(w, "_Never ran in the recorded battles._\n");
        return;
    }

    let inputs: Vec<&[(String, f32)]> = samples.runs.iter().map(|r| r.inputs.as_slice()).collect();
    let vectors: Vec<Vec<f32>> = inputs
        .iter()
        .map(|named| named.iter().map(|(_, value)| *value).collect())
        .collect();
    let outputs: Vec<Vec<f32>> = vectors.iter().map(|v| evaluate_module(module, v)).collect();

    for (j, output_name) in spec.outputs.iter().enumerate() {
        let column = sorted(outputs.iter().map(|o| o[j]).collect());
        let mean = column.iter().sum::<f32>() / column.len() as f32;
        let strong = column.iter().filter(|v| v.abs() > 0.5).count() as f32 / column.len() as f32;
        let _ = writeln!(w, "### output `{output_name}`\n");
        let _ = writeln!(
            w,
            "mean {} · 10th pct {} · median {} · 90th pct {} · strongly active (|x| > 0.5) {:.0}% of the time\n",
            signed(mean),
            signed(quantile(&column, 0.1)),
            signed(quantile(&column, 0.5)),
            signed(quantile(&column, 0.9)),
            strong * 100.0
        );

        // Swing: change in this output when one input moves from its typical
        // low (10th pct) to typical high (90th pct), others as observed.
        let mut rows = Vec::new();
        let mut medians = Vec::new();
        let mut spans = Vec::new();
        for (i, input_name) in spec.inputs.iter().enumerate() {
            let column_i = sorted(vectors.iter().map(|v| v[i]).collect());
            let (low, high) = (quantile(&column_i, 0.1), quantile(&column_i, 0.9));
            medians.push(quantile(&column_i, 0.5));
            spans.push((quantile(&column_i, 0.0), quantile(&column_i, 1.0)));
            let (mut total_abs, mut total_signed) = (0.0f32, 0.0f32);
            if high > low {
                for vector in &vectors {
                    let mut at_low = vector.clone();
                    let mut at_high = vector.clone();
                    at_low[i] = low;
                    at_high[i] = high;
                    let delta =
                        evaluate_module(module, &at_high)[j] - evaluate_module(module, &at_low)[j];
                    total_abs += delta.abs();
                    total_signed += delta;
                }
            }
            let n = vectors.len() as f32;
            rows.push((
                input_name.as_str(),
                quantile(&column_i, 0.5),
                low,
                high,
                total_abs / n,
                total_signed / n,
            ));
        }
        rows.sort_by(|a, b| b.4.total_cmp(&a.4));
        let strongest = rows.first().map_or(1.0, |r| r.4).max(1e-6);

        let _ = writeln!(
            w,
            "What it listens to, most influential first. *Swing* is the average change in `{output_name}` when the input goes from its typical low to its typical high.\n"
        );
        let _ = writeln!(w, "| input | typical range (10–90%) | swing | effect | |");
        let _ = writeln!(w, "|---|---|---|---|---|");
        for (name, _median, low, high, swing, net) in &rows {
            let effect = if *swing < 0.02 {
                "ignored".to_owned()
            } else if net.abs() < 0.5 * swing {
                "mixed (depends on other inputs)".to_owned()
            } else if *net > 0.0 {
                format!("higher → raises `{output_name}`")
            } else {
                format!("higher → lowers `{output_name}`")
            };
            let _ = writeln!(
                w,
                "| `{name}` | {low:.2} – {high:.2} | {swing:.2} | {effect} | `{}` |",
                bar(swing / strongest)
            );
        }
        let _ = writeln!(w);

        // Sweeps with everything else at its median.
        let _ = writeln!(
            w,
            "Sweeps (one input varied across what it really takes; others at their median):\n"
        );
        let header: Vec<String> = (0..SWEEP_POINTS)
            .map(|k| format!("point {}", k + 1))
            .collect();
        let _ = writeln!(w, "| input | {} |", header.join(" | "));
        let _ = writeln!(w, "|---|{}|", vec!["---"; SWEEP_POINTS].join("|"));
        for (i, input_name) in spec.inputs.iter().enumerate() {
            let (low, high) = spans[i];
            if high <= low {
                continue;
            }
            let cells: Vec<String> = (0..SWEEP_POINTS)
                .map(|k| {
                    let value = low + (high - low) * k as f32 / (SWEEP_POINTS - 1) as f32;
                    let mut probe = medians.clone();
                    probe[i] = value;
                    format!(
                        "{value:.2} → {}",
                        signed(evaluate_module(module, &probe)[j])
                    )
                })
                .collect();
            let _ = writeln!(w, "| `{input_name}` | {} |", cells.join(" | "));
        }
        let _ = writeln!(w);
    }

    write_hidden_units(w, module);
}

fn write_hidden_units(w: &mut String, module: &Module) {
    let layers = module.net.layers();
    if layers.len() != 2 {
        let _ = writeln!(
            w,
            "_Deeper than one hidden layer: see the weight matrices in the brain file._\n"
        );
        return;
    }
    let (hidden_weights, hidden_biases) = &layers[0];
    let (output_weights, _) = &layers[1];
    let _ = writeln!(
        w,
        "Hidden units (each reads the inputs, then pushes the outputs):\n"
    );
    for (k, row) in hidden_weights.iter().enumerate() {
        let mut ranked: Vec<(usize, f32)> = row.iter().copied().enumerate().collect();
        ranked.sort_by(|a, b| b.1.abs().total_cmp(&a.1.abs()));
        let reads: Vec<String> = ranked
            .iter()
            .take(3)
            .filter(|(_, weight)| weight.abs() > 0.05)
            .map(|(i, weight)| format!("{}`{}`", signed(*weight), module.spec.inputs[*i]))
            .collect();
        let pushes: Vec<String> = module
            .spec
            .outputs
            .iter()
            .enumerate()
            .map(|(j, name)| format!("{} → `{name}`", signed(output_weights[j][k])))
            .collect();
        let reads = if reads.is_empty() {
            "nothing (dormant)".to_owned()
        } else {
            reads.join(" ")
        };
        let _ = writeln!(
            w,
            "- **h{k}** bias {}: reads {reads}; pushes {}",
            signed(hidden_biases[k]),
            pushes.join(", ")
        );
    }
    let _ = writeln!(w);
}

fn write_examples(w: &mut String, recording: &Recording) {
    let moving: Vec<&DecisionTrace> = recording
        .traces
        .iter()
        .filter(|t| t.throttle > 0.0)
        .collect();
    let pool = if moving.is_empty() {
        recording.traces.iter().collect()
    } else {
        moving
    };
    if pool.is_empty() {
        return;
    }
    let _ = writeln!(w, "## Worked decisions\n");
    for k in 0..EXAMPLES.min(pool.len()) {
        let trace = pool[k * pool.len() / EXAMPLES.min(pool.len())];
        let _ = writeln!(
            w,
            "**t = {:.1}s, unit #{}** at ({:.0}, {:.0}) chose enemy #{}",
            trace.time,
            trace.unit,
            trace.position[0],
            trace.position[1],
            trace
                .chosen_target
                .map_or("-".to_owned(), |id| id.to_string())
        );
        for run in &trace.modules {
            let inputs = run
                .inputs
                .iter()
                .map(|(name, value)| format!("{name}={value:.2}"))
                .collect::<Vec<_>>()
                .join(", ");
            let outputs = run
                .outputs
                .iter()
                .map(|(name, value)| format!("{name}={}", signed(*value)))
                .collect::<Vec<_>>()
                .join(", ");
            let label = run
                .label
                .as_deref()
                .map_or(String::new(), |l| format!(" [{l}]"));
            let _ = writeln!(w, "- `{}`{label}: {inputs} → **{outputs}**", run.module);
        }
        let _ = writeln!(
            w,
            "- combined: advance {} · orbit {} · regroup {} · retreat {} · bloom {} (platoon commit {:.2}) → movement ({:+.2}, {:+.2}), throttle {:.2}\n",
            signed(trace.advance),
            signed(trace.orbit),
            signed(trace.regroup),
            signed(trace.retreat),
            signed(trace.bloom),
            trace.commit,
            trace.movement[0],
            trace.movement[1],
            trace.throttle
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learn::rng::Rng;
    use crate::learn::tactical::{self, tests::skirmisher};
    use crate::learn::train::{evaluate_validation, opponents_from_config};

    fn setup() -> (TrainConfig, UnitCatalog, Brain) {
        let mut catalog = UnitCatalog::default();
        catalog.units.insert("skirm".into(), skirmisher("skirm"));
        let config = TrainConfig {
            opponents: vec!["charge".into(), "kite".into()],
            max_time: 15.0,
            validation_seeds: 2,
            ..TrainConfig::default_for_unit("skirm")
        };
        let brain =
            Brain::random(tactical::KIND, tactical::specs(5), &mut Rng::new(3), 1.0).unwrap();
        (config, catalog, brain)
    }

    #[test]
    fn report_names_every_module_input_and_output() {
        let (config, catalog, brain) = setup();
        let policy = TacticalPolicy::new(brain.clone()).unwrap();
        let opponents = opponents_from_config(&config).unwrap();
        let recording = record(&config, &catalog, &policy, &opponents, 1).unwrap();
        assert!(!recording.traces.is_empty());
        let evaluation = evaluate_validation(&config, &catalog, &policy, &opponents).unwrap();
        let text = report(&brain, &recording, Some(&evaluation));

        for module in &brain.modules {
            assert!(
                text.contains(&format!("## `{}`", module.spec.name)),
                "{text}"
            );
            for output in &module.spec.outputs {
                assert!(text.contains(&format!("### output `{output}`")));
            }
        }
        assert!(text.contains("## Results on validation battles"));
        assert!(text.contains("## Worked decisions"));
        assert!(text.contains("Hidden units"));
        assert!(text.contains("3v5 underdog"));
    }

    #[test]
    fn recording_is_reproducible() {
        let (config, catalog, brain) = setup();
        let policy = TacticalPolicy::new(brain).unwrap();
        let opponents = opponents_from_config(&config).unwrap();
        let a = record(&config, &catalog, &policy, &opponents, 1).unwrap();
        let b = record(&config, &catalog, &policy, &opponents, 1).unwrap();
        assert_eq!(a.traces, b.traces);
    }

    #[test]
    fn an_ignored_input_is_reported_as_ignored() {
        // Zero every weight that reads `my_hp_fraction` in `spacing`; the
        // report must then show that input as having no effect.
        let (config, catalog, mut brain) = setup();
        let index = brain.module_index(tactical::SPACING).unwrap();
        let module = &mut brain.modules[index];
        let hp = module
            .spec
            .inputs
            .iter()
            .position(|n| n == "my_hp_fraction")
            .unwrap();
        let (inputs, hidden) = (module.spec.inputs.len(), module.spec.hidden[0]);
        for unit in 0..hidden {
            module.net.params_mut()[unit * inputs + hp] = 0.0;
        }
        let policy = TacticalPolicy::new(brain.clone()).unwrap();
        let opponents = opponents_from_config(&config).unwrap();
        let recording = record(&config, &catalog, &policy, &opponents, 1).unwrap();
        let text = report(&brain, &recording, None);
        let line = text
            .lines()
            .find(|l| l.starts_with("| `my_hp_fraction`") && l.contains("ignored"))
            .unwrap_or_else(|| panic!("no ignored line in:\n{text}"));
        assert!(line.contains("0.00"), "{line}");
    }
}
