use approx::assert_relative_eq;
use powerio_core::Source;
use powerio_dist::{NeutralKronOptions, neutral_kron_reduce};
use powerio_matrix::{
    LinDist3FlowDecisionVariable, LinDist3FlowStandardCone, LinDist3FlowVariable,
    build_lindist3flow_standard_form, lindist3flow_values_from_standard_primal,
};
use powerio_prob::{LinDist3FlowBuildOptions, LinDist3FlowOpfInstance};

const EXPLICIT_NEUTRAL_TWO_BUS: &str = r#"
{
  "terminal_conventions": {"phase": ["a"], "neutral": ["n"]},
  "bus": {
    "source": {
      "terminal_names": ["a", "n"],
      "perfectly_grounded_terminals": ["n"]
    },
    "load": {
      "terminal_names": ["a", "n"],
      "perfectly_grounded_terminals": ["n"],
      "v_min": [180.0, 0.0],
      "v_max": [250.0, 0.0]
    }
  },
  "linecode": {
    "lc": {
      "R_series_1_1": 0.2,
      "X_series_1_1": 0.1,
      "R_series_1_2": 0.02,
      "X_series_1_2": 0.01,
      "R_series_2_2": 0.1,
      "X_series_2_2": 0.05,
      "i_max": [100.0, 100.0],
      "s_max": [30000.0, 30000.0]
    }
  },
  "line": {
    "line": {
      "bus_from": "source",
      "bus_to": "load",
      "terminal_map_from": ["a", "n"],
      "terminal_map_to": ["a", "n"],
      "linecode": "lc",
      "length": 1.0
    }
  },
  "voltage_source": {
    "source": {
      "bus": "source",
      "terminal_map": ["a", "n"],
      "v_magnitude": [230.0, 0.0],
      "v_angle": [0.0, 0.0],
      "cost": [1.0, 0.0]
    }
  },
  "load": {
    "load": {
      "bus": "load",
      "terminal_map": ["a", "n"],
      "configuration": "SINGLE_PHASE",
      "model": "constant_power",
      "p_nom": [10000.0],
      "q_nom": [2000.0]
    }
  }
}
"#;

fn physical_value(
    form: &powerio_matrix::LinDist3FlowStandardForm,
    variable: &LinDist3FlowDecisionVariable,
) -> f64 {
    match variable {
        LinDist3FlowDecisionVariable::SquaredVoltage { node } => {
            match form.canonical.preparation.network.nodes[*node]
                .node
                .bus
                .as_str()
            {
                "source" => 230.0f64.powi(2),
                "load" => 48_588.0,
                bus => panic!("unexpected voltage bus {bus}"),
            }
        }
        LinDist3FlowDecisionVariable::Power(variable) => match variable {
            LinDist3FlowVariable::LineActive { .. } | LinDist3FlowVariable::SourceActive { .. } => {
                10_000.0
            }
            LinDist3FlowVariable::LineReactive { .. }
            | LinDist3FlowVariable::SourceReactive { .. } => 2_000.0,
            other => panic!("unexpected dispatch variable {other:?}"),
        },
        other => panic!("unexpected decision variable {other:?}"),
    }
}

fn standard_slack(form: &powerio_matrix::LinDist3FlowStandardForm, primal: &[f64]) -> Vec<f64> {
    let mut slack = form.b.clone();
    for (column, entries) in form.a.outer_iterator().enumerate() {
        for (row, coefficient) in entries.iter() {
            slack[row] -= coefficient * primal[column];
        }
    }
    slack
}

#[test]
fn bmopf_kron_standard_form_and_si_decode_match_reference_feeder() {
    let source = Source::from_memory(
        "two_bus.bmopf.json",
        EXPLICIT_NEUTRAL_TWO_BUS.as_bytes().to_vec(),
    )
    .unwrap();
    let module = powerio_dist::parse(source).unwrap();
    let reduction = neutral_kron_reduce(module.value(), &NeutralKronOptions::default()).unwrap();
    assert_eq!(reduction.report().buses.len(), 2);
    assert_eq!(reduction.report().recoveries.len(), 1);

    let options = LinDist3FlowBuildOptions::default().with_required_neutral_provenance(true);
    let instance =
        LinDist3FlowOpfInstance::from_network(reduction.network().clone(), options).unwrap();
    let form = build_lindist3flow_standard_form(&instance).unwrap();
    assert_eq!(form.scaling.apparent_power_base, Some(1_000_000.0));

    let solver_primal = form
        .canonical
        .variables
        .iter()
        .zip(&form.scaling.variable_scale)
        .map(|(variable, scale)| physical_value(&form, &variable.variable) / scale)
        .collect::<Vec<_>>();
    let slack = standard_slack(&form, &solver_primal);

    let mut row = 0;
    for cone in &form.cones {
        let dimension = cone.dimension();
        let block = &slack[row..row + dimension];
        match cone {
            LinDist3FlowStandardCone::Zero { .. } => {
                assert!(
                    block.iter().all(|value| value.abs() < 1e-12),
                    "zero-cone residual {block:?}"
                );
            }
            LinDist3FlowStandardCone::Nonnegative { .. } => {
                assert!(block.iter().all(|value| *value >= -1e-12));
            }
            LinDist3FlowStandardCone::SecondOrder { .. } => {
                let tail_norm = block[1..]
                    .iter()
                    .map(|value| value.powi(2))
                    .sum::<f64>()
                    .sqrt();
                assert!(block[0] + 1e-12 >= tail_norm);
            }
            other => panic!("unexpected standard cone {other:?}"),
        }
        row += dimension;
    }
    assert_eq!(row, form.b.len());

    let values = lindist3flow_values_from_standard_primal(&form, &solver_primal).unwrap();
    let load_node = form
        .canonical
        .preparation
        .network
        .nodes
        .iter()
        .position(|node| node.node.bus == "load")
        .unwrap();
    assert_relative_eq!(
        values.terminal_voltage_magnitude_squared[load_node],
        48_588.0,
        epsilon = 1e-8
    );
    assert_relative_eq!(values.line_active_power[0], 10_000.0, epsilon = 1e-9);
    assert_relative_eq!(values.line_reactive_power[0], 2_000.0, epsilon = 1e-9);
    assert_relative_eq!(values.source_active_power[0], 10_000.0, epsilon = 1e-9);

    let objective = form
        .q
        .iter()
        .zip(&solver_primal)
        .map(|(coefficient, value)| coefficient * value)
        .sum::<f64>();
    assert_relative_eq!(objective, 10.0, epsilon = 1e-12);
}
