use powerio_dist::{
    DistBus, DistIbr, DistLine, DistLineCode, DistLoad, IbrPrimeMover, IbrTopology,
    MulticonductorNetwork, NeutralKronGrounding, NeutralKronOptions, VoltageSource,
    neutral_kron_reduce,
};

fn bus(id: &str, terminals: &[&str], neutral: &str, grounded: bool) -> DistBus {
    let mut bus = DistBus::new(
        id,
        terminals
            .iter()
            .map(|terminal| (*terminal).to_owned())
            .collect(),
    );
    if grounded {
        bus.grounded.push(neutral.to_owned());
    }
    bus
}

fn four_wire_code() -> DistLineCode {
    let r = vec![
        vec![1.0, 0.0, 0.0, 0.2],
        vec![0.0, 1.0, 0.0, 0.2],
        vec![0.0, 0.0, 1.0, 0.2],
        vec![0.2, 0.2, 0.2, 2.0],
    ];
    let mut code = DistLineCode::new("four-wire", r, vec![vec![0.0; 4]; 4]);
    code.i_max = Some(vec![100.0, 100.0, 100.0, f64::INFINITY]);
    code
}

fn basic_network(grounded: bool) -> MulticonductorNetwork {
    let mut network = MulticonductorNetwork::named("kron");
    network
        .buses_mut()
        .push(bus("source", &["1", "2", "3", "4"], "4", grounded));
    network
        .buses_mut()
        .push(bus("load", &["1", "2", "3", "4"], "4", grounded));
    network.line_codes_mut().push(four_wire_code());
    let mut line = DistLine::new(
        "line",
        "source",
        "load",
        vec!["1".into(), "2".into(), "3".into(), "4".into()],
        vec!["1".into(), "2".into(), "3".into(), "4".into()],
        "four-wire",
        1.0,
    );
    line.i_max = Some(vec![90.0, 91.0, 92.0, f64::INFINITY]);
    network.lines_mut().push(line);
    network.sources_mut().push(VoltageSource::new(
        "grid",
        "source",
        vec!["1".into(), "2".into(), "3".into(), "4".into()],
        vec![230.0, 230.0, 230.0, 0.0],
        vec![0.0, -2.094, 2.094, 0.0],
    ));
    network.loads_mut().push(DistLoad::new(
        "load",
        "load",
        vec!["1".into(), "2".into(), "3".into(), "4".into()],
        powerio_dist::Configuration::Wye,
        vec![1_000.0, 2_000.0, 3_000.0],
        vec![100.0, 200.0, 300.0],
    ));
    network
}

#[test]
fn reduces_series_impedance_and_terminal_aligned_data_without_mutating_input() {
    let mut network = basic_network(true);
    network.buses_mut()[1].v_min_phase = Some(vec![210.0, 211.0, 212.0, 0.0]);
    network.buses_mut()[1].v_max_phase = Some(vec![240.0, 241.0, 242.0, 0.0]);
    let reduction = neutral_kron_reduce(&network, &NeutralKronOptions::default()).unwrap();
    let reduced = reduction.network();

    assert_eq!(network.buses()[0].terminals.len(), 4, "input was mutated");
    assert_eq!(reduced.buses()[0].terminals, ["1", "2", "3"]);
    assert_eq!(reduced.buses()[1].terminals, ["1", "2", "3"]);
    assert_eq!(
        reduced.buses()[1].v_min_phase.as_deref(),
        Some(&[210.0, 211.0, 212.0][..])
    );
    assert_eq!(
        reduced.buses()[1].v_max_phase.as_deref(),
        Some(&[240.0, 241.0, 242.0][..])
    );
    assert!(reduced.buses().iter().all(|bus| bus.grounded.is_empty()));
    assert_eq!(reduced.lines()[0].terminal_map_from, ["1", "2", "3"]);
    assert_eq!(reduced.lines()[0].terminal_map_to, ["1", "2", "3"]);
    assert_eq!(
        reduced.lines()[0].i_max.as_deref(),
        Some(&[90.0, 91.0, 92.0][..])
    );
    assert_eq!(reduced.sources()[0].v_magnitude, [230.0, 230.0, 230.0]);
    assert_eq!(reduced.loads()[0].p_nom, [1_000.0, 2_000.0, 3_000.0]);

    let code = &reduced.line_codes()[0];
    assert_eq!(code.n_conductors, 3);
    assert_eq!(code.i_max.as_deref(), Some(&[100.0, 100.0, 100.0][..]));
    for row in 0..3 {
        for column in 0..3 {
            let expected = if row == column { 0.98 } else { -0.02 };
            assert!((code.r_series[row][column] - expected).abs() < 1e-12);
            assert!(code.x_series[row][column].abs() < 1e-12);
        }
    }

    let report = reduction.report();
    assert_eq!(report.buses.len(), 2);
    assert!(
        report
            .buses
            .iter()
            .all(|bus| bus.grounding == NeutralKronGrounding::Perfect)
    );
    assert_eq!(report.recoveries.len(), 1);
    assert_eq!(report.recoveries[0].retained_positions, [0, 1, 2]);
    assert_eq!(report.recoveries[0].k_re, [-0.1, -0.1, -0.1]);
    assert_eq!(report.recoveries[0].k_im, [0.0, 0.0, 0.0]);
    assert!(reduced.extras().contains_key("powerio_neutral_kron"));
}

#[test]
fn floating_neutral_requires_an_explicit_idealization() {
    let network = basic_network(false);
    let error = neutral_kron_reduce(&network, &NeutralKronOptions::default()).unwrap_err();
    assert!(error.to_string().contains("is not perfectly grounded"));

    let options = NeutralKronOptions::default().with_forced_ideal_ground(true);
    let reduction = neutral_kron_reduce(&network, &options).unwrap();
    assert!(
        reduction
            .report()
            .buses
            .iter()
            .all(|bus| bus.grounding == NeutralKronGrounding::ForcedIdeal)
    );
}

#[test]
fn one_shared_linecode_is_cloned_for_different_neutral_positions() {
    let mut network = MulticonductorNetwork::named("positions");
    for (id, terminals, neutral) in [
        ("a", vec!["1", "2", "3", "4"], "4"),
        ("b", vec!["1", "2", "3", "4"], "4"),
        ("c", vec!["n", "1", "2", "3"], "n"),
        ("d", vec!["n", "1", "2", "3"], "n"),
    ] {
        network.buses_mut().push(bus(id, &terminals, neutral, true));
    }
    let mut code = four_wire_code();
    code.i_max = None;
    network.line_codes_mut().push(code);
    network.lines_mut().push(DistLine::new(
        "ab",
        "a",
        "b",
        vec!["1".into(), "2".into(), "3".into(), "4".into()],
        vec!["1".into(), "2".into(), "3".into(), "4".into()],
        "four-wire",
        1.0,
    ));
    network.lines_mut().push(DistLine::new(
        "cd",
        "c",
        "d",
        vec!["n".into(), "1".into(), "2".into(), "3".into()],
        vec!["n".into(), "1".into(), "2".into(), "3".into()],
        "four-wire",
        1.0,
    ));

    let reduction = neutral_kron_reduce(&network, &NeutralKronOptions::default()).unwrap();
    assert_ne!(
        reduction.network().lines()[0].linecode,
        reduction.network().lines()[1].linecode
    );
    assert_eq!(reduction.report().recoveries.len(), 2);
    assert_eq!(reduction.network().line_codes().len(), 3);
}

#[test]
fn active_neutral_leg_ibr_is_rejected() {
    let mut network = basic_network(true);
    network.ibrs_mut().push(DistIbr::new(
        "four-leg",
        "load",
        vec!["1".into(), "2".into(), "3".into(), "4".into()],
        IbrTopology::FourLeg,
        IbrPrimeMover::Pv,
        vec![1_000.0, 1_000.0, 1_000.0],
    ));
    let error = neutral_kron_reduce(&network, &NeutralKronOptions::default()).unwrap_err();
    assert!(error.to_string().contains("active neutral leg"));
}

#[test]
fn singular_neutral_self_impedance_is_rejected() {
    let mut network = basic_network(true);
    network.line_codes_mut()[0].r_series[3][3] = 0.0;
    let error = neutral_kron_reduce(&network, &NeutralKronOptions::default()).unwrap_err();
    assert!(error.to_string().contains("singular or near-singular"));
}

#[test]
fn neutral_current_limits_cannot_disappear_during_projection() {
    for on_line in [false, true] {
        let mut network = basic_network(true);
        if on_line {
            network.lines_mut()[0].i_max.as_mut().unwrap()[3] = 10.0;
        } else {
            network.line_codes_mut()[0].i_max.as_mut().unwrap()[3] = 20.0;
        }
        let error = neutral_kron_reduce(&network, &NeutralKronOptions::default()).unwrap_err();
        assert!(error.to_string().contains("neutral current limit"));
    }
}

#[test]
fn recovered_current_has_zero_neutral_voltage_drop_and_preserves_phase_drop() {
    use num_complex::Complex64;
    let mut network = basic_network(true);
    let source = &mut network.line_codes_mut()[0];
    source.x_series = source
        .r_series
        .iter()
        .map(|row| row.iter().map(|r| r * 0.4).collect())
        .collect();
    let reduction = neutral_kron_reduce(&network, &NeutralKronOptions::default()).unwrap();
    let recovery = &reduction.report().recoveries[0];
    let phases = [
        Complex64::new(8.0, -2.0),
        Complex64::new(-3.0, 4.0),
        Complex64::new(1.0, 7.0),
    ];
    let neutral: Complex64 = phases
        .iter()
        .enumerate()
        .map(|(i, current)| Complex64::new(recovery.k_re[i], recovery.k_im[i]) * current)
        .sum();
    let currents = [phases[0], phases[1], phases[2], neutral];
    let source = &network.line_codes()[0];
    let reduced = &reduction.network().line_codes()[0];
    for row in 0..4 {
        let drop: Complex64 = currents
            .iter()
            .enumerate()
            .map(|(col, current)| {
                Complex64::new(source.r_series[row][col], source.x_series[row][col]) * current
            })
            .sum();
        let expected = if row == 3 {
            Complex64::new(0.0, 0.0)
        } else {
            phases
                .iter()
                .enumerate()
                .map(|(col, current)| {
                    Complex64::new(reduced.r_series[row][col], reduced.x_series[row][col]) * current
                })
                .sum()
        };
        assert!((drop - expected).norm() < 1e-12);
    }
}
