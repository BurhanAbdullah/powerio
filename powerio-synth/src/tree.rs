//! Random spanning tree topology.

use powerio::{BalancedNetwork, Branch, Bus, BusId, BusType};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::SynthSpec;

pub fn generate_tree(spec: &SynthSpec) -> BalancedNetwork {
    let n = spec.n.max(2);
    let mut rng = ChaCha8Rng::seed_from_u64(spec.seed);
    let buses = make_buses(n);
    let mut branches = Vec::with_capacity(n - 1);
    for k in 1..n {
        let parent = rng.random_range(0..k);
        branches.push(make_branch(parent + 1, k + 1, spec, &mut rng));
    }
    net(format!("synth_tree_n{n}"), buses, branches)
}

pub(crate) fn net(name: String, buses: Vec<Bus>, branches: Vec<Branch>) -> BalancedNetwork {
    BalancedNetwork::in_memory(name, 100.0, buses, branches)
}

pub(crate) fn make_buses(n: usize) -> Vec<Bus> {
    let mut buses: Vec<Bus> = (0..n).map(|i| make_bus(i + 1)).collect();
    buses[0].kind = BusType::Ref;
    buses
}

pub(crate) fn make_bus(id: usize) -> Bus {
    Bus::new(BusId(id), BusType::Pq, 345.0)
}

pub(crate) fn make_branch(
    from: usize,
    to: usize,
    spec: &SynthSpec,
    rng: &mut ChaCha8Rng,
) -> Branch {
    let log_low = (spec.mean_x * 0.5).ln();
    let log_high = (spec.mean_x * 2.0).ln();
    let x = rng.random_range(log_low..log_high).exp().max(1e-6);
    Branch::new(BusId(from), BusId(to), spec.r_over_x * x, x)
}
