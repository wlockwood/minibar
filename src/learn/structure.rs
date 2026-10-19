//! Executable structure of a fixed-width feed-forward module.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Activation {
    Tanh,
    Softsign,
    LeakyRelu,
}

impl Activation {
    pub fn apply(self, value: f32) -> f32 {
        match self {
            Self::Tanh => value.tanh(),
            Self::Softsign => value / (1.0 + value.abs()),
            Self::LeakyRelu => {
                if value < 0.0 {
                    value * 0.01
                } else {
                    value
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MlpStructure {
    pub inputs: Vec<bool>,
    /// [layer][destination][source], matching the stored weight matrices.
    pub connections: Vec<Vec<Vec<bool>>>,
    /// Hidden layers only; final outputs always use tanh.
    pub hidden: Vec<Vec<Activation>>,
}

impl MlpStructure {
    pub fn dense(sizes: &[usize]) -> Self {
        Self {
            inputs: vec![true; sizes[0]],
            connections: sizes
                .windows(2)
                .map(|p| vec![vec![true; p[0]]; p[1]])
                .collect(),
            hidden: sizes[1..sizes.len() - 1]
                .iter()
                .map(|n| vec![Activation::Tanh; *n])
                .collect(),
        }
    }

    pub fn validate(&self, sizes: &[usize]) -> Result<(), String> {
        if sizes.len() < 2
            || self.inputs.len() != sizes[0]
            || self.connections.len() != sizes.len() - 1
            || self.hidden.len() != sizes.len() - 2
        {
            return Err("structure dimensions do not match network".into());
        }
        for (edges, p) in self.connections.iter().zip(sizes.windows(2)) {
            if edges.len() != p[1] || edges.iter().any(|row| row.len() != p[0]) {
                return Err("connection mask does not match weight matrix".into());
            }
        }
        if self
            .hidden
            .iter()
            .zip(&sizes[1..sizes.len() - 1])
            .any(|(a, n)| a.len() != *n)
        {
            return Err("hidden activation count does not match layer width".into());
        }
        Ok(())
    }

    pub fn parameter_activity(&self, sizes: &[usize]) -> Vec<bool> {
        let mut active = Vec::new();
        for (layer, p) in sizes.windows(2).enumerate() {
            for row in &self.connections[layer] {
                active.extend(
                    row.iter()
                        .enumerate()
                        .map(|(i, on)| *on && (layer != 0 || self.inputs[i])),
                );
            }
            active.extend(std::iter::repeat_n(true, p[1]));
        }
        active
    }

    pub fn outputs_reachable(&self, sizes: &[usize]) -> bool {
        if self.validate(sizes).is_err() {
            return false;
        }
        let mut reachable = self.inputs.clone();
        for layer in &self.connections {
            reachable = layer
                .iter()
                .map(|row| {
                    row.iter()
                        .zip(&reachable)
                        .any(|(edge, from)| *edge && *from)
                })
                .collect();
        }
        reachable.iter().all(|on| *on)
    }
}
