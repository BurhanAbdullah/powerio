//! PEGASE-like meshed topology: a spanning tree plus random cross-edges.

use powerio::BalancedNetwork;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::SynthSpec;
use crate::tree::{make_branch, make_buses, net};

pub fn generate_pegase_like(spec: &SynthSpec) -> BalancedNetwork {
    let n = spec.n.max(2);
    let mut rng = ChaCha8Rng::seed_from_u64(spec.seed);
    let buses = make_buses(n);
    let mut branches = Vec::with_capacity((n as f64 * 1.3) as usize);

    for k in 1..n {
        let parent = rng.random_range(0..k);
        branches.push(make_branch(parent + 1, k + 1, spec, &mut rng));
    }

    for _ in 0..n / 3 {
        let i = rng.random_range(0..n);
        let mut j = rng.random_range(0..n);
        if i == j {
            j = (j + 1) % n;
        }
        branches.push(make_branch(i + 1, j + 1, spec, &mut rng));
    }

    net(format!("synth_pegase_n{n}"), buses, branches)
}
