//! Small, deterministic edits of a champion's existing feed-forward structure.
use super::{brain::Brain, rng::Rng, structure::Activation};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StructuralSearchConfig {
    pub every: u64,
    pub activations: Vec<Activation>,
    pub activation_weight: f64,
    pub input_weight: f64,
    pub connection_weight: f64,
}
impl Default for StructuralSearchConfig {
    fn default() -> Self {
        Self {
            every: 10,
            activations: vec![
                Activation::Tanh,
                Activation::Softsign,
                Activation::LeakyRelu,
            ],
            activation_weight: 1.0,
            input_weight: 1.0,
            connection_weight: 1.0,
        }
    }
}
impl StructuralSearchConfig {
    pub fn validate(&self) -> Result<(), String> {
        let weights = [
            self.activation_weight,
            self.input_weight,
            self.connection_weight,
        ];
        if self.every == 0
            || weights.iter().any(|v| !v.is_finite() || *v < 0.0)
            || !weights.iter().sum::<f64>().is_finite()
            || weights.iter().sum::<f64>() == 0.0
            || (self.activation_weight > 0.0 && self.activations.is_empty())
        {
            return Err(
                "invalid structural search cadence, activations or mutation weights".into(),
            );
        }
        for (i, a) in self.activations.iter().enumerate() {
            if self.activations[..i].contains(a) {
                return Err("duplicate structural activation choice".into());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StructuralEdit {
    Activation {
        module: String,
        layer: usize,
        neuron: usize,
        before: Activation,
        after: Activation,
    },
    Input {
        module: String,
        input: String,
        before: bool,
        after: bool,
    },
    Connection {
        module: String,
        layer: usize,
        source: usize,
        destination: usize,
        before: bool,
        after: bool,
    },
}

pub fn mutate(
    parent: &Brain,
    trainable: &[bool],
    config: &StructuralSearchConfig,
    seed: u64,
) -> Result<(Brain, StructuralEdit), String> {
    config.validate()?;
    if trainable.len() != parent.modules.len() {
        return Err("structural module mask mismatch".into());
    }
    // Store edits, not copies of whole brains, while enumerating legal proposals.
    let mut categories: [Vec<(usize, StructuralEdit)>; 3] = Default::default();
    for (m, module) in parent.modules.iter().enumerate() {
        if !trainable[m] {
            continue;
        }
        let structure = module.net.structure();
        if config.activation_weight > 0.0 {
            for (layer, neurons) in structure.hidden.iter().enumerate() {
                for (neuron, before) in neurons.iter().enumerate() {
                    for after in &config.activations {
                        if before != after {
                            categories[0].push((
                                m,
                                StructuralEdit::Activation {
                                    module: module.spec.name.clone(),
                                    layer,
                                    neuron,
                                    before: *before,
                                    after: *after,
                                },
                            ));
                        }
                    }
                }
            }
        }
        if config.input_weight > 0.0 {
            for (input, before) in structure.inputs.iter().enumerate() {
                let mut trial = structure.clone();
                trial.inputs[input] = !before;
                if trial.outputs_reachable(module.net.sizes()) {
                    categories[1].push((
                        m,
                        StructuralEdit::Input {
                            module: module.spec.name.clone(),
                            input: module.spec.inputs[input].clone(),
                            before: *before,
                            after: !before,
                        },
                    ));
                }
            }
        }
        if config.connection_weight > 0.0 {
            for (layer, rows) in structure.connections.iter().enumerate() {
                for (destination, row) in rows.iter().enumerate() {
                    for (source, before) in row.iter().enumerate() {
                        let mut trial = structure.clone();
                        trial.connections[layer][destination][source] = !before;
                        if trial.outputs_reachable(module.net.sizes()) {
                            categories[2].push((
                                m,
                                StructuralEdit::Connection {
                                    module: module.spec.name.clone(),
                                    layer,
                                    source,
                                    destination,
                                    before: *before,
                                    after: !before,
                                },
                            ));
                        }
                    }
                }
            }
        }
    }
    let weights = [
        config.activation_weight,
        config.input_weight,
        config.connection_weight,
    ];
    let total: f64 = categories
        .iter()
        .zip(weights)
        .filter(|(c, _)| !c.is_empty())
        .map(|(_, w)| w)
        .sum();
    if total == 0.0 {
        return Err("no legal effective structural mutation in trainable modules".into());
    }
    let mut rng = Rng::new(seed);
    let mut choice = f64::from(rng.uniform()) * total;
    let mut selected = 0;
    for (i, category) in categories.iter().enumerate() {
        if category.is_empty() {
            continue;
        }
        selected = i;
        if choice < weights[i] {
            break;
        }
        choice -= weights[i];
    }
    let category = &categories[selected];
    let (module_index, edit) = category[(rng.next_u64() % category.len() as u64) as usize].clone();
    let mut child = parent.clone();
    let module = &mut child.modules[module_index];
    let mut structure = module.net.structure().clone();
    match &edit {
        StructuralEdit::Activation {
            layer,
            neuron,
            after,
            ..
        } => structure.hidden[*layer][*neuron] = *after,
        StructuralEdit::Input { input, after, .. } => {
            structure.inputs[module.spec.inputs.iter().position(|n| n == input).unwrap()] = *after
        }
        StructuralEdit::Connection {
            layer,
            source,
            destination,
            after,
            ..
        } => structure.connections[*layer][*destination][*source] = *after,
    }
    module
        .net
        .set_structure(structure)
        .map_err(|e| e.to_string())?;
    Ok((child, edit))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::learn::tactical;
    #[test]
    fn mutations_are_deterministic_inherit_weights_and_respect_frozen_modules() {
        let parent = tactical::template(4).unwrap();
        let mask: Vec<bool> = parent
            .modules
            .iter()
            .enumerate()
            .map(|(i, _)| i == 1)
            .collect();
        for kind in 0..3 {
            let config = StructuralSearchConfig {
                activation_weight: if kind == 0 { 1.0 } else { 0.0 },
                input_weight: if kind == 1 { 1.0 } else { 0.0 },
                connection_weight: if kind == 2 { 1.0 } else { 0.0 },
                ..Default::default()
            };
            let (child, edit) = mutate(&parent, &mask, &config, 91).unwrap();
            assert_ne!(child, parent);
            assert_eq!(
                (child.clone(), edit),
                mutate(&parent, &mask, &config, 91).unwrap()
            );
            for (i, (a, b)) in parent.modules.iter().zip(&child.modules).enumerate() {
                assert_eq!(a.net.params(), b.net.params());
                assert!(b.net.structure().outputs_reachable(b.net.sizes()));
                if !mask[i] {
                    assert_eq!(a, b);
                }
            }
        }
        assert!(mutate(&parent, &vec![false; mask.len()], &Default::default(), 1).is_err());
    }
}
