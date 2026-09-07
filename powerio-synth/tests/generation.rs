use powerio_synth::{generate, SynthSpec, Topology};

fn assert_basic(spec: SynthSpec, expected_buses: usize, expected_min_branches: usize) {
    let a = generate(&spec);
    let b = generate(&spec);
    assert_eq!(a.buses(), b.buses());
    assert_eq!(a.branches(), b.branches());
    assert_eq!(a.buses().len(), expected_buses);
    assert!(a.branches().len() >= expected_min_branches);
    assert_eq!(a.buses()[0].kind, powerio::BusType::Ref);
}

#[test]
fn tree_generation_is_deterministic() {
    assert_basic(
        SynthSpec { topology: Topology::Tree, n: 16, ..SynthSpec::default() },
        16,
        15,
    );
}

#[test]
fn lattice_rounds_to_a_square() {
    assert_basic(
        SynthSpec { topology: Topology::Lattice2D, n: 10, ..SynthSpec::default() },
        16,
        24,
    );
}

#[test]
fn meshed_generation_adds_cross_edges() {
    assert_basic(
        SynthSpec { topology: Topology::PegaseLike, n: 30, ..SynthSpec::default() },
        30,
        39,
    );
}

#[test]
fn different_seeds_change_the_case() {
    let a = generate(&SynthSpec { seed: 1, ..SynthSpec::default() });
    let b = generate(&SynthSpec { seed: 2, ..SynthSpec::default() });
    assert_ne!(a.branches(), b.branches());
}
