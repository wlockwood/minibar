use super::mlp::{Mlp, MlpError};
use super::rng::Rng;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;

pub const BRAIN_FORMAT: &str = "minibar-brain";
pub const BRAIN_VERSION: u32 = 2;

/// What a module is *for*, in human terms. The input and output names are the
/// contract between the encoder code and the weights; they are stored in the
/// weights file so it can be read (and checked) without the source.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleSpec {
    pub name: String,
    /// One sentence: the single decision this module owns.
    pub role: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    /// Hidden layer widths. Small on purpose: a handful of units can be read.
    pub hidden: Vec<usize>,
}

impl ModuleSpec {
    pub fn layer_sizes(&self) -> Vec<usize> {
        let mut sizes = vec![self.inputs.len()];
        sizes.extend(&self.hidden);
        sizes.push(self.outputs.len());
        sizes
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Module {
    pub spec: ModuleSpec,
    pub net: Mlp,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Brain {
    /// Names the wiring that consumes the modules, e.g. `tactical_v1`.
    pub kind: String,
    pub modules: Vec<Module>,
}

#[derive(Debug)]
pub enum BrainError {
    Net { module: String, source: MlpError },
    Io(String),
    Parse(String),
    UnsupportedFormat(String),
    KindMismatch { expected: String, found: String },
    ModuleMismatch { module: String, reason: String },
    ParameterCount { expected: usize, found: usize },
    UnknownModule(String),
}

impl fmt::Display for BrainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Net { module, source } => write!(f, "module {module:?}: {source}"),
            Self::Io(message) => f.write_str(message),
            Self::Parse(message) => write!(f, "invalid brain file: {message}"),
            Self::UnsupportedFormat(found) => write!(f, "unsupported brain format {found:?}"),
            Self::KindMismatch { expected, found } => {
                write!(f, "brain kind is {found:?}, expected {expected:?}")
            }
            Self::ModuleMismatch { module, reason } => write!(f, "module {module:?}: {reason}"),
            Self::ParameterCount { expected, found } => {
                write!(f, "expected {expected} trainable parameters, found {found}")
            }
            Self::UnknownModule(name) => write!(f, "no module named {name:?}"),
        }
    }
}

impl std::error::Error for BrainError {}

// --- File format ----------------------------------------------------------

#[derive(Serialize, Deserialize)]
struct LayerFile {
    /// `weights[output_unit][input_unit]`
    weights: Vec<Vec<f32>>,
    biases: Vec<f32>,
}

#[derive(Serialize, Deserialize)]
struct ModuleFile {
    #[serde(flatten)]
    spec: ModuleSpec,
    layers: Vec<LayerFile>,
    #[serde(default)]
    structure: Option<super::structure::MlpStructure>,
}

#[derive(Serialize, Deserialize)]
struct BrainFile {
    format: String,
    version: u32,
    kind: String,
    modules: Vec<ModuleFile>,
}

impl Brain {
    /// Fresh random brain from specs. Output layers are scaled down so the
    /// untrained council is close to neutral.
    pub fn random(
        kind: &str,
        specs: Vec<ModuleSpec>,
        rng: &mut Rng,
        output_scale: f32,
    ) -> Result<Self, BrainError> {
        let mut modules = Vec::with_capacity(specs.len());
        for spec in specs {
            let net =
                Mlp::randomized(&spec.layer_sizes(), rng, output_scale).map_err(|source| {
                    BrainError::Net {
                        module: spec.name.clone(),
                        source,
                    }
                })?;
            modules.push(Module { spec, net });
        }
        Ok(Self {
            kind: kind.to_owned(),
            modules,
        })
    }

    pub fn module(&self, name: &str) -> Option<&Module> {
        self.modules.iter().find(|module| module.spec.name == name)
    }

    pub fn module_index(&self, name: &str) -> Result<usize, BrainError> {
        self.modules
            .iter()
            .position(|module| module.spec.name == name)
            .ok_or_else(|| BrainError::UnknownModule(name.to_owned()))
    }

    pub fn param_count(&self) -> usize {
        self.modules.iter().map(|m| m.net.params().len()).sum()
    }

    /// Checks names, roles are free to differ, but the labelled inputs/outputs
    /// must match what the code that consumes this brain expects. Hidden widths
    /// may differ, which is how a smaller or larger council is trained.
    pub fn check_against(&self, kind: &str, expected: &[ModuleSpec]) -> Result<(), BrainError> {
        if self.kind != kind {
            return Err(BrainError::KindMismatch {
                expected: kind.to_owned(),
                found: self.kind.clone(),
            });
        }
        if self.modules.len() != expected.len() {
            return Err(BrainError::ModuleMismatch {
                module: "*".into(),
                reason: format!(
                    "expected {} modules, found {}",
                    expected.len(),
                    self.modules.len()
                ),
            });
        }
        for (module, want) in self.modules.iter().zip(expected) {
            let mismatch = |reason: String| BrainError::ModuleMismatch {
                module: want.name.clone(),
                reason,
            };
            if module.spec.name != want.name {
                return Err(mismatch(format!("found module {:?}", module.spec.name)));
            }
            if module.spec.inputs != want.inputs {
                return Err(mismatch(format!(
                    "inputs {:?} differ from expected {:?}",
                    module.spec.inputs, want.inputs
                )));
            }
            if module.spec.outputs != want.outputs {
                return Err(mismatch(format!(
                    "outputs {:?} differ from expected {:?}",
                    module.spec.outputs, want.outputs
                )));
            }
        }
        Ok(())
    }

    // --- Evolution-strategy view -------------------------------------------

    /// Number of parameters in the modules selected by `trainable`.
    pub fn trainable_len(&self, trainable: &[bool]) -> usize {
        self.modules
            .iter()
            .zip(trainable)
            .filter(|(_, on)| **on)
            .map(|(module, _)| module.net.params().len())
            .sum()
    }

    /// Concatenated parameters of the selected modules, in module order.
    pub fn flatten(&self, trainable: &[bool]) -> Vec<f32> {
        let mut flat = Vec::with_capacity(self.trainable_len(trainable));
        for (module, on) in self.modules.iter().zip(trainable) {
            if *on {
                flat.extend_from_slice(module.net.params());
            }
        }
        flat
    }

    pub fn parameter_activity(&self, trainable: &[bool]) -> Vec<bool> {
        self.modules
            .iter()
            .zip(trainable)
            .filter(|(_, on)| **on)
            .flat_map(|(m, _)| m.net.structure().parameter_activity(m.net.sizes()))
            .collect()
    }

    /// Copy of this brain with the selected modules' parameters replaced.
    /// Unselected modules are untouched (frozen).
    pub fn with_flat(&self, trainable: &[bool], flat: &[f32]) -> Result<Self, BrainError> {
        let expected = self.trainable_len(trainable);
        if flat.len() != expected {
            return Err(BrainError::ParameterCount {
                expected,
                found: flat.len(),
            });
        }
        let mut brain = self.clone();
        let mut offset = 0;
        for (module, on) in brain.modules.iter_mut().zip(trainable) {
            if *on {
                let len = module.net.params().len();
                module
                    .net
                    .params_mut()
                    .copy_from_slice(&flat[offset..offset + len]);
                offset += len;
            }
        }
        Ok(brain)
    }

    /// Mask selecting the named modules, or every module when `names` is empty.
    pub fn trainable_mask(&self, names: &[String]) -> Result<Vec<bool>, BrainError> {
        if names.is_empty() {
            return Ok(vec![true; self.modules.len()]);
        }
        let mut mask = vec![false; self.modules.len()];
        for name in names {
            mask[self.module_index(name)?] = true;
        }
        Ok(mask)
    }

    // --- Persistence ---------------------------------------------------------

    pub fn to_json(&self) -> String {
        let file = BrainFile {
            format: BRAIN_FORMAT.into(),
            version: BRAIN_VERSION,
            kind: self.kind.clone(),
            modules: self
                .modules
                .iter()
                .map(|module| ModuleFile {
                    spec: module.spec.clone(),
                    structure: Some(module.net.structure().clone()),
                    layers: module
                        .net
                        .layers()
                        .into_iter()
                        .map(|(weights, biases)| LayerFile { weights, biases })
                        .collect(),
                })
                .collect(),
        };
        serde_json::to_string_pretty(&file).expect("brain serializes")
    }

    pub fn from_json(text: &str) -> Result<Self, BrainError> {
        let file: BrainFile =
            serde_json::from_str(text).map_err(|error| BrainError::Parse(error.to_string()))?;
        if file.format != BRAIN_FORMAT || !matches!(file.version, 1 | BRAIN_VERSION) {
            return Err(BrainError::UnsupportedFormat(format!(
                "{} v{}",
                file.format, file.version
            )));
        }
        let mut modules = Vec::with_capacity(file.modules.len());
        for module in file.modules {
            let sizes = module.spec.layer_sizes();
            if module.layers.len() != sizes.len() - 1 {
                return Err(BrainError::ModuleMismatch {
                    module: module.spec.name.clone(),
                    reason: format!(
                        "expected {} layers, found {}",
                        sizes.len() - 1,
                        module.layers.len()
                    ),
                });
            }
            let mut params = Vec::new();
            for (layer, pair) in module.layers.iter().zip(sizes.windows(2)) {
                let (inputs, outputs) = (pair[0], pair[1]);
                let shape_ok = layer.weights.len() == outputs
                    && layer.weights.iter().all(|row| row.len() == inputs)
                    && layer.biases.len() == outputs;
                if !shape_ok {
                    return Err(BrainError::ModuleMismatch {
                        module: module.spec.name.clone(),
                        reason: format!("layer shape is not {outputs} x {inputs}"),
                    });
                }
                for row in &layer.weights {
                    params.extend_from_slice(row);
                }
                params.extend_from_slice(&layer.biases);
            }
            let mut net = Mlp::from_params(&sizes, params).map_err(|source| BrainError::Net {
                module: module.spec.name.clone(),
                source,
            })?;
            if file.version == 1 && module.structure.is_some() {
                return Err(BrainError::Parse(
                    "version 1 cannot contain structural metadata".into(),
                ));
            }
            if file.version == BRAIN_VERSION {
                let structure = module.structure.ok_or_else(|| {
                    BrainError::Parse("version 2 requires module structure".into())
                })?;
                net.set_structure(structure)
                    .map_err(|source| BrainError::Net {
                        module: module.spec.name.clone(),
                        source,
                    })?;
            }
            modules.push(Module {
                spec: module.spec,
                net,
            });
        }
        Ok(Self {
            kind: file.kind,
            modules,
        })
    }

    pub fn save(&self, path: &Path) -> Result<(), BrainError> {
        std::fs::write(path, self.to_json())
            .map_err(|error| BrainError::Io(format!("{}: {error}", path.display())))
    }

    pub fn load(path: &Path) -> Result<Self, BrainError> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| BrainError::Io(format!("{}: {error}", path.display())))?;
        Self::from_json(&text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structural_brain_round_trip_and_legacy_defaults() {
        use crate::learn::structure::Activation;
        let mut brain = Brain::random("test", specs(), &mut Rng::new(7), 1.0).unwrap();
        let mut structure = brain.modules[0].net.structure().clone();
        structure.inputs[0] = false;
        structure.hidden[0][0] = Activation::LeakyRelu;
        brain.modules[0].net.set_structure(structure).unwrap();
        let encoded = brain.to_json();
        assert_eq!(Brain::from_json(&encoded).unwrap(), brain);
        let mut legacy: serde_json::Value = serde_json::from_str(&encoded).unwrap();
        legacy["version"] = 1.into();
        for module in legacy["modules"].as_array_mut().unwrap() {
            module.as_object_mut().unwrap().remove("structure");
        }
        let loaded = Brain::from_json(&legacy.to_string()).unwrap();
        assert!(
            loaded.modules[0]
                .net
                .structure()
                .inputs
                .iter()
                .all(|on| *on)
        );
        legacy["version"] = 2.into();
        assert!(Brain::from_json(&legacy.to_string()).is_err());
    }

    fn specs() -> Vec<ModuleSpec> {
        vec![
            ModuleSpec {
                name: "alpha".into(),
                role: "first".into(),
                inputs: vec!["a".into(), "b".into()],
                outputs: vec!["x".into()],
                hidden: vec![3],
            },
            ModuleSpec {
                name: "beta".into(),
                role: "second".into(),
                inputs: vec!["c".into()],
                outputs: vec!["y".into(), "z".into()],
                hidden: vec![],
            },
        ]
    }

    #[test]
    fn json_round_trip_is_exact_and_self_describing() {
        let brain = Brain::random("test", specs(), &mut Rng::new(1), 1.0).unwrap();
        let json = brain.to_json();
        assert!(json.contains("\"role\": \"first\""), "{json}");
        assert!(json.contains("\"weights\""));
        assert_eq!(Brain::from_json(&json).unwrap(), brain);
    }

    #[test]
    fn frozen_modules_are_untouched_by_flat_updates() {
        let brain = Brain::random("test", specs(), &mut Rng::new(2), 1.0).unwrap();
        let mask = brain.trainable_mask(&["beta".to_string()]).unwrap();
        assert_eq!(mask, vec![false, true]);
        let mut flat = brain.flatten(&mask);
        assert_eq!(flat.len(), brain.modules[1].net.params().len());
        flat.iter_mut().for_each(|p| *p += 1.0);
        let updated = brain.with_flat(&mask, &flat).unwrap();
        assert_eq!(updated.modules[0], brain.modules[0]);
        assert_ne!(updated.modules[1], brain.modules[1]);
        assert!(brain.with_flat(&mask, &flat[1..]).is_err());
    }

    #[test]
    fn mismatched_labels_are_rejected_but_hidden_width_may_differ() {
        let brain = Brain::random("test", specs(), &mut Rng::new(3), 1.0).unwrap();
        let mut wider = specs();
        wider[0].hidden = vec![5, 5];
        assert!(brain.check_against("test", &wider).is_ok());
        let mut renamed = specs();
        renamed[0].inputs[1] = "different".into();
        assert!(matches!(
            brain.check_against("test", &renamed),
            Err(BrainError::ModuleMismatch { .. })
        ));
        assert!(matches!(
            brain.check_against("other", &specs()),
            Err(BrainError::KindMismatch { .. })
        ));
    }

    #[test]
    fn corrupt_layer_shapes_fail_to_load() {
        let brain = Brain::random("test", specs(), &mut Rng::new(4), 1.0).unwrap();
        let broken = brain
            .to_json()
            .replacen("\"biases\": [", "\"biases\": [0.0,", 1);
        assert!(Brain::from_json(&broken).is_err());
        assert!(
            Brain::from_json("{\"format\":\"nope\",\"version\":1,\"kind\":\"k\",\"modules\":[]}")
                .is_err()
        );
    }
}
