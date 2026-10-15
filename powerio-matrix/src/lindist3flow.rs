//! Solver-neutral affine coefficient oracles for LinDist3Flow.
//!
//! These functions expose the small dense blocks used when a solver adapter
//! assembles the formulation. They do not own variables, sparse row numbers,
//! or a solver model.

use std::collections::{BTreeMap, BTreeSet};

use num_complex::Complex64;
use powerio_prob::{LinDist3FlowNode, LinDist3FlowOpfInstance};

use crate::{Error, Result};

/// Affine closure `c + a w_phi + b w_psi` for one cross-voltage product.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct CrossVoltageCoefficients {
    pub constant: Complex64,
    pub coefficient_phi: Complex64,
    pub coefficient_psi: Complex64,
}

/// Real affine scalar `constant + coefficients' * w`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct AffineScalarCoefficients {
    pub constant: f64,
    pub coefficients: Vec<f64>,
}

/// Reference-frozen map from physical channel powers to terminal powers.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ConnectionPowerMap {
    /// Complex terminal-by-channel map.
    pub matrix: Vec<Vec<Complex64>>,
    pub real_part: Vec<Vec<f64>>,
    pub imag_part: Vec<Vec<f64>>,
    /// Reference voltage across each physical channel.
    pub reference_winding_voltage: Vec<Complex64>,
}

/// Real coefficient blocks in `w_child = w_parent - M p - N q`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct LineDropCoefficients {
    /// `M = 2 Re(conj(Z) .* Gamma)`.
    pub active: Vec<Vec<f64>>,
    /// `N = -2 Im(conj(Z) .* Gamma)`.
    pub reactive: Vec<Vec<f64>>,
}

/// One squared-voltage variable row in the solver-neutral network data.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct LinDist3FlowNodeData {
    pub node: LinDist3FlowNode,
    pub reference_magnitude: f64,
    pub reference_angle: f64,
    pub squared_voltage_min: Option<f64>,
    pub squared_voltage_max: Option<f64>,
    /// Fixed squared voltage for a voltage-source terminal.
    pub fixed_squared_voltage: Option<f64>,
}

/// One coupled line block in the solver-neutral network data.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct LinDist3FlowLineData {
    pub line: String,
    pub source_line_row: usize,
    pub parent_nodes: Vec<usize>,
    pub child_nodes: Vec<usize>,
    pub reversed: bool,
    pub drop: LineDropCoefficients,
    /// Per-conductor amperes; `None` means unbounded.
    pub current_limit: Vec<Option<f64>>,
    /// Per-conductor VA; `None` means unbounded.
    pub apparent_power_limit: Vec<Option<f64>>,
}

/// Numerical rows for the voltage, line-drop, and line SOC portion of L3F.
///
/// Device connection maps and nodal injection rows remain separate compiler
/// stages. Keeping this boundary explicit lets a Tellegen adapter consume the
/// line physics now without treating an incomplete solver model as complete.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct LinDist3FlowNetworkData {
    pub nodes: Vec<LinDist3FlowNodeData>,
    pub lines: Vec<LinDist3FlowLineData>,
}

fn invalid(reason: impl Into<String>) -> Error {
    Error::InvalidLinDist3FlowCoefficients {
        reason: reason.into(),
    }
}

fn valid_complex(value: Complex64) -> bool {
    value.re.is_finite() && value.im.is_finite()
}

fn valid_reference(value: Complex64) -> bool {
    valid_complex(value) && value.norm_sqr() > 0.0
}

fn validate_reference(reference: &[Complex64], label: &str) -> Result<()> {
    if reference.is_empty() {
        return Err(invalid(format!("{label} must not be empty")));
    }
    if let Some(position) = reference.iter().position(|value| !valid_reference(*value)) {
        return Err(invalid(format!(
            "{label} entry {position} must be finite and nonzero"
        )));
    }
    Ok(())
}

fn validate_real_row(row: &[f64], expected: usize, label: &str, position: usize) -> Result<()> {
    if row.len() != expected {
        return Err(invalid(format!(
            "{label} row {position} has length {}, expected {expected}",
            row.len()
        )));
    }
    if row.iter().any(|value| !value.is_finite()) {
        return Err(invalid(format!(
            "{label} row {position} contains a non-finite value"
        )));
    }
    Ok(())
}

fn squared_bound(value: Option<f64>, label: &str) -> Result<Option<f64>> {
    value
        .map(|value| {
            if !value.is_finite() || value < 0.0 {
                return Err(invalid(format!("{label} must be finite and nonnegative")));
            }
            Ok(value * value)
        })
        .transpose()
}

fn terminal_bounds(
    bus: &powerio_dist::DistBus,
    position: usize,
) -> Result<(Option<f64>, Option<f64>)> {
    let lower = if bus.v_min.is_some() {
        bus.v_min
    } else if let Some(values) = &bus.v_min_phase {
        if values.len() != bus.terminals.len() {
            return Err(invalid(format!(
                "bus `{}` phase voltage lower bounds have length {}, expected {}",
                bus.id,
                values.len(),
                bus.terminals.len()
            )));
        }
        Some(values[position])
    } else {
        None
    };
    let upper = if bus.v_max.is_some() {
        bus.v_max
    } else if let Some(values) = &bus.v_max_phase {
        if values.len() != bus.terminals.len() {
            return Err(invalid(format!(
                "bus `{}` phase voltage upper bounds have length {}, expected {}",
                bus.id,
                values.len(),
                bus.terminals.len()
            )));
        }
        Some(values[position])
    } else {
        None
    };
    Ok((
        squared_bound(lower, "voltage lower bound")?,
        squared_bound(upper, "voltage upper bound")?,
    ))
}

fn optional_ratings(
    values: Option<&[f64]>,
    conductors: usize,
    label: &str,
) -> Result<Vec<Option<f64>>> {
    let Some(values) = values else {
        return Ok(vec![None; conductors]);
    };
    if values.len() != conductors {
        return Err(invalid(format!(
            "{label} has length {}, expected {conductors}",
            values.len()
        )));
    }
    if let Some(position) = values
        .iter()
        .position(|value| !value.is_finite() || *value <= 0.0)
    {
        return Err(invalid(format!(
            "{label} entry {position} must be finite and positive"
        )));
    }
    Ok(values.iter().copied().map(Some).collect())
}

/// Form the fixed-angle first-order closure of `v_phi * conj(v_psi)`.
///
/// At the reference point it reproduces the product exactly. Away from that
/// point it is the first-order Taylor expansion of
/// `sqrt(w_phi w_psi) exp(j delta_theta)` with the angle difference fixed.
///
/// # Errors
/// Either reference phasor is zero or non-finite.
pub fn cross_voltage_coefficients(
    reference_phi: Complex64,
    reference_psi: Complex64,
) -> Result<CrossVoltageCoefficients> {
    if !valid_reference(reference_phi) || !valid_reference(reference_psi) {
        return Err(invalid(
            "cross-voltage reference phasors must be finite and nonzero",
        ));
    }
    let coefficient_phi = reference_psi.conj() / (2.0 * reference_phi.conj());
    let coefficient_psi = reference_phi / (2.0 * reference_psi);
    let constant = reference_phi * reference_psi.conj()
        - coefficient_phi * reference_phi.norm_sqr()
        - coefficient_psi * reference_psi.norm_sqr();
    Ok(CrossVoltageCoefficients {
        constant,
        coefficient_phi,
        coefficient_psi,
    })
}

/// Evaluate a cross-voltage affine closure at two squared magnitudes.
#[must_use]
pub fn evaluate_cross_voltage(
    coefficients: &CrossVoltageCoefficients,
    w_phi: f64,
    w_psi: f64,
) -> Complex64 {
    coefficients.constant
        + coefficients.coefficient_phi * w_phi
        + coefficients.coefficient_psi * w_psi
}

/// Form real affine coefficients for the squared winding voltage `|d v|^2`.
///
/// # Errors
/// The incidence row and reference have different or zero length, contain a
/// non-finite value, or a reference phasor is zero.
pub fn winding_voltage_coefficients(
    incidence: &[f64],
    reference: &[Complex64],
) -> Result<AffineScalarCoefficients> {
    validate_reference(reference, "winding reference")?;
    if incidence.len() != reference.len() {
        return Err(invalid(format!(
            "winding incidence has length {}, expected {}",
            incidence.len(),
            reference.len()
        )));
    }
    if incidence.iter().any(|value| !value.is_finite()) {
        return Err(invalid("winding incidence contains a non-finite value"));
    }
    let mut constant = 0.0;
    let mut coefficients = vec![0.0; reference.len()];
    for phi in 0..reference.len() {
        for psi in 0..reference.len() {
            let scale = incidence[phi] * incidence[psi];
            if scale.abs() <= f64::EPSILON {
                continue;
            }
            let cross = cross_voltage_coefficients(reference[phi], reference[psi])?;
            constant += scale * cross.constant.re;
            coefficients[phi] += scale * cross.coefficient_phi.re;
            coefficients[psi] += scale * cross.coefficient_psi.re;
        }
    }
    Ok(AffineScalarCoefficients {
        constant,
        coefficients,
    })
}

/// Evaluate a real affine scalar.
///
/// # Errors
/// The coefficient and variable vectors differ in length.
pub fn evaluate_affine(coefficients: &AffineScalarCoefficients, values: &[f64]) -> Result<f64> {
    if coefficients.coefficients.len() != values.len() {
        return Err(invalid(format!(
            "affine value vector has length {}, expected {}",
            values.len(),
            coefficients.coefficients.len()
        )));
    }
    Ok(coefficients.constant
        + coefficients
            .coefficients
            .iter()
            .zip(values)
            .map(|(coefficient, value)| coefficient * value)
            .sum::<f64>())
}

/// Construct `H = diag(vbar) D' diag(D vbar)^-1`.
///
/// A channel power `s_channel` maps to terminal powers as
/// `s_terminal = H s_channel`; the split is frozen at the reference phasors.
///
/// # Errors
/// The incidence matrix is empty, ragged, non-finite, has the wrong terminal
/// arity, or produces a zero reference winding voltage.
pub fn connection_power_map(
    incidence: &[Vec<f64>],
    reference: &[Complex64],
) -> Result<ConnectionPowerMap> {
    validate_reference(reference, "connection reference")?;
    if incidence.is_empty() {
        return Err(invalid(
            "connection incidence must have at least one channel",
        ));
    }
    for (position, row) in incidence.iter().enumerate() {
        validate_real_row(row, reference.len(), "connection incidence", position)?;
    }
    let reference_winding_voltage = incidence
        .iter()
        .map(|row| {
            row.iter()
                .zip(reference)
                .map(|(entry, voltage)| *voltage * *entry)
                .sum::<Complex64>()
        })
        .collect::<Vec<_>>();
    if let Some(position) = reference_winding_voltage
        .iter()
        .position(|value| !valid_reference(*value))
    {
        return Err(invalid(format!(
            "connection channel {position} has a zero or non-finite reference winding voltage"
        )));
    }

    let mut matrix = vec![vec![Complex64::new(0.0, 0.0); incidence.len()]; reference.len()];
    for terminal in 0..reference.len() {
        for channel in 0..incidence.len() {
            matrix[terminal][channel] = reference[terminal] * incidence[channel][terminal]
                / reference_winding_voltage[channel];
        }
    }
    let real_part = matrix
        .iter()
        .map(|row| row.iter().map(|value| value.re).collect())
        .collect();
    let imag_part = matrix
        .iter()
        .map(|row| row.iter().map(|value| value.im).collect())
        .collect();
    Ok(ConnectionPowerMap {
        matrix,
        real_part,
        imag_part,
        reference_winding_voltage,
    })
}

/// Construct the line voltage-drop blocks for a complex series impedance.
///
/// `Gamma[phi, psi] = vbar_phi / vbar_psi`,
/// `M = 2 Re(conj(Z) .* Gamma)`, and
/// `N = -2 Im(conj(Z) .* Gamma)`.
///
/// # Errors
/// `Z` is not finite and square with the reference arity, or a reference
/// phasor is zero or non-finite.
pub fn line_drop_coefficients(
    impedance: &[Vec<Complex64>],
    reference_from: &[Complex64],
) -> Result<LineDropCoefficients> {
    validate_reference(reference_from, "line reference")?;
    if impedance.len() != reference_from.len() {
        return Err(invalid(format!(
            "line impedance has {} rows, expected {}",
            impedance.len(),
            reference_from.len()
        )));
    }
    let n = reference_from.len();
    for (position, row) in impedance.iter().enumerate() {
        if row.len() != n {
            return Err(invalid(format!(
                "line impedance row {position} has length {}, expected {n}",
                row.len()
            )));
        }
        if row.iter().any(|value| !valid_complex(*value)) {
            return Err(invalid(format!(
                "line impedance row {position} contains a non-finite value"
            )));
        }
    }
    let mut active = vec![vec![0.0; n]; n];
    let mut reactive = vec![vec![0.0; n]; n];
    for phi in 0..n {
        for psi in 0..n {
            let value = impedance[phi][psi].conj() * reference_from[phi] / reference_from[psi];
            active[phi][psi] = 2.0 * value.re;
            reactive[phi][psi] = -2.0 * value.im;
        }
    }
    Ok(LineDropCoefficients { active, reactive })
}

/// Prepare the voltage, coupled line-drop, and line-limit rows of an L3F
/// instance in SI units.
///
/// The line relation is `w_child = w_parent - M p - N q`. A solver adapter can
/// impose each stated current limit as the native rotated cone
/// `p² + q² <= w I_max²` at both endpoints, and each apparent-power limit as
/// `p² + q² <= S_max²`.
///
/// # Errors
/// A topology identity cannot be resolved, a line block does not align with
/// its terminal maps or linecode, or a voltage/limit/coefficient is invalid.
#[allow(clippy::too_many_lines)]
pub fn build_lindist3flow_network_data(
    instance: &LinDist3FlowOpfInstance,
) -> Result<LinDist3FlowNetworkData> {
    let network = instance.network();
    let mut node_positions = BTreeMap::new();
    for (position, node) in instance.topology().nodes.iter().enumerate() {
        node_positions.insert(
            (node.bus.to_ascii_lowercase(), node.terminal.clone()),
            position,
        );
    }
    let source_nodes = network
        .sources()
        .iter()
        .flat_map(|source| {
            source
                .terminal_map
                .iter()
                .map(move |terminal| (source.bus.to_ascii_lowercase(), terminal.clone()))
        })
        .collect::<BTreeSet<_>>();
    let buses = network
        .buses()
        .iter()
        .map(|bus| (bus.id.to_ascii_lowercase(), bus))
        .collect::<BTreeMap<_, _>>();

    let mut nodes = Vec::with_capacity(instance.topology().nodes.len());
    for node in &instance.topology().nodes {
        let reference = instance
            .reference()
            .voltage(&node.bus, &node.terminal)
            .ok_or_else(|| {
                invalid(format!(
                    "reference has no voltage for `{}/{}`",
                    node.bus, node.terminal
                ))
            })?;
        let bus = buses
            .get(&node.bus.to_ascii_lowercase())
            .ok_or_else(|| invalid(format!("topology names unknown bus `{}`", node.bus)))?;
        let terminal_position = bus
            .terminals
            .iter()
            .position(|terminal| terminal == &node.terminal)
            .ok_or_else(|| {
                invalid(format!(
                    "topology names unknown terminal `{}/{}`",
                    node.bus, node.terminal
                ))
            })?;
        let voltage_bounds_selected = instance
            .base_instance()
            .constraints()
            .terminal_voltage_bounds
            .selects(&bus.id);
        let (squared_voltage_min, squared_voltage_max) = if voltage_bounds_selected {
            terminal_bounds(bus, terminal_position)?
        } else {
            (None, None)
        };
        if squared_voltage_min
            .zip(squared_voltage_max)
            .is_some_and(|(lower, upper)| lower > upper)
        {
            return Err(invalid(format!(
                "bus `{}` terminal `{}` has an inverted voltage interval",
                node.bus, node.terminal
            )));
        }
        let fixed_squared_voltage = source_nodes
            .contains(&(node.bus.to_ascii_lowercase(), node.terminal.clone()))
            .then_some(reference.magnitude * reference.magnitude);
        nodes.push(LinDist3FlowNodeData {
            node: node.clone(),
            reference_magnitude: reference.magnitude,
            reference_angle: reference.angle,
            squared_voltage_min,
            squared_voltage_max,
            fixed_squared_voltage,
        });
    }

    let mut grouped = BTreeMap::<usize, Vec<_>>::new();
    for conductor in &instance.topology().conductors {
        grouped
            .entry(conductor.source_line_row)
            .or_default()
            .push(conductor);
    }
    let mut lines = Vec::with_capacity(grouped.len());
    for (line_row, mut conductors) in grouped {
        let line = network
            .lines()
            .get(line_row)
            .ok_or_else(|| invalid(format!("topology names unknown line row {line_row}")))?;
        conductors.sort_by_key(|conductor| conductor.conductor_position);
        if conductors.len() != line.terminal_map_from.len()
            || conductors
                .iter()
                .enumerate()
                .any(|(position, conductor)| conductor.conductor_position != position)
        {
            return Err(invalid(format!(
                "line `{}` topology does not contain one row per conductor",
                line.name
            )));
        }
        let reversed = conductors[0].reversed;
        if conductors
            .iter()
            .any(|conductor| conductor.reversed != reversed)
        {
            return Err(invalid(format!(
                "line `{}` has inconsistent conductor orientation",
                line.name
            )));
        }
        let parent_nodes = conductors
            .iter()
            .map(|conductor| {
                node_positions
                    .get(&(
                        conductor.parent.bus.to_ascii_lowercase(),
                        conductor.parent.terminal.clone(),
                    ))
                    .copied()
                    .ok_or_else(|| {
                        invalid(format!(
                            "line `{}` parent terminal is absent from the topology",
                            line.name
                        ))
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        let child_nodes = conductors
            .iter()
            .map(|conductor| {
                node_positions
                    .get(&(
                        conductor.child.bus.to_ascii_lowercase(),
                        conductor.child.terminal.clone(),
                    ))
                    .copied()
                    .ok_or_else(|| {
                        invalid(format!(
                            "line `{}` child terminal is absent from the topology",
                            line.name
                        ))
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        let code = network.linecode(&line.linecode).ok_or_else(|| {
            invalid(format!(
                "line `{}` names missing linecode `{}`",
                line.name, line.linecode
            ))
        })?;
        let n = conductors.len();
        if code.r_series.len() != n || code.x_series.len() != n {
            return Err(invalid(format!(
                "linecode `{}` does not have {n} series rows",
                code.name
            )));
        }
        let mut impedance = Vec::with_capacity(n);
        for row in 0..n {
            if code.r_series[row].len() != n || code.x_series[row].len() != n {
                return Err(invalid(format!(
                    "linecode `{}` series row {row} does not have {n} entries",
                    code.name
                )));
            }
            impedance.push(
                (0..n)
                    .map(|column| {
                        Complex64::new(
                            code.r_series[row][column] * line.length,
                            code.x_series[row][column] * line.length,
                        )
                    })
                    .collect(),
            );
        }
        let reference_from = parent_nodes
            .iter()
            .map(|&node| {
                Complex64::from_polar(nodes[node].reference_magnitude, nodes[node].reference_angle)
            })
            .collect::<Vec<_>>();
        let drop = line_drop_coefficients(&impedance, &reference_from)?;
        let limits_selected = instance
            .base_instance()
            .constraints()
            .conductor_limits
            .selects(&line.name);
        let current_limit = if limits_selected {
            optional_ratings(
                line.i_max.as_deref().or(code.i_max.as_deref()),
                n,
                "line current limit",
            )?
        } else {
            vec![None; n]
        };
        let apparent_power_limit = if limits_selected {
            optional_ratings(
                line.s_max.as_deref().or(code.s_max.as_deref()),
                n,
                "line apparent-power limit",
            )?
        } else {
            vec![None; n]
        };
        lines.push(LinDist3FlowLineData {
            line: line.name.clone(),
            source_line_row: line_row,
            parent_nodes,
            child_nodes,
            reversed,
            drop,
            current_limit,
            apparent_power_limit,
        });
    }
    Ok(LinDist3FlowNetworkData { nodes, lines })
}

#[cfg(test)]
mod tests {
    use std::f64::consts::PI;

    use approx::assert_relative_eq;
    use powerio_dist::{DistBus, DistLine, DistLineCode, MulticonductorNetwork, VoltageSource};
    use powerio_prob::{LinDist3FlowBuildOptions, LinDist3FlowOpfInstance};

    use super::*;

    #[test]
    fn cross_voltage_closure_is_exact_at_its_reference() {
        let left = Complex64::from_polar(230.0, 0.13);
        let right = Complex64::from_polar(221.0, -2.01);
        let coefficients = cross_voltage_coefficients(left, right).unwrap();
        let value = evaluate_cross_voltage(&coefficients, left.norm_sqr(), right.norm_sqr());

        assert_relative_eq!(value.re, (left * right.conj()).re, epsilon = 1e-10);
        assert_relative_eq!(value.im, (left * right.conj()).im, epsilon = 1e-10);
    }

    #[test]
    fn winding_closure_reproduces_a_line_to_line_voltage() {
        let reference = [
            Complex64::from_polar(230.0, 0.0),
            Complex64::from_polar(230.0, -2.0 * PI / 3.0),
        ];
        let coefficients = winding_voltage_coefficients(&[1.0, -1.0], &reference).unwrap();
        let value = evaluate_affine(
            &coefficients,
            &[reference[0].norm_sqr(), reference[1].norm_sqr()],
        )
        .unwrap();

        assert_relative_eq!(
            value,
            (reference[0] - reference[1]).norm_sqr(),
            epsilon = 1e-9
        );
    }

    #[test]
    fn identity_connection_maps_each_channel_to_its_terminal() {
        let reference = [
            Complex64::from_polar(1.0, 0.0),
            Complex64::from_polar(1.0, -2.0 * PI / 3.0),
        ];
        let map = connection_power_map(&[vec![1.0, 0.0], vec![0.0, 1.0]], &reference).unwrap();

        assert_relative_eq!(map.matrix[0][0].re, 1.0, epsilon = 1e-12);
        assert_relative_eq!(map.matrix[1][1].re, 1.0, epsilon = 1e-12);
        assert_relative_eq!(map.matrix[0][1].norm(), 0.0, epsilon = 1e-12);
        assert_relative_eq!(map.matrix[1][0].norm(), 0.0, epsilon = 1e-12);
    }

    #[test]
    fn line_drop_uses_the_reference_phase_ratios() {
        let z = vec![
            vec![Complex64::new(0.1, 0.2), Complex64::new(0.03, 0.04)],
            vec![Complex64::new(0.03, 0.04), Complex64::new(0.1, 0.2)],
        ];
        let reference = [
            Complex64::from_polar(1.0, 0.0),
            Complex64::from_polar(1.0, -2.0 * PI / 3.0),
        ];
        let coefficients = line_drop_coefficients(&z, &reference).unwrap();

        assert_relative_eq!(coefficients.active[0][0], 0.2, epsilon = 1e-12);
        assert_relative_eq!(coefficients.reactive[0][0], 0.4, epsilon = 1e-12);
        let expected = z[0][1].conj() * reference[0] / reference[1];
        assert_relative_eq!(
            coefficients.active[0][1],
            2.0 * expected.re,
            epsilon = 1e-12
        );
        assert_relative_eq!(
            coefficients.reactive[0][1],
            -2.0 * expected.im,
            epsilon = 1e-12
        );
    }

    #[test]
    fn invalid_coefficient_operands_are_refused() {
        assert!(
            cross_voltage_coefficients(Complex64::new(0.0, 0.0), Complex64::new(1.0, 0.0)).is_err()
        );
        assert!(connection_power_map(&[vec![1.0, -1.0]], &[Complex64::new(1.0, 0.0)]).is_err());
        assert!(
            line_drop_coefficients(
                &[vec![Complex64::new(1.0, 0.0)]],
                &[Complex64::new(1.0, 0.0), Complex64::new(1.0, 0.0),]
            )
            .is_err()
        );
    }

    #[test]
    fn network_data_keeps_si_scaling_orientation_bounds_and_ratings() {
        let terminals = vec!["1".to_owned(), "2".to_owned()];
        let mut network = MulticonductorNetwork::named("line-data");
        let mut source_bus = DistBus::new("source", terminals.clone());
        source_bus.v_min = Some(220.0);
        source_bus.v_max = Some(240.0);
        network.buses_mut().push(source_bus);
        network
            .buses_mut()
            .push(DistBus::new("load", terminals.clone()));
        let mut code = DistLineCode::new(
            "two-phase",
            vec![vec![0.1, 0.0], vec![0.0, 0.1]],
            vec![vec![0.2, 0.0], vec![0.0, 0.2]],
        );
        code.i_max = Some(vec![100.0, 101.0]);
        code.s_max = Some(vec![10_000.0, 11_000.0]);
        network.line_codes_mut().push(code);
        let mut line = DistLine::new(
            "line",
            "load",
            "source",
            terminals.clone(),
            terminals.clone(),
            "two-phase",
            10.0,
        );
        line.i_max = Some(vec![90.0, 91.0]);
        network.lines_mut().push(line);
        network.sources_mut().push(VoltageSource::new(
            "grid",
            "source",
            terminals,
            vec![230.0, 230.0],
            vec![0.0, -2.0 * PI / 3.0],
        ));
        let instance =
            LinDist3FlowOpfInstance::from_network(network, LinDist3FlowBuildOptions::default())
                .unwrap();

        let data = build_lindist3flow_network_data(&instance).unwrap();

        assert_eq!(data.nodes.len(), 4);
        assert_relative_eq!(
            data.nodes[0].fixed_squared_voltage.unwrap(),
            230.0_f64.powi(2),
            epsilon = 1e-12
        );
        assert_relative_eq!(
            data.nodes[0].squared_voltage_min.unwrap(),
            220.0_f64.powi(2),
            epsilon = 1e-12
        );
        assert!(data.nodes[2].fixed_squared_voltage.is_none());
        assert_eq!(data.lines.len(), 1);
        let line = &data.lines[0];
        assert!(line.reversed);
        assert_eq!(line.parent_nodes, [0, 1]);
        assert_eq!(line.child_nodes, [2, 3]);
        assert_eq!(line.current_limit, [Some(90.0), Some(91.0)]);
        assert_eq!(line.apparent_power_limit, [Some(10_000.0), Some(11_000.0)]);
        assert_relative_eq!(line.drop.active[0][0], 2.0, epsilon = 1e-12);
        assert_relative_eq!(line.drop.reactive[0][0], 4.0, epsilon = 1e-12);
    }
}
