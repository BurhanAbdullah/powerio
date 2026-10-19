//! Sparse conic standard form for solver adapters.
//!
//! This is the final PowerIO-owned numerical boundary. It follows
//! `min 1/2 x' P x + q' x` subject to `A x + s = b`, `s in K`, which is the
//! convention consumed by Clarabel. `P` is currently zero because the BMOPF
//! dispatch objective is linear.

use powerio_prob::LinDist3FlowOpfInstance;

use crate::matrix::triplet::CooBuilder;
use crate::{
    Error, LinDist3FlowCone, LinDist3FlowConeOrigin, LinDist3FlowConicProblem,
    LinDist3FlowDecisionVariable, LinDist3FlowEqualityOrigin, LinDist3FlowLinearExpression, Result,
    SparseMatrix, build_lindist3flow_conic_problem, lindist3flow_values_from_primal,
};

/// Numerical coordinate choices for the sparse solver program.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct LinDist3FlowStandardFormOptions {
    /// Use per-unit decision variables and scaled constraint rows.
    pub per_unit: bool,
    /// System apparent-power base in VA.
    pub apparent_power_base: f64,
}

impl LinDist3FlowStandardFormOptions {
    #[must_use]
    pub const fn si() -> Self {
        Self {
            per_unit: false,
            apparent_power_base: 1_000_000.0,
        }
    }

    #[must_use]
    pub const fn per_unit(apparent_power_base: f64) -> Self {
        Self {
            per_unit: true,
            apparent_power_base,
        }
    }
}

impl Default for LinDist3FlowStandardFormOptions {
    fn default() -> Self {
        Self::per_unit(1_000_000.0)
    }
}

/// Diagonal coordinate maps applied to the canonical SI program.
///
/// Physical primals satisfy `x_si = variable_scale .* x_solver`. Scaled rows
/// satisfy `(A_solver, b_solver) = row_scale .* (A_si * variable_scale, b_si)`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct LinDist3FlowScaling {
    /// Apparent-power base in VA, or `None` when solver coordinates are SI.
    pub apparent_power_base: Option<f64>,
    /// One positive SI-unit multiplier per decision variable.
    pub variable_scale: Vec<f64>,
    /// One positive multiplier per standard-form constraint row.
    pub row_scale: Vec<f64>,
}

/// One contiguous cone block in standard-form row order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LinDist3FlowStandardCone {
    Zero { dimension: usize },
    Nonnegative { dimension: usize },
    SecondOrder { dimension: usize },
}

impl LinDist3FlowStandardCone {
    #[must_use]
    pub const fn dimension(self) -> usize {
        match self {
            Self::Zero { dimension }
            | Self::Nonnegative { dimension }
            | Self::SecondOrder { dimension } => dimension,
        }
    }
}

/// Semantic provenance for one row of `A` and `b`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LinDist3FlowStandardRowOrigin {
    Equality(LinDist3FlowEqualityOrigin),
    VariableLowerBound {
        column: usize,
    },
    VariableUpperBound {
        column: usize,
    },
    Cone {
        origin: LinDist3FlowConeOrigin,
        /// Position after any rotated-to-standard transformation.
        row_in_cone: usize,
    },
}

/// Clarabel-compatible sparse conic data plus its semantic source model.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct LinDist3FlowStandardForm {
    /// Zero quadratic objective matrix, CSC, `n x n`.
    pub p: SparseMatrix,
    /// Linear objective vector, length `n`.
    pub q: Vec<f64>,
    /// Constraint matrix in CSC storage.
    pub a: SparseMatrix,
    /// Constraint right-hand side, length `m`.
    pub b: Vec<f64>,
    /// Contiguous cone blocks whose dimensions sum to `m`.
    pub cones: Vec<LinDist3FlowStandardCone>,
    /// One semantic origin per constraint row.
    pub row_origins: Vec<LinDist3FlowStandardRowOrigin>,
    /// Exact diagonal maps between solver coordinates and canonical SI.
    pub scaling: LinDist3FlowScaling,
    /// Canonical model retained for primal decoding and identity lookup.
    pub canonical: LinDist3FlowConicProblem,
}

fn invalid(reason: impl Into<String>) -> Error {
    Error::InvalidLinDist3FlowCoefficients {
        reason: reason.into(),
    }
}

fn bound_count(problem: &LinDist3FlowConicProblem) -> usize {
    problem
        .variables
        .iter()
        .map(|variable| {
            usize::from(variable.lower.is_some()) + usize::from(variable.upper.is_some())
        })
        .sum()
}

fn cone_dimension(cone: &LinDist3FlowCone) -> usize {
    match cone {
        LinDist3FlowCone::SecondOrder { arguments, .. }
        | LinDist3FlowCone::RotatedSecondOrder { arguments, .. } => arguments.len(),
    }
}

fn add_slack_expression(
    a: &mut CooBuilder,
    b: &mut Vec<f64>,
    row: usize,
    parts: &[(&LinDist3FlowLinearExpression, f64)],
) -> Result<()> {
    let mut constant = 0.0;
    for (expression, scale) in parts {
        constant += scale * expression.constant;
        for term in &expression.terms {
            let coefficient = -scale * term.coefficient;
            if !coefficient.is_finite() {
                return Err(invalid(format!(
                    "standard-form row {row} has a non-finite matrix coefficient"
                )));
            }
            a.add(row, term.column, coefficient);
        }
    }
    if !constant.is_finite() {
        return Err(invalid(format!(
            "standard-form row {row} has a non-finite right-hand side"
        )));
    }
    b.push(constant);
    Ok(())
}

fn push_cone_origins(
    origins: &mut Vec<LinDist3FlowStandardRowOrigin>,
    origin: &LinDist3FlowConeOrigin,
    dimension: usize,
) {
    origins.extend(
        (0..dimension).map(|row_in_cone| LinDist3FlowStandardRowOrigin::Cone {
            origin: origin.clone(),
            row_in_cone,
        }),
    );
}

/// Compile a LinDist3Flow instance into sparse `P, q, A, b, K` standard form.
///
/// Rotated cones `(u, v, z...)` are mapped to the ordinary SOC
/// `(u + v, u - v, sqrt(2) z...)`. This preserves
/// `2 u v >= ||z||²` and means an adapter needs only zero, nonnegative, and
/// ordinary second-order cones.
///
/// # Errors
/// As [`build_lindist3flow_conic_problem`], or a cone has an invalid
/// dimension or produces non-finite standard-form data.
#[allow(clippy::many_single_char_names, clippy::too_many_lines)]
fn build_lindist3flow_standard_form_si(
    instance: &LinDist3FlowOpfInstance,
) -> Result<LinDist3FlowStandardForm> {
    let canonical = build_lindist3flow_conic_problem(instance)?;
    let n = canonical.variables.len();
    let bounds = bound_count(&canonical);
    let cone_rows: usize = canonical.cones.iter().map(cone_dimension).sum();
    let m = canonical
        .equalities
        .len()
        .checked_add(bounds)
        .and_then(|rows| rows.checked_add(cone_rows))
        .ok_or_else(|| invalid("LinDist3Flow standard-form row count overflows usize"))?;
    let estimated_nnz = canonical
        .equalities
        .iter()
        .map(|row| row.expression.terms.len())
        .sum::<usize>()
        .saturating_add(bounds)
        .saturating_add(
            canonical
                .cones
                .iter()
                .map(|cone| match cone {
                    LinDist3FlowCone::SecondOrder { arguments, .. }
                    | LinDist3FlowCone::RotatedSecondOrder { arguments, .. } => arguments
                        .iter()
                        .map(|argument| argument.terms.len())
                        .sum::<usize>(),
                })
                .sum::<usize>(),
        );
    let p = CooBuilder::new(n).finish_csc();
    let q = canonical
        .variables
        .iter()
        .map(|variable| variable.objective_coefficient)
        .collect();
    let mut a = CooBuilder::with_capacity_rect(m, n, estimated_nnz);
    let mut b = Vec::with_capacity(m);
    let mut cones = Vec::new();
    let mut row_origins = Vec::with_capacity(m);
    let mut row = 0;

    if !canonical.equalities.is_empty() {
        cones.push(LinDist3FlowStandardCone::Zero {
            dimension: canonical.equalities.len(),
        });
    }
    for equality in &canonical.equalities {
        for term in &equality.expression.terms {
            a.add(row, term.column, term.coefficient);
        }
        b.push(-equality.expression.constant);
        row_origins.push(LinDist3FlowStandardRowOrigin::Equality(
            equality.origin.clone(),
        ));
        row += 1;
    }

    if bounds != 0 {
        cones.push(LinDist3FlowStandardCone::Nonnegative { dimension: bounds });
    }
    for (column, variable) in canonical.variables.iter().enumerate() {
        if let Some(lower) = variable.lower {
            // -x + s = -lower, so s = x - lower >= 0.
            a.add(row, column, -1.0);
            b.push(-lower);
            row_origins.push(LinDist3FlowStandardRowOrigin::VariableLowerBound { column });
            row += 1;
        }
        if let Some(upper) = variable.upper {
            // x + s = upper, so s = upper - x >= 0.
            a.add(row, column, 1.0);
            b.push(upper);
            row_origins.push(LinDist3FlowStandardRowOrigin::VariableUpperBound { column });
            row += 1;
        }
    }

    for cone in &canonical.cones {
        match cone {
            LinDist3FlowCone::SecondOrder { origin, arguments } => {
                if arguments.len() < 2 {
                    return Err(invalid("a second-order cone needs at least two rows"));
                }
                cones.push(LinDist3FlowStandardCone::SecondOrder {
                    dimension: arguments.len(),
                });
                push_cone_origins(&mut row_origins, origin, arguments.len());
                for argument in arguments {
                    add_slack_expression(&mut a, &mut b, row, &[(argument, 1.0)])?;
                    row += 1;
                }
            }
            LinDist3FlowCone::RotatedSecondOrder { origin, arguments } => {
                if arguments.len() < 3 {
                    return Err(invalid(
                        "a rotated second-order cone needs at least three rows",
                    ));
                }
                cones.push(LinDist3FlowStandardCone::SecondOrder {
                    dimension: arguments.len(),
                });
                push_cone_origins(&mut row_origins, origin, arguments.len());
                add_slack_expression(
                    &mut a,
                    &mut b,
                    row,
                    &[(&arguments[0], 1.0), (&arguments[1], 1.0)],
                )?;
                row += 1;
                add_slack_expression(
                    &mut a,
                    &mut b,
                    row,
                    &[(&arguments[0], 1.0), (&arguments[1], -1.0)],
                )?;
                row += 1;
                for argument in &arguments[2..] {
                    add_slack_expression(
                        &mut a,
                        &mut b,
                        row,
                        &[(argument, std::f64::consts::SQRT_2)],
                    )?;
                    row += 1;
                }
            }
        }
    }
    debug_assert_eq!(row, m);
    debug_assert_eq!(b.len(), m);
    debug_assert_eq!(row_origins.len(), m);
    debug_assert_eq!(cones.iter().map(|cone| cone.dimension()).sum::<usize>(), m);
    Ok(LinDist3FlowStandardForm {
        p,
        q,
        a: a.finish_csc(),
        b,
        cones,
        row_origins,
        scaling: LinDist3FlowScaling {
            apparent_power_base: None,
            variable_scale: vec![1.0; n],
            row_scale: vec![1.0; m],
        },
        canonical,
    })
}

fn per_unit_scales(
    form: &LinDist3FlowStandardForm,
    power_base: f64,
) -> Result<LinDist3FlowScaling> {
    if !power_base.is_finite() || power_base <= 0.0 {
        return Err(invalid(
            "LinDist3Flow apparent-power base must be finite and positive",
        ));
    }
    let variable_scale = form
        .canonical
        .variables
        .iter()
        .map(|variable| match &variable.variable {
            LinDist3FlowDecisionVariable::SquaredVoltage { node } => {
                form.canonical.preparation.network.nodes[*node]
                    .reference_magnitude
                    .powi(2)
            }
            LinDist3FlowDecisionVariable::Power(_) => power_base,
        })
        .collect::<Vec<_>>();
    if variable_scale
        .iter()
        .any(|scale| !scale.is_finite() || *scale <= 0.0)
    {
        return Err(invalid(
            "LinDist3Flow variable scaling contains a non-finite or nonpositive base",
        ));
    }
    let row_scale = form
        .row_origins
        .iter()
        .map(|origin| match origin {
            LinDist3FlowStandardRowOrigin::Equality(LinDist3FlowEqualityOrigin::LineDrop {
                line,
                conductor,
            }) => {
                let node = form.canonical.preparation.network.lines[*line].child_nodes[*conductor];
                1.0 / form.canonical.preparation.network.nodes[node]
                    .reference_magnitude
                    .powi(2)
            }
            LinDist3FlowStandardRowOrigin::Equality(
                LinDist3FlowEqualityOrigin::ActiveBalance { .. }
                | LinDist3FlowEqualityOrigin::ReactiveBalance { .. },
            )
            | LinDist3FlowStandardRowOrigin::Cone { .. } => 1.0 / power_base,
            LinDist3FlowStandardRowOrigin::VariableLowerBound { column }
            | LinDist3FlowStandardRowOrigin::VariableUpperBound { column } => {
                1.0 / variable_scale[*column]
            }
        })
        .collect::<Vec<_>>();
    Ok(LinDist3FlowScaling {
        apparent_power_base: Some(power_base),
        variable_scale,
        row_scale,
    })
}

fn apply_scaling(form: &mut LinDist3FlowStandardForm, scaling: LinDist3FlowScaling) {
    for (column, mut entries) in form.a.outer_iterator_mut().enumerate() {
        for (row, value) in entries.iter_mut() {
            *value *= scaling.variable_scale[column] * scaling.row_scale[row];
        }
    }
    for (coefficient, scale) in form.q.iter_mut().zip(&scaling.variable_scale) {
        *coefficient *= scale;
    }
    for (right_hand_side, scale) in form.b.iter_mut().zip(&scaling.row_scale) {
        *right_hand_side *= scale;
    }
    form.scaling = scaling;
}

/// Compile a LinDist3Flow instance in the default per-unit coordinates using
/// a 1 MVA system power base. Input, canonical data, and decoded results remain
/// SI.
///
/// # Errors
/// As [`build_lindist3flow_standard_form_with_options`].
pub fn build_lindist3flow_standard_form(
    instance: &LinDist3FlowOpfInstance,
) -> Result<LinDist3FlowStandardForm> {
    build_lindist3flow_standard_form_with_options(
        instance,
        LinDist3FlowStandardFormOptions::default(),
    )
}

/// Compile sparse standard form using explicit SI or per-unit solver
/// coordinates.
///
/// # Errors
/// As [`build_lindist3flow_conic_problem`], or the requested power base is
/// non-finite or nonpositive.
pub fn build_lindist3flow_standard_form_with_options(
    instance: &LinDist3FlowOpfInstance,
    options: LinDist3FlowStandardFormOptions,
) -> Result<LinDist3FlowStandardForm> {
    let mut form = build_lindist3flow_standard_form_si(instance)?;
    if options.per_unit {
        let scaling = per_unit_scales(&form, options.apparent_power_base)?;
        apply_scaling(&mut form, scaling);
    }
    Ok(form)
}

/// Decode a solver-coordinate primal vector into physical SI values.
///
/// # Errors
/// The primal vector has the wrong length or contains an inconsistent
/// canonical semantic index.
pub fn lindist3flow_values_from_standard_primal(
    form: &LinDist3FlowStandardForm,
    primal: &[f64],
) -> Result<powerio_prob::LinDist3FlowOpfValues> {
    if primal.len() != form.scaling.variable_scale.len() {
        return Err(invalid(format!(
            "LinDist3Flow solver primal has length {}, expected {}",
            primal.len(),
            form.scaling.variable_scale.len()
        )));
    }
    let physical = primal
        .iter()
        .zip(&form.scaling.variable_scale)
        .map(|(value, scale)| value * scale)
        .collect::<Vec<_>>();
    lindist3flow_values_from_primal(&form.canonical, &physical)
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
        let mut network = MulticonductorNetwork::named("standard");
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

    fn entry(matrix: &SparseMatrix, row: usize, column: usize) -> f64 {
        matrix.get(row, column).copied().unwrap_or_default()
    }

    fn si_form() -> LinDist3FlowStandardForm {
        build_lindist3flow_standard_form_with_options(
            &instance(),
            LinDist3FlowStandardFormOptions::si(),
        )
        .unwrap()
    }

    #[test]
    fn sparse_shapes_and_cone_blocks_match_clarabel_standard_form() {
        let form = si_form();
        assert_eq!(form.p.shape(), (8, 8));
        assert!(form.p.is_csc());
        assert_eq!(form.p.nnz(), 0);
        assert_eq!(form.a.shape(), (31, 8));
        assert!(form.a.is_csc());
        assert_eq!(form.q.len(), 8);
        assert_eq!(form.b.len(), 31);
        assert_eq!(form.row_origins.len(), 31);
        assert_eq!(
            form.cones,
            vec![
                LinDist3FlowStandardCone::Zero { dimension: 5 },
                LinDist3FlowStandardCone::Nonnegative { dimension: 8 },
                LinDist3FlowStandardCone::SecondOrder { dimension: 3 },
                LinDist3FlowStandardCone::SecondOrder { dimension: 4 },
                LinDist3FlowStandardCone::SecondOrder { dimension: 4 },
                LinDist3FlowStandardCone::SecondOrder { dimension: 3 },
                LinDist3FlowStandardCone::SecondOrder { dimension: 4 },
            ]
        );
    }

    #[test]
    fn equality_and_bound_rows_have_ax_plus_s_equals_b_signs() {
        let form = si_form();
        assert_relative_eq!(entry(&form.a, 0, 0), -1.0);
        assert_relative_eq!(entry(&form.a, 0, 1), 1.0);
        assert_relative_eq!(entry(&form.a, 0, 2), 0.2);
        assert_relative_eq!(entry(&form.a, 0, 3), 0.4);
        assert_relative_eq!(form.b[0], 0.0);

        assert_relative_eq!(entry(&form.a, 5, 0), -1.0);
        assert_relative_eq!(form.b[5], -230.0f64.powi(2));
        assert_relative_eq!(entry(&form.a, 6, 0), 1.0);
        assert_relative_eq!(form.b[6], 230.0f64.powi(2));
    }

    #[test]
    fn rotated_current_cone_is_an_equivalent_ordinary_soc() {
        let form = si_form();
        // Five equalities, eight bounds, then the 3-row line apparent-power
        // cone. The first line-current SOC therefore starts at row 16.
        let row = 16;
        assert_relative_eq!(entry(&form.a, row, 0), -10.0 / 230.0);
        assert_relative_eq!(form.b[row], 1_150.0);
        assert_relative_eq!(entry(&form.a, row + 1, 0), -10.0 / 230.0);
        assert_relative_eq!(form.b[row + 1], -1_150.0);
        assert_relative_eq!(entry(&form.a, row + 2, 2), -std::f64::consts::SQRT_2);
        assert_relative_eq!(entry(&form.a, row + 3, 3), -std::f64::consts::SQRT_2);
    }

    #[test]
    fn default_per_unit_coordinates_round_trip_to_si_values() {
        let form = build_lindist3flow_standard_form(&instance()).unwrap();
        assert_eq!(form.scaling.apparent_power_base, Some(1_000_000.0));
        assert_relative_eq!(form.scaling.variable_scale[0], 230.0f64.powi(2));
        assert_relative_eq!(form.scaling.variable_scale[2], 1_000_000.0);

        // The physical cost rates are applied to per-unit power variables.
        assert_relative_eq!(form.q[4], 100.0);
        assert_relative_eq!(form.q[6], 300.0);

        // Per-unit voltage bounds are normalized by each node's reference
        // magnitude. The source voltage is fixed at 230 V.
        assert_relative_eq!(entry(&form.a, 5, 0), -1.0);
        assert_relative_eq!(form.b[5], -1.0);
        assert_relative_eq!(entry(&form.a, 6, 0), 1.0);
        assert_relative_eq!(form.b[6], 1.0);

        // Canonical order is w(source), w(load), line p/q, generator p/q,
        // source p/q. Solver values decode back to physical SI values.
        let primal = vec![
            1.0,
            228.0f64.powi(2) / 230.0f64.powi(2),
            0.001,
            0.0002,
            0.0005,
            0.0,
            0.001,
            0.0002,
        ];
        let values = lindist3flow_values_from_standard_primal(&form, &primal).unwrap();
        assert_relative_eq!(
            values.terminal_voltage_magnitude_squared[0],
            230.0f64.powi(2)
        );
        assert_relative_eq!(
            values.terminal_voltage_magnitude_squared[1],
            228.0f64.powi(2)
        );
        assert_relative_eq!(values.line_active_power[0], 1_000.0);
        assert_relative_eq!(values.generator_active_power[0], 500.0);
        assert_relative_eq!(values.source_reactive_power[0], 200.0);
    }

    #[test]
    fn per_unit_form_is_an_exact_diagonal_scaling_of_si_form() {
        let si = si_form();
        let per_unit = build_lindist3flow_standard_form_with_options(
            &instance(),
            LinDist3FlowStandardFormOptions::per_unit(2_000_000.0),
        )
        .unwrap();

        assert_eq!(per_unit.a.shape(), si.a.shape());
        assert_eq!(per_unit.cones, si.cones);
        assert_eq!(per_unit.row_origins, si.row_origins);
        for column in 0..si.a.cols() {
            for row in 0..si.a.rows() {
                assert_relative_eq!(
                    entry(&per_unit.a, row, column),
                    entry(&si.a, row, column)
                        * per_unit.scaling.variable_scale[column]
                        * per_unit.scaling.row_scale[row],
                    epsilon = 1e-12
                );
            }
        }
        for column in 0..si.q.len() {
            assert_relative_eq!(
                per_unit.q[column],
                si.q[column] * per_unit.scaling.variable_scale[column],
                epsilon = 1e-12
            );
        }
        for row in 0..si.b.len() {
            assert_relative_eq!(
                per_unit.b[row],
                si.b[row] * per_unit.scaling.row_scale[row],
                epsilon = 1e-12
            );
        }
    }

    #[test]
    fn per_unit_power_base_must_be_positive_and_finite() {
        for power_base in [0.0, -1.0, f64::INFINITY, f64::NAN] {
            assert!(
                build_lindist3flow_standard_form_with_options(
                    &instance(),
                    LinDist3FlowStandardFormOptions::per_unit(power_base),
                )
                .is_err()
            );
        }
    }
}
