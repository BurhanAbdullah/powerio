//! Solver-independent LinDist3Flow OPF results.
//!
//! Values follow formulation identities and physical table order, never a
//! solver adapter's column order. This lets a native or WASM solver return the
//! same public solution type after translating its primal vector.

use std::collections::BTreeMap;
use std::sync::Arc;

use powerio_core::Error;
use powerio_dist::MulticonductorNetwork;

use crate::diagnostics::codes;
use crate::instance::{LinDist3FlowNode, LinDist3FlowOpfInstance};
use crate::solution::{Producer, Residuals, Termination};

/// Physical primal values from a LinDist3Flow solve.
///
/// Node values follow `instance.topology().nodes`. Line values follow
/// `instance.topology().conductors`. Generator channels are flattened in
/// generator table order, then channel order. Source channels are flattened
/// in source table order, then terminal-map order. Voltage is squared volts;
/// all power quantities are watts or vars.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct LinDist3FlowOpfValues {
    pub terminal_voltage_magnitude_squared: Vec<f64>,
    pub line_active_power: Vec<f64>,
    pub line_reactive_power: Vec<f64>,
    pub generator_active_power: Vec<f64>,
    pub generator_reactive_power: Vec<f64>,
    pub source_active_power: Vec<f64>,
    pub source_reactive_power: Vec<f64>,
}

fn shape(what: &str, actual: usize, expected: usize) -> Result<(), Error> {
    if actual == expected {
        Ok(())
    } else {
        Err(Error::new(
            &codes::BUILD_SOLUTION_SHAPE_MISMATCH,
            format!("{what} carries {actual} values; expected {expected}"),
        ))
    }
}

fn duplicate(family: &str, identity: &str) -> Error {
    Error::new(
        &codes::BUILD_OPERATING_POINT_IDENTITY_UNKNOWN,
        format!("duplicate {family} identity `{identity}`"),
    )
}

#[derive(Clone, Debug)]
struct SolutionIndex {
    nodes: BTreeMap<(String, String), usize>,
    line_conductors: BTreeMap<(String, usize), usize>,
    generators: BTreeMap<String, (usize, usize)>,
    sources: BTreeMap<String, (usize, usize)>,
}

impl SolutionIndex {
    fn new(instance: &LinDist3FlowOpfInstance) -> Result<Self, Error> {
        let mut nodes = BTreeMap::new();
        for (position, node) in instance.topology().nodes.iter().enumerate() {
            let key = (node.bus.to_ascii_lowercase(), node.terminal.clone());
            if nodes.insert(key, position).is_some() {
                return Err(duplicate(
                    "terminal",
                    &format!("{}/{}", node.bus, node.terminal),
                ));
            }
        }

        let mut line_conductors = BTreeMap::new();
        for (position, conductor) in instance.topology().conductors.iter().enumerate() {
            let key = (conductor.line.clone(), conductor.conductor_position);
            if line_conductors.insert(key, position).is_some() {
                return Err(duplicate("line conductor", &conductor.line));
            }
        }

        let mut generators = BTreeMap::new();
        let mut offset = 0;
        for generator in instance.network().generators() {
            let channels = generator.p_nom.len();
            if generators
                .insert(generator.name.clone(), (offset, channels))
                .is_some()
            {
                return Err(duplicate("generator", &generator.name));
            }
            offset += channels;
        }

        let mut sources = BTreeMap::new();
        let mut offset = 0;
        for source in instance.network().sources() {
            let channels = source.terminal_map.len();
            if sources
                .insert(source.name.clone(), (offset, channels))
                .is_some()
            {
                return Err(duplicate("voltage source", &source.name));
            }
            offset += channels;
        }
        Ok(Self {
            nodes,
            line_conductors,
            generators,
            sources,
        })
    }
}

/// A solution of the fixed-reference LinDist3Flow OPF approximation.
#[derive(Clone, Debug)]
pub struct LinDist3FlowOpfSolution {
    instance: Arc<LinDist3FlowOpfInstance>,
    termination: Termination,
    residuals: Residuals,
    producer: Producer,
    values: LinDist3FlowOpfValues,
    objective: f64,
    index: SolutionIndex,
}

impl LinDist3FlowOpfSolution {
    pub const FORMULATION: &'static str = "lindist3flow";

    /// Construct a result in the formulation's documented physical order.
    ///
    /// # Errors
    /// Any primal column disagrees with the instance's node or device axes.
    pub fn new(
        instance: Arc<LinDist3FlowOpfInstance>,
        termination: Termination,
        values: LinDist3FlowOpfValues,
        objective: f64,
    ) -> Result<Self, Error> {
        let network = instance.network();
        let nodes = instance.topology().nodes.len();
        let conductors = instance.topology().conductors.len();
        let generators = network
            .generators()
            .iter()
            .map(|generator| generator.p_nom.len())
            .sum();
        let sources = network
            .sources()
            .iter()
            .map(|source| source.terminal_map.len())
            .sum();
        shape(
            "terminal squared voltage",
            values.terminal_voltage_magnitude_squared.len(),
            nodes,
        )?;
        shape(
            "line active power",
            values.line_active_power.len(),
            conductors,
        )?;
        shape(
            "line reactive power",
            values.line_reactive_power.len(),
            conductors,
        )?;
        shape(
            "generator active power",
            values.generator_active_power.len(),
            generators,
        )?;
        shape(
            "generator reactive power",
            values.generator_reactive_power.len(),
            generators,
        )?;
        shape(
            "source active power",
            values.source_active_power.len(),
            sources,
        )?;
        shape(
            "source reactive power",
            values.source_reactive_power.len(),
            sources,
        )?;
        let index = SolutionIndex::new(&instance)?;
        Ok(Self {
            instance,
            termination,
            residuals: Residuals::default(),
            producer: None,
            values,
            objective,
            index,
        })
    }

    #[must_use]
    pub const fn formulation(&self) -> &'static str {
        Self::FORMULATION
    }

    #[must_use]
    pub fn instance(&self) -> &LinDist3FlowOpfInstance {
        &self.instance
    }

    #[must_use]
    pub fn shared_instance(&self) -> Arc<LinDist3FlowOpfInstance> {
        Arc::clone(&self.instance)
    }

    #[must_use]
    pub fn network(&self) -> &MulticonductorNetwork {
        self.instance.network()
    }

    #[must_use]
    pub const fn termination(&self) -> &Termination {
        &self.termination
    }

    #[must_use]
    pub const fn residuals(&self) -> &Residuals {
        &self.residuals
    }

    #[must_use]
    pub fn producer(&self) -> Option<&str> {
        self.producer.as_deref()
    }

    #[must_use]
    pub const fn values(&self) -> &LinDist3FlowOpfValues {
        &self.values
    }

    #[must_use]
    pub const fn objective(&self) -> f64 {
        self.objective
    }

    #[must_use]
    pub fn with_producer(mut self, producer: impl Into<String>) -> Self {
        self.producer = Some(producer.into());
        self
    }

    #[must_use]
    pub const fn with_residuals(mut self, residuals: Residuals) -> Self {
        self.residuals = residuals;
        self
    }

    pub fn node_order(&self) -> impl ExactSizeIterator<Item = &LinDist3FlowNode> {
        self.instance.topology().nodes.iter()
    }

    #[must_use]
    pub fn terminal_voltage_magnitude_squared(&self, bus: &str, terminal: &str) -> Option<f64> {
        let position = self
            .index
            .nodes
            .get(&(bus.to_ascii_lowercase(), terminal.to_owned()))?;
        Some(self.values.terminal_voltage_magnitude_squared[*position])
    }

    #[must_use]
    pub fn terminal_voltage_magnitude(&self, bus: &str, terminal: &str) -> Option<f64> {
        let squared = self.terminal_voltage_magnitude_squared(bus, terminal)?;
        (squared >= 0.0).then(|| squared.sqrt())
    }

    #[must_use]
    pub fn line_active_power(&self, line: &str, conductor: usize) -> Option<f64> {
        Some(
            self.values.line_active_power[*self
                .index
                .line_conductors
                .get(&(line.to_owned(), conductor))?],
        )
    }

    #[must_use]
    pub fn line_reactive_power(&self, line: &str, conductor: usize) -> Option<f64> {
        Some(
            self.values.line_reactive_power[*self
                .index
                .line_conductors
                .get(&(line.to_owned(), conductor))?],
        )
    }

    fn channel_position(
        index: &BTreeMap<String, (usize, usize)>,
        identity: &str,
        channel: usize,
    ) -> Option<usize> {
        let (offset, count) = *index.get(identity)?;
        (channel < count).then_some(offset + channel)
    }

    #[must_use]
    pub fn generator_active_power(&self, generator: &str, channel: usize) -> Option<f64> {
        Some(
            self.values.generator_active_power
                [Self::channel_position(&self.index.generators, generator, channel)?],
        )
    }

    #[must_use]
    pub fn generator_reactive_power(&self, generator: &str, channel: usize) -> Option<f64> {
        Some(
            self.values.generator_reactive_power
                [Self::channel_position(&self.index.generators, generator, channel)?],
        )
    }

    #[must_use]
    pub fn source_active_power(&self, source: &str, channel: usize) -> Option<f64> {
        Some(
            self.values.source_active_power
                [Self::channel_position(&self.index.sources, source, channel)?],
        )
    }

    #[must_use]
    pub fn source_reactive_power(&self, source: &str, channel: usize) -> Option<f64> {
        Some(
            self.values.source_reactive_power
                [Self::channel_position(&self.index.sources, source, channel)?],
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::{LinDist3FlowBuildOptions, LinDist3FlowOpfInstance};
    use powerio_dist::{DistBus, DistLine, DistLineCode, MulticonductorNetwork, VoltageSource};

    use super::*;

    fn instance() -> Arc<LinDist3FlowOpfInstance> {
        let terminal = vec!["1".to_owned()];
        let mut network = MulticonductorNetwork::named("solution");
        network
            .buses_mut()
            .push(DistBus::new("source", terminal.clone()));
        network
            .buses_mut()
            .push(DistBus::new("load", terminal.clone()));
        network
            .line_codes_mut()
            .push(DistLineCode::new("one", vec![vec![0.1]], vec![vec![0.2]]));
        network.lines_mut().push(DistLine::new(
            "line",
            "source",
            "load",
            terminal.clone(),
            terminal.clone(),
            "one",
            1.0,
        ));
        network.sources_mut().push(VoltageSource::new(
            "grid",
            "source",
            terminal,
            vec![230.0],
            vec![0.0],
        ));
        Arc::new(
            LinDist3FlowOpfInstance::from_network(network, LinDist3FlowBuildOptions::default())
                .unwrap(),
        )
    }

    fn values() -> LinDist3FlowOpfValues {
        LinDist3FlowOpfValues {
            terminal_voltage_magnitude_squared: vec![230.0f64.powi(2), 228.0f64.powi(2)],
            line_active_power: vec![1_000.0],
            line_reactive_power: vec![200.0],
            generator_active_power: Vec::new(),
            generator_reactive_power: Vec::new(),
            source_active_power: vec![1_000.0],
            source_reactive_power: vec![200.0],
        }
    }

    #[test]
    fn result_is_keyed_by_formulation_identities() {
        let instance = instance();
        let solution = LinDist3FlowOpfSolution::new(
            Arc::clone(&instance),
            Termination::Converged,
            values(),
            0.3,
        )
        .unwrap()
        .with_producer("test-solver");
        assert_eq!(solution.formulation(), "lindist3flow");
        assert_eq!(solution.producer(), Some("test-solver"));
        assert!((solution.terminal_voltage_magnitude("load", "1").unwrap() - 228.0).abs() < 1e-12);
        assert!((solution.line_active_power("line", 0).unwrap() - 1_000.0).abs() < 1e-12);
        assert!((solution.source_reactive_power("grid", 0).unwrap() - 200.0).abs() < 1e-12);
        assert!(std::ptr::eq(solution.instance(), instance.as_ref()));
    }

    #[test]
    fn every_physical_axis_is_shape_checked() {
        let instance = instance();
        let mut wrong = values();
        wrong.line_active_power.push(0.0);
        assert!(
            LinDist3FlowOpfSolution::new(instance, Termination::Converged, wrong, 0.0).is_err()
        );
    }
}
