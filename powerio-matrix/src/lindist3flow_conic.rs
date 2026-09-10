//! Canonical solver-neutral conic assembly for LinDist3Flow.
//!
//! The output deliberately describes variables, affine equalities, bounds,
//! and cone arguments rather than using any solver crate's model objects.
//! A Clarabel, ECOS, or other conic adapter can therefore own the final sparse
//! matrix convention without leaking solver state into PowerIO.

use std::collections::BTreeMap;

use powerio_prob::{LinDist3FlowOpfInstance, LinDist3FlowOpfValues};

use crate::{
    Error, LinDist3FlowAffineExpression, LinDist3FlowBalanceEquation, LinDist3FlowPreparation,
    LinDist3FlowVariable, Result, build_lindist3flow_preparation,
};

/// Stable semantic identity of one scalar decision variable.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum LinDist3FlowDecisionVariable {
    SquaredVoltage { node: usize },
    Power(LinDist3FlowVariable),
}

/// One variable column, including its native bound and objective data.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct LinDist3FlowVariableData {
    pub variable: LinDist3FlowDecisionVariable,
    pub lower: Option<f64>,
    pub upper: Option<f64>,
    pub objective_coefficient: f64,
}

/// One coefficient in a canonical affine expression.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct LinDist3FlowLinearTerm {
    pub column: usize,
    pub coefficient: f64,
}

/// `constant + terms' * x` in canonical variable order.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct LinDist3FlowLinearExpression {
    pub constant: f64,
    pub terms: Vec<LinDist3FlowLinearTerm>,
}

/// Physical origin of one equality constrained to zero.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LinDist3FlowEqualityOrigin {
    LineDrop { line: usize, conductor: usize },
    ActiveBalance { node: usize },
    ReactiveBalance { node: usize },
}

/// One affine equality `expression == 0`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct LinDist3FlowEquality {
    pub origin: LinDist3FlowEqualityOrigin,
    pub expression: LinDist3FlowLinearExpression,
}

/// Physical origin and meaning of one cone row block.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LinDist3FlowConeOrigin {
    LineApparentPower {
        line: usize,
        conductor: usize,
    },
    LineCurrent {
        line: usize,
        conductor: usize,
        node: usize,
    },
    GeneratorApparentPower {
        generator: usize,
        channel: usize,
    },
    GeneratorCurrent {
        generator: usize,
        channel: usize,
    },
}

/// A standard or rotated second-order cone over affine arguments.
///
/// Standard arguments mean `(t, x...)` with `t >= ||x||₂`. Rotated
/// arguments mean `(u, v, x...)` with `u, v >= 0` and
/// `2 u v >= ||x||₂²`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LinDist3FlowCone {
    SecondOrder {
        origin: LinDist3FlowConeOrigin,
        arguments: Vec<LinDist3FlowLinearExpression>,
    },
    RotatedSecondOrder {
        origin: LinDist3FlowConeOrigin,
        arguments: Vec<LinDist3FlowLinearExpression>,
    },
}

/// Complete conic program structure in stable PowerIO variable order.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct LinDist3FlowConicProblem {
    pub preparation: LinDist3FlowPreparation,
    pub variables: Vec<LinDist3FlowVariableData>,
    pub equalities: Vec<LinDist3FlowEquality>,
    pub cones: Vec<LinDist3FlowCone>,
}

fn invalid(reason: impl Into<String>) -> Error {
    Error::InvalidLinDist3FlowCoefficients {
        reason: reason.into(),
    }
}

fn constant(value: f64) -> LinDist3FlowLinearExpression {
    LinDist3FlowLinearExpression {
        constant: value,
        terms: Vec::new(),
    }
}

fn column(column: usize, coefficient: f64) -> LinDist3FlowLinearExpression {
    LinDist3FlowLinearExpression {
        constant: 0.0,
        terms: vec![LinDist3FlowLinearTerm {
            column,
            coefficient,
        }],
    }
}

fn normalized_expression(
    constant: f64,
    terms: impl IntoIterator<Item = (usize, f64)>,
) -> Result<LinDist3FlowLinearExpression> {
    if !constant.is_finite() {
        return Err(invalid(
            "a conic affine expression has a non-finite constant",
        ));
    }
    let mut combined = BTreeMap::<usize, f64>::new();
    for (column, coefficient) in terms {
        if !coefficient.is_finite() {
            return Err(invalid(format!(
                "a conic affine expression has a non-finite coefficient at column {column}"
            )));
        }
        *combined.entry(column).or_default() += coefficient;
    }
    Ok(LinDist3FlowLinearExpression {
        constant,
        terms: combined
            .into_iter()
            .filter_map(|(column, coefficient)| {
                (coefficient.abs() > f64::EPSILON).then_some(LinDist3FlowLinearTerm {
                    column,
                    coefficient,
                })
            })
            .collect(),
    })
}

fn voltage_expression(
    expression: &LinDist3FlowAffineExpression,
    voltage_columns: &[usize],
) -> Result<LinDist3FlowLinearExpression> {
    normalized_expression(
        expression.constant,
        expression
            .voltage_terms
            .iter()
            .map(|term| (voltage_columns[term.node], term.coefficient)),
    )
}

fn balance_expression(
    equation: &LinDist3FlowBalanceEquation,
    voltage_columns: &[usize],
    power_columns: &BTreeMap<LinDist3FlowVariable, usize>,
) -> Result<LinDist3FlowLinearExpression> {
    let voltage_terms = equation
        .affine
        .voltage_terms
        .iter()
        .map(|term| (voltage_columns[term.node], term.coefficient));
    let power_terms = equation.variable_terms.iter().map(|term| {
        power_columns
            .get(&term.variable)
            .copied()
            .map(|column| (column, term.coefficient))
            .ok_or_else(|| {
                invalid(format!(
                    "nodal balance references unregistered variable {:?}",
                    term.variable
                ))
            })
    });
    normalized_expression(
        equation.affine.constant,
        voltage_terms.chain(power_terms.collect::<Result<Vec<_>>>()?),
    )
}

fn push_power_variable(
    variables: &mut Vec<LinDist3FlowVariableData>,
    power_columns: &mut BTreeMap<LinDist3FlowVariable, usize>,
    variable: LinDist3FlowVariable,
    lower: Option<f64>,
    upper: Option<f64>,
    objective_coefficient: f64,
) -> Result<()> {
    if lower.is_some_and(|value| !value.is_finite())
        || upper.is_some_and(|value| !value.is_finite())
        || !objective_coefficient.is_finite()
    {
        return Err(invalid(format!(
            "variable {variable:?} has non-finite bound or objective data"
        )));
    }
    if lower.zip(upper).is_some_and(|(lower, upper)| lower > upper) {
        return Err(invalid(format!(
            "variable {variable:?} has inverted bounds"
        )));
    }
    let column = variables.len();
    if power_columns.insert(variable.clone(), column).is_some() {
        return Err(invalid(format!(
            "variable {variable:?} was registered more than once"
        )));
    }
    variables.push(LinDist3FlowVariableData {
        variable: LinDist3FlowDecisionVariable::Power(variable),
        lower,
        upper,
        objective_coefficient,
    });
    Ok(())
}

fn power_column(
    columns: &BTreeMap<LinDist3FlowVariable, usize>,
    variable: &LinDist3FlowVariable,
) -> Result<usize> {
    columns
        .get(variable)
        .copied()
        .ok_or_else(|| invalid(format!("missing conic variable {variable:?}")))
}

/// Assemble the complete supported LinDist3Flow slice as a canonical conic
/// program.
///
/// Variable bounds remain native bounds. Equalities are normalized to zero.
/// Apparent-power limits use standard SOCs. Current limits use the exact
/// rotated representation `(w, I_max² / 2, p, q) in K_r`; line limits are
/// enforced against the voltage at both endpoints.
///
/// # Errors
/// As [`build_lindist3flow_preparation`], or an internal semantic variable is
/// absent, duplicated, or has non-finite numerical data.
#[allow(clippy::too_many_lines)]
pub fn build_lindist3flow_conic_problem(
    instance: &LinDist3FlowOpfInstance,
) -> Result<LinDist3FlowConicProblem> {
    let preparation = build_lindist3flow_preparation(instance)?;
    let mut variables = Vec::new();
    let mut voltage_columns = Vec::with_capacity(preparation.network.nodes.len());
    for (node, data) in preparation.network.nodes.iter().enumerate() {
        let column = variables.len();
        voltage_columns.push(column);
        let (lower, upper) = if let Some(fixed) = data.fixed_squared_voltage {
            if data
                .squared_voltage_min
                .is_some_and(|minimum| fixed < minimum)
                || data
                    .squared_voltage_max
                    .is_some_and(|maximum| fixed > maximum)
            {
                return Err(invalid(format!(
                    "fixed voltage at node {node} is outside its selected voltage bounds"
                )));
            }
            (Some(fixed), Some(fixed))
        } else {
            (data.squared_voltage_min, data.squared_voltage_max)
        };
        variables.push(LinDist3FlowVariableData {
            variable: LinDist3FlowDecisionVariable::SquaredVoltage { node },
            lower,
            upper,
            objective_coefficient: 0.0,
        });
    }

    let mut power_columns = BTreeMap::new();
    for (line, data) in preparation.network.lines.iter().enumerate() {
        for conductor in 0..data.parent_nodes.len() {
            push_power_variable(
                &mut variables,
                &mut power_columns,
                LinDist3FlowVariable::LineActive { line, conductor },
                None,
                None,
                0.0,
            )?;
            push_power_variable(
                &mut variables,
                &mut power_columns,
                LinDist3FlowVariable::LineReactive { line, conductor },
                None,
                None,
                0.0,
            )?;
        }
    }
    for (generator, data) in preparation.devices.generators.iter().enumerate() {
        for (channel, channel_data) in data.channels.iter().enumerate() {
            push_power_variable(
                &mut variables,
                &mut power_columns,
                LinDist3FlowVariable::GeneratorActive { generator, channel },
                channel_data.active_min,
                channel_data.active_max,
                channel_data.active_objective_coefficient,
            )?;
            push_power_variable(
                &mut variables,
                &mut power_columns,
                LinDist3FlowVariable::GeneratorReactive { generator, channel },
                channel_data.reactive_min,
                channel_data.reactive_max,
                0.0,
            )?;
        }
    }
    for (source, data) in preparation.devices.sources.iter().enumerate() {
        for (channel, &objective) in data.active_objective_coefficient.iter().enumerate() {
            push_power_variable(
                &mut variables,
                &mut power_columns,
                LinDist3FlowVariable::SourceActive { source, channel },
                None,
                None,
                objective,
            )?;
            push_power_variable(
                &mut variables,
                &mut power_columns,
                LinDist3FlowVariable::SourceReactive { source, channel },
                None,
                None,
                0.0,
            )?;
        }
    }

    let mut equalities = Vec::new();
    for (line, data) in preparation.network.lines.iter().enumerate() {
        for conductor in 0..data.parent_nodes.len() {
            let terms = [
                (voltage_columns[data.child_nodes[conductor]], 1.0),
                (voltage_columns[data.parent_nodes[conductor]], -1.0),
            ]
            .into_iter()
            .chain(
                data.drop.active[conductor]
                    .iter()
                    .enumerate()
                    .map(|(other, &coefficient)| {
                        let column = power_column(
                            &power_columns,
                            &LinDist3FlowVariable::LineActive {
                                line,
                                conductor: other,
                            },
                        );
                        column.map(|column| (column, coefficient))
                    })
                    .collect::<Result<Vec<_>>>()?,
            )
            .chain(
                data.drop.reactive[conductor]
                    .iter()
                    .enumerate()
                    .map(|(other, &coefficient)| {
                        let column = power_column(
                            &power_columns,
                            &LinDist3FlowVariable::LineReactive {
                                line,
                                conductor: other,
                            },
                        );
                        column.map(|column| (column, coefficient))
                    })
                    .collect::<Result<Vec<_>>>()?,
            );
            equalities.push(LinDist3FlowEquality {
                origin: LinDist3FlowEqualityOrigin::LineDrop { line, conductor },
                expression: normalized_expression(0.0, terms)?,
            });
        }
    }
    for balance in &preparation.devices.balances {
        equalities.push(LinDist3FlowEquality {
            origin: LinDist3FlowEqualityOrigin::ActiveBalance { node: balance.node },
            expression: balance_expression(&balance.active, &voltage_columns, &power_columns)?,
        });
        equalities.push(LinDist3FlowEquality {
            origin: LinDist3FlowEqualityOrigin::ReactiveBalance { node: balance.node },
            expression: balance_expression(&balance.reactive, &voltage_columns, &power_columns)?,
        });
    }

    let mut cones = Vec::new();
    for (line, data) in preparation.network.lines.iter().enumerate() {
        for conductor in 0..data.parent_nodes.len() {
            let active = power_column(
                &power_columns,
                &LinDist3FlowVariable::LineActive { line, conductor },
            )?;
            let reactive = power_column(
                &power_columns,
                &LinDist3FlowVariable::LineReactive { line, conductor },
            )?;
            if let Some(limit) = data.apparent_power_limit[conductor] {
                cones.push(LinDist3FlowCone::SecondOrder {
                    origin: LinDist3FlowConeOrigin::LineApparentPower { line, conductor },
                    arguments: vec![constant(limit), column(active, 1.0), column(reactive, 1.0)],
                });
            }
            if let Some(limit) = data.current_limit[conductor] {
                for &node in &[data.parent_nodes[conductor], data.child_nodes[conductor]] {
                    cones.push(LinDist3FlowCone::RotatedSecondOrder {
                        origin: LinDist3FlowConeOrigin::LineCurrent {
                            line,
                            conductor,
                            node,
                        },
                        arguments: vec![
                            column(voltage_columns[node], 1.0),
                            constant(limit.powi(2) / 2.0),
                            column(active, 1.0),
                            column(reactive, 1.0),
                        ],
                    });
                }
            }
        }
    }
    for (generator, data) in preparation.devices.generators.iter().enumerate() {
        for (channel, channel_data) in data.channels.iter().enumerate() {
            let active = power_column(
                &power_columns,
                &LinDist3FlowVariable::GeneratorActive { generator, channel },
            )?;
            let reactive = power_column(
                &power_columns,
                &LinDist3FlowVariable::GeneratorReactive { generator, channel },
            )?;
            if let Some(limit) = channel_data.apparent_power_limit {
                cones.push(LinDist3FlowCone::SecondOrder {
                    origin: LinDist3FlowConeOrigin::GeneratorApparentPower { generator, channel },
                    arguments: vec![constant(limit), column(active, 1.0), column(reactive, 1.0)],
                });
            }
            if let Some(limit) = channel_data.current_limit {
                cones.push(LinDist3FlowCone::RotatedSecondOrder {
                    origin: LinDist3FlowConeOrigin::GeneratorCurrent { generator, channel },
                    arguments: vec![
                        voltage_expression(
                            &channel_data.squared_winding_voltage,
                            &voltage_columns,
                        )?,
                        constant(limit.powi(2) / 2.0),
                        column(active, 1.0),
                        column(reactive, 1.0),
                    ],
                });
            }
        }
    }

    Ok(LinDist3FlowConicProblem {
        preparation,
        variables,
        equalities,
        cones,
    })
}

/// Translate a canonical primal vector back to formulation-ordered physical
/// values suitable for [`powerio_prob::LinDist3FlowOpfSolution`].
///
/// # Errors
/// The primal length differs from the canonical variable count, or the
/// problem contains an inconsistent semantic index.
#[allow(clippy::too_many_lines)]
pub fn lindist3flow_values_from_primal(
    problem: &LinDist3FlowConicProblem,
    primal: &[f64],
) -> Result<LinDist3FlowOpfValues> {
    if primal.len() != problem.variables.len() {
        return Err(invalid(format!(
            "LinDist3Flow primal has length {}, expected {}",
            primal.len(),
            problem.variables.len()
        )));
    }
    let line_offsets = problem
        .preparation
        .network
        .lines
        .iter()
        .scan(0, |offset, line| {
            let current = *offset;
            *offset += line.parent_nodes.len();
            Some(current)
        })
        .collect::<Vec<_>>();
    let generator_offsets = problem
        .preparation
        .devices
        .generators
        .iter()
        .scan(0, |offset, generator| {
            let current = *offset;
            *offset += generator.channels.len();
            Some(current)
        })
        .collect::<Vec<_>>();
    let source_offsets = problem
        .preparation
        .devices
        .sources
        .iter()
        .scan(0, |offset, source| {
            let current = *offset;
            *offset += source.terminal_nodes.len();
            Some(current)
        })
        .collect::<Vec<_>>();
    let line_count = problem
        .preparation
        .network
        .lines
        .iter()
        .map(|line| line.parent_nodes.len())
        .sum();
    let generator_count = problem
        .preparation
        .devices
        .generators
        .iter()
        .map(|generator| generator.channels.len())
        .sum();
    let source_count = problem
        .preparation
        .devices
        .sources
        .iter()
        .map(|source| source.terminal_nodes.len())
        .sum();
    let mut values = LinDist3FlowOpfValues::default();
    values.terminal_voltage_magnitude_squared = vec![0.0; problem.preparation.network.nodes.len()];
    values.line_active_power = vec![0.0; line_count];
    values.line_reactive_power = vec![0.0; line_count];
    values.generator_active_power = vec![0.0; generator_count];
    values.generator_reactive_power = vec![0.0; generator_count];
    values.source_active_power = vec![0.0; source_count];
    values.source_reactive_power = vec![0.0; source_count];
    for (column, variable) in problem.variables.iter().enumerate() {
        let value = primal[column];
        match &variable.variable {
            LinDist3FlowDecisionVariable::SquaredVoltage { node } => {
                *values
                    .terminal_voltage_magnitude_squared
                    .get_mut(*node)
                    .ok_or_else(|| invalid(format!("unknown voltage node {node}")))? = value;
            }
            LinDist3FlowDecisionVariable::Power(power) => match power {
                LinDist3FlowVariable::LineActive { line, conductor } => {
                    let position = line_offsets
                        .get(*line)
                        .copied()
                        .and_then(|offset| offset.checked_add(*conductor))
                        .ok_or_else(|| invalid("unknown line active-power index"))?;
                    *values
                        .line_active_power
                        .get_mut(position)
                        .ok_or_else(|| invalid("unknown line active-power conductor"))? = value;
                }
                LinDist3FlowVariable::LineReactive { line, conductor } => {
                    let position = line_offsets
                        .get(*line)
                        .copied()
                        .and_then(|offset| offset.checked_add(*conductor))
                        .ok_or_else(|| invalid("unknown line reactive-power index"))?;
                    *values
                        .line_reactive_power
                        .get_mut(position)
                        .ok_or_else(|| invalid("unknown line reactive-power conductor"))? = value;
                }
                LinDist3FlowVariable::GeneratorActive { generator, channel } => {
                    let position = generator_offsets
                        .get(*generator)
                        .copied()
                        .and_then(|offset| offset.checked_add(*channel))
                        .ok_or_else(|| invalid("unknown generator active-power index"))?;
                    *values
                        .generator_active_power
                        .get_mut(position)
                        .ok_or_else(|| invalid("unknown generator active-power channel"))? = value;
                }
                LinDist3FlowVariable::GeneratorReactive { generator, channel } => {
                    let position = generator_offsets
                        .get(*generator)
                        .copied()
                        .and_then(|offset| offset.checked_add(*channel))
                        .ok_or_else(|| invalid("unknown generator reactive-power index"))?;
                    *values
                        .generator_reactive_power
                        .get_mut(position)
                        .ok_or_else(|| invalid("unknown generator reactive-power channel"))? =
                        value;
                }
                LinDist3FlowVariable::SourceActive { source, channel } => {
                    let position = source_offsets
                        .get(*source)
                        .copied()
                        .and_then(|offset| offset.checked_add(*channel))
                        .ok_or_else(|| invalid("unknown source active-power index"))?;
                    *values
                        .source_active_power
                        .get_mut(position)
                        .ok_or_else(|| invalid("unknown source active-power channel"))? = value;
                }
                LinDist3FlowVariable::SourceReactive { source, channel } => {
                    let position = source_offsets
                        .get(*source)
                        .copied()
                        .and_then(|offset| offset.checked_add(*channel))
                        .ok_or_else(|| invalid("unknown source reactive-power index"))?;
                    *values
                        .source_reactive_power
                        .get_mut(position)
                        .ok_or_else(|| invalid("unknown source reactive-power channel"))? = value;
                }
            },
        }
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;
    use powerio_dist::{
        Configuration, DistBus, DistGenerator, DistLine, DistLineCode, MulticonductorNetwork,
        VoltageSource,
    };
    use powerio_prob::{LinDist3FlowBuildOptions, LinDist3FlowOpfInstance};

    use super::*;

    fn instance() -> LinDist3FlowOpfInstance {
        let terminal = vec!["1".to_owned()];
        let mut network = MulticonductorNetwork::named("conic");
        let mut source_bus = DistBus::new("source", terminal.clone());
        source_bus.v_min = Some(220.0);
        source_bus.v_max = Some(240.0);
        network.buses_mut().push(source_bus);
        let mut load_bus = DistBus::new("load", terminal.clone());
        load_bus.v_min = Some(210.0);
        load_bus.v_max = Some(240.0);
        network.buses_mut().push(load_bus);
        let mut code = DistLineCode::new("one", vec![vec![0.1]], vec![vec![0.2]]);
        code.i_max = Some(vec![10.0]);
        code.s_max = Some(vec![2_000.0]);
        network.line_codes_mut().push(code);
        network.lines_mut().push(DistLine::new(
            "line",
            "source",
            "load",
            terminal.clone(),
            terminal.clone(),
            "one",
            1.0,
        ));
        let mut source =
            VoltageSource::new("grid", "source", terminal.clone(), vec![230.0], vec![0.0]);
        source.energy_cost_rate = Some(vec![0.3]);
        network.sources_mut().push(source);
        let mut generator = DistGenerator::new(
            "pv",
            "load",
            terminal,
            Configuration::Wye,
            vec![500.0],
            vec![0.0],
        );
        generator.p_min = Some(vec![0.0]);
        generator.p_max = Some(vec![1_000.0]);
        generator.q_min = Some(vec![-500.0]);
        generator.q_max = Some(vec![500.0]);
        generator.s_max = Some(vec![1_100.0]);
        generator.i_max = Some(vec![5.0]);
        generator.cost = Some(vec![0.1]);
        network.generators_mut().push(generator);
        LinDist3FlowOpfInstance::from_network(network, LinDist3FlowBuildOptions::default()).unwrap()
    }

    fn coefficient(expression: &LinDist3FlowLinearExpression, column: usize) -> f64 {
        expression
            .terms
            .iter()
            .find(|term| term.column == column)
            .map_or(0.0, |term| term.coefficient)
    }

    #[test]
    fn assembly_has_stable_variables_equalities_and_objective() {
        let problem = build_lindist3flow_conic_problem(&instance()).unwrap();
        assert_eq!(problem.variables.len(), 8);
        assert_eq!(problem.equalities.len(), 5);
        assert_eq!(
            problem.variables[0].variable,
            LinDist3FlowDecisionVariable::SquaredVoltage { node: 0 }
        );
        assert_eq!(problem.variables[0].lower, Some(230.0f64.powi(2)));
        assert_eq!(problem.variables[0].upper, Some(230.0f64.powi(2)));
        assert_relative_eq!(problem.variables[4].objective_coefficient, 0.1 / 1000.0);
        assert_relative_eq!(problem.variables[6].objective_coefficient, 0.3 / 1000.0);
        assert_eq!(
            problem.equalities[0].origin,
            LinDist3FlowEqualityOrigin::LineDrop {
                line: 0,
                conductor: 0
            }
        );
    }

    #[test]
    fn line_drop_and_balances_reference_canonical_columns() {
        let problem = build_lindist3flow_conic_problem(&instance()).unwrap();
        let drop = &problem.equalities[0].expression;
        assert_relative_eq!(coefficient(drop, 0), -1.0);
        assert_relative_eq!(coefficient(drop, 1), 1.0);
        assert_relative_eq!(coefficient(drop, 2), 0.2);
        assert_relative_eq!(coefficient(drop, 3), 0.4);

        let source_active = &problem.equalities[1].expression;
        assert_relative_eq!(coefficient(source_active, 2), -1.0);
        assert_relative_eq!(coefficient(source_active, 6), 1.0);
        let load_active = &problem.equalities[3].expression;
        assert_relative_eq!(coefficient(load_active, 2), 1.0);
        assert_relative_eq!(coefficient(load_active, 4), 1.0);
    }

    #[test]
    fn line_and_generator_limits_become_native_cones() {
        let problem = build_lindist3flow_conic_problem(&instance()).unwrap();
        assert_eq!(problem.cones.len(), 5);
        let LinDist3FlowCone::SecondOrder { origin, arguments } = &problem.cones[0] else {
            panic!("first limit should be a standard SOC")
        };
        assert_eq!(
            origin,
            &LinDist3FlowConeOrigin::LineApparentPower {
                line: 0,
                conductor: 0
            }
        );
        assert_relative_eq!(arguments[0].constant, 2_000.0);

        let LinDist3FlowCone::RotatedSecondOrder { origin, arguments } = &problem.cones[1] else {
            panic!("line current should use a rotated SOC")
        };
        assert!(matches!(
            origin,
            LinDist3FlowConeOrigin::LineCurrent { node: 0, .. }
        ));
        assert_relative_eq!(arguments[1].constant, 50.0);
        assert_relative_eq!(coefficient(&arguments[0], 0), 1.0);

        let LinDist3FlowCone::RotatedSecondOrder { origin, arguments } = &problem.cones[4] else {
            panic!("generator current should use a rotated SOC")
        };
        assert_eq!(
            origin,
            &LinDist3FlowConeOrigin::GeneratorCurrent {
                generator: 0,
                channel: 0
            }
        );
        assert_relative_eq!(arguments[1].constant, 12.5);
    }

    #[test]
    fn canonical_primal_translates_to_physical_table_order() {
        let problem = build_lindist3flow_conic_problem(&instance()).unwrap();
        let primal = (0..problem.variables.len())
            .map(|column| column as f64 + 10.0)
            .collect::<Vec<_>>();
        let values = lindist3flow_values_from_primal(&problem, &primal).unwrap();
        assert_eq!(values.terminal_voltage_magnitude_squared, vec![10.0, 11.0]);
        assert_eq!(values.line_active_power, vec![12.0]);
        assert_eq!(values.line_reactive_power, vec![13.0]);
        assert_eq!(values.generator_active_power, vec![14.0]);
        assert_eq!(values.generator_reactive_power, vec![15.0]);
        assert_eq!(values.source_active_power, vec![16.0]);
        assert_eq!(values.source_reactive_power, vec![17.0]);
        assert!(lindist3flow_values_from_primal(&problem, &primal[..7]).is_err());
    }
}
