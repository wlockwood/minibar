use super::rng::Rng;
use super::structure::{Activation, MlpStructure};
use serde::Serialize;

/// Widest layer an [`Mlp`] may have. Inference keeps its two activation
/// buffers on the stack, so a policy call never allocates.
pub const MAX_WIDTH: usize = 32;

/// Fixed-width feed-forward network with optional input/edge masks and hidden
/// responses. Final outputs always use tanh for bounded, readable `-1..1` signals.
///
/// Parameters live in one flat vector (evolution strategies perturb it as a
/// whole). Per layer the order is the `out × in` weight matrix in row-major
/// order, then `out` biases.
#[derive(Clone, Debug, PartialEq)]
pub struct Mlp {
    sizes: Vec<usize>,
    params: Vec<f32>,
    structure: MlpStructure,
}

/// Per-layer activations recorded for explanation. `layers[0]` is the input.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MlpTrace {
    pub layers: Vec<Vec<f32>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MlpError {
    TooFewLayers,
    EmptyLayer,
    LayerTooWide { width: usize },
    ParameterCount { expected: usize, found: usize },
    Structure(String),
}

impl std::fmt::Display for MlpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Structure(message) => f.write_str(message),
            Self::TooFewLayers => f.write_str("a perceptron needs an input and an output layer"),
            Self::EmptyLayer => f.write_str("a perceptron layer cannot be empty"),
            Self::LayerTooWide { width } => {
                write!(f, "layer width {width} exceeds the maximum of {MAX_WIDTH}")
            }
            Self::ParameterCount { expected, found } => {
                write!(f, "expected {expected} parameters, found {found}")
            }
        }
    }
}

impl std::error::Error for MlpError {}

fn validate(sizes: &[usize]) -> Result<(), MlpError> {
    if sizes.len() < 2 {
        return Err(MlpError::TooFewLayers);
    }
    for &width in sizes {
        if width == 0 {
            return Err(MlpError::EmptyLayer);
        }
        if width > MAX_WIDTH {
            return Err(MlpError::LayerTooWide { width });
        }
    }
    Ok(())
}

pub fn parameter_count(sizes: &[usize]) -> usize {
    sizes
        .windows(2)
        .map(|pair| pair[0] * pair[1] + pair[1])
        .sum()
}

impl Mlp {
    pub fn zeroed(sizes: &[usize]) -> Result<Self, MlpError> {
        validate(sizes)?;
        Ok(Self {
            sizes: sizes.to_vec(),
            params: vec![0.0; parameter_count(sizes)],
            structure: MlpStructure::dense(sizes),
        })
    }

    pub fn from_params(sizes: &[usize], params: Vec<f32>) -> Result<Self, MlpError> {
        validate(sizes)?;
        let expected = parameter_count(sizes);
        if params.len() != expected {
            return Err(MlpError::ParameterCount {
                expected,
                found: params.len(),
            });
        }
        if params.iter().any(|p| !p.is_finite()) {
            return Err(MlpError::Structure("parameters must be finite".into()));
        }
        Ok(Self {
            sizes: sizes.to_vec(),
            params,
            structure: MlpStructure::dense(sizes),
        })
    }

    /// Glorot-style hidden layers. The output layer is scaled by
    /// `output_scale` so a fresh module starts close to neutral (outputs near
    /// zero) and every behaviour it later shows was learned.
    pub fn randomized(sizes: &[usize], rng: &mut Rng, output_scale: f32) -> Result<Self, MlpError> {
        let mut net = Self::zeroed(sizes)?;
        let layer_count = sizes.len() - 1;
        let mut offset = 0;
        for layer in 0..layer_count {
            let (inputs, outputs) = (sizes[layer], sizes[layer + 1]);
            let mut scale = (2.0 / (inputs + outputs) as f32).sqrt();
            if layer + 1 == layer_count {
                scale *= output_scale;
            }
            for weight in &mut net.params[offset..offset + inputs * outputs] {
                *weight = rng.gaussian() * scale;
            }
            // Biases stay zero.
            offset += inputs * outputs + outputs;
        }
        Ok(net)
    }

    pub fn sizes(&self) -> &[usize] {
        &self.sizes
    }

    pub fn input_len(&self) -> usize {
        self.sizes[0]
    }

    pub fn output_len(&self) -> usize {
        *self.sizes.last().expect("validated non-empty")
    }

    pub fn params(&self) -> &[f32] {
        &self.params
    }

    pub fn params_mut(&mut self) -> &mut [f32] {
        &mut self.params
    }

    pub fn structure(&self) -> &MlpStructure {
        &self.structure
    }

    pub fn set_structure(&mut self, structure: MlpStructure) -> Result<(), MlpError> {
        structure
            .validate(&self.sizes)
            .map_err(MlpError::Structure)?;
        self.structure = structure;
        Ok(())
    }

    /// Allocation-free inference. `input.len()` must equal `input_len()` and
    /// `output.len()` must equal `output_len()`.
    pub fn forward(&self, input: &[f32], output: &mut [f32]) {
        debug_assert_eq!(input.len(), self.input_len());
        debug_assert_eq!(output.len(), self.output_len());
        let mut current = [0.0f32; MAX_WIDTH];
        let mut next = [0.0f32; MAX_WIDTH];
        current[..input.len()].copy_from_slice(input);
        let mut offset = 0;
        for (layer, pair) in self.sizes.windows(2).enumerate() {
            let (inputs, outputs) = (pair[0], pair[1]);
            let weights = &self.params[offset..offset + inputs * outputs];
            let biases =
                &self.params[offset + inputs * outputs..offset + inputs * outputs + outputs];
            for row in 0..outputs {
                let mut sum = biases[row];
                for (column, (weight, value)) in weights[row * inputs..(row + 1) * inputs]
                    .iter()
                    .zip(&current[..inputs])
                    .enumerate()
                {
                    if self.structure.connections[layer][row][column]
                        && (layer != 0 || self.structure.inputs[column])
                    {
                        sum += weight * value;
                    }
                }
                next[row] = if layer < self.structure.hidden.len() {
                    self.structure.hidden[layer][row]
                } else {
                    Activation::Tanh
                }
                .apply(sum);
            }
            std::mem::swap(&mut current, &mut next);
            offset += inputs * outputs + outputs;
        }
        output.copy_from_slice(&current[..output.len()]);
    }

    /// Same arithmetic as [`forward`](Self::forward), but keeps every layer.
    /// Allocates; intended for explanation only.
    pub fn trace(&self, input: &[f32]) -> MlpTrace {
        let mut layers = vec![input.to_vec()];
        let mut offset = 0;
        for (layer, pair) in self.sizes.windows(2).enumerate() {
            let (inputs, outputs) = (pair[0], pair[1]);
            let previous = layers.last().expect("input layer present");
            let mut activations = Vec::with_capacity(outputs);
            for row in 0..outputs {
                let mut sum = self.params[offset + inputs * outputs + row];
                for column in 0..inputs {
                    if self.structure.connections[layer][row][column]
                        && (layer != 0 || self.structure.inputs[column])
                    {
                        sum += self.params[offset + row * inputs + column] * previous[column];
                    }
                }
                activations.push(
                    if layer < self.structure.hidden.len() {
                        self.structure.hidden[layer][row]
                    } else {
                        Activation::Tanh
                    }
                    .apply(sum),
                );
            }
            layers.push(activations);
            offset += inputs * outputs + outputs;
        }
        MlpTrace { layers }
    }

    /// `(weights[out][in], biases[out])` for each layer, for display/export.
    pub fn layers(&self) -> Vec<(Vec<Vec<f32>>, Vec<f32>)> {
        let mut layers = Vec::new();
        let mut offset = 0;
        for pair in self.sizes.windows(2) {
            let (inputs, outputs) = (pair[0], pair[1]);
            let weights = (0..outputs)
                .map(|row| self.params[offset + row * inputs..offset + (row + 1) * inputs].to_vec())
                .collect();
            let biases = self.params
                [offset + inputs * outputs..offset + inputs * outputs + outputs]
                .to_vec();
            layers.push((weights, biases));
            offset += inputs * outputs + outputs;
        }
        layers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structure_controls_inputs_edges_and_hidden_response() {
        use crate::learn::structure::{Activation, MlpStructure};
        let mut net = Mlp::from_params(&[2, 1, 1], vec![1.0, 100.0, 0.0, 1.0, 0.0]).unwrap();
        let original = net.params().to_vec();
        let mut structure = MlpStructure::dense(net.sizes());
        structure.inputs[1] = false;
        structure.hidden[0][0] = Activation::Softsign;
        net.set_structure(structure.clone()).unwrap();
        let mut out = [0.0];
        net.forward(&[3.0, 10.0], &mut out);
        assert_eq!(out[0], 0.75f32.tanh());
        assert_eq!(net.trace(&[3.0, 10.0]).layers.last().unwrap(), &out);
        assert_eq!(net.params(), original);
        structure.inputs[1] = true;
        structure.connections[0][0][1] = false;
        net.set_structure(structure).unwrap();
        net.forward(&[3.0, 10.0], &mut out);
        assert_eq!(out[0], 0.75f32.tanh());
        assert!((Activation::LeakyRelu.apply(-5.0) + 0.05).abs() < 1e-7);
    }

    #[test]
    fn malformed_structure_and_nonfinite_parameters_are_rejected() {
        use crate::learn::structure::MlpStructure;
        let mut net = Mlp::zeroed(&[2, 1]).unwrap();
        let mut structure = MlpStructure::dense(net.sizes());
        structure.inputs.pop();
        assert!(net.set_structure(structure).is_err());
        assert!(Mlp::from_params(&[1, 1], vec![f32::NAN, 0.0]).is_err());
    }

    #[test]
    fn matches_a_hand_computed_network() {
        // 2 -> 2 -> 1.  hidden0 = tanh(1*a + 2*b + 0.5), hidden1 = tanh(-a + 0.0*b)
        // out = tanh(0.5*h0 - 1.0*h1 + 0.1)
        let net = Mlp::from_params(
            &[2, 2, 1],
            vec![
                1.0, 2.0, -1.0, 0.0, // layer 0 weights
                0.5, 0.0, // layer 0 biases
                0.5, -1.0, // layer 1 weights
                0.1,  // layer 1 bias
            ],
        )
        .unwrap();
        let (a, b) = (0.3f32, -0.2f32);
        let h0 = (1.0 * a + 2.0 * b + 0.5).tanh();
        let h1 = (-a).tanh();
        let expected = (0.5 * h0 - 1.0 * h1 + 0.1).tanh();
        let mut out = [0.0];
        net.forward(&[a, b], &mut out);
        assert!((out[0] - expected).abs() < 1e-6);
    }

    #[test]
    fn trace_agrees_with_forward() {
        let mut rng = Rng::new(3);
        let net = Mlp::randomized(&[5, 7, 3], &mut rng, 1.0).unwrap();
        let input = [0.2, -0.4, 0.9, 0.0, 0.5];
        let mut out = [0.0; 3];
        net.forward(&input, &mut out);
        let trace = net.trace(&input);
        assert_eq!(trace.layers.len(), 3);
        assert_eq!(trace.layers[1].len(), 7);
        for (a, b) in out.iter().zip(&trace.layers[2]) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn parameter_count_matches_layout_and_shapes_are_validated() {
        assert_eq!(parameter_count(&[7, 8, 1]), 7 * 8 + 8 + 8 + 1);
        assert_eq!(
            Mlp::from_params(&[2, 1], vec![0.0; 2]),
            Err(MlpError::ParameterCount {
                expected: 3,
                found: 2
            })
        );
        assert_eq!(Mlp::zeroed(&[3]), Err(MlpError::TooFewLayers));
        assert_eq!(Mlp::zeroed(&[3, 0, 1]), Err(MlpError::EmptyLayer));
        assert_eq!(
            Mlp::zeroed(&[3, MAX_WIDTH + 1, 1]),
            Err(MlpError::LayerTooWide {
                width: MAX_WIDTH + 1
            })
        );
    }

    #[test]
    fn small_output_scale_starts_near_neutral() {
        let mut rng = Rng::new(9);
        let net = Mlp::randomized(&[6, 6, 1], &mut rng, 0.1).unwrap();
        let mut out = [0.0];
        net.forward(&[1.0, -1.0, 0.5, 2.0, 0.0, 1.0], &mut out);
        assert!(out[0].abs() < 0.3, "{}", out[0]);
    }

    #[test]
    fn layers_view_round_trips_the_flat_layout() {
        let net = Mlp::from_params(&[2, 2, 1], (0..9).map(|i| i as f32).collect()).unwrap();
        let layers = net.layers();
        assert_eq!(layers[0].0, vec![vec![0.0, 1.0], vec![2.0, 3.0]]);
        assert_eq!(layers[0].1, vec![4.0, 5.0]);
        assert_eq!(layers[1].0, vec![vec![6.0, 7.0]]);
        assert_eq!(layers[1].1, vec![8.0]);
    }
}
