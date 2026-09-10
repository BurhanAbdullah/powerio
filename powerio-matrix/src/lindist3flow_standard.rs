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
    LinDist3FlowEqualityOrigin, LinDist3FlowLinearExpression, Result, SparseMatrix,
    build_lindist3flow_conic_problem,
};

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
pub fn build_lindist3flow_standard_form(
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
        canonical,
    })
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

    #[test]
    fn sparse_shapes_and_cone_blocks_match_clarabel_standard_form() {
        let form = build_lindist3flow_standard_form(&instance()).unwrap();
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
        let form = build_lindist3flow_standard_form(&instance()).unwrap();
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
        let form = build_lindist3flow_standard_form(&instance()).unwrap();
        // Five equalities, eight bounds, then the 3-row line apparent-power
        // cone. The first line-current SOC therefore starts at row 16.
        let row = 16;
        assert_relative_eq!(entry(&form.a, row, 0), -1.0);
        assert_relative_eq!(form.b[row], 50.0);
        assert_relative_eq!(entry(&form.a, row + 1, 0), -1.0);
        assert_relative_eq!(form.b[row + 1], -50.0);
        assert_relative_eq!(entry(&form.a, row + 2, 2), -std::f64::consts::SQRT_2);
        assert_relative_eq!(entry(&form.a, row + 3, 3), -std::f64::consts::SQRT_2);
    }
}
