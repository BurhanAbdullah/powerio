//! Deterministic synthetic power-system cases for PowerIO tests and studies.
//!
//! The generator is deliberately independent of matrix construction: it emits
//! the public [`powerio::BalancedNetwork`] model, so the same synthetic case can
//! feed matrix, OPF, serialization, and downstream validation workflows.
//!
//! Identical specifications and seeds produce identical networks.

mod lattice;
mod pegase_like;
mod tree;

pub use lattice::generate_lattice;
pub use pegase_like::generate_pegase_like;
pub use tree::generate_tree;

use serde::{Deserialize, Serialize};

/// Supported synthetic topology families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Topology {
    /// Random recursive spanning tree.
    Tree,
    /// Square two-dimensional lattice; `n` is rounded up to a square.
    Lattice2D,
    /// Spanning tree plus approximately `n / 3` random cross-edges.
    PegaseLike,
}

/// Parameters for deterministic synthetic case generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SynthSpec {
    pub topology: Topology,
    pub n: usize,
    /// Branch series resistance-to-reactance ratio.
    pub r_over_x: f64,
    /// Mean branch reactance in per-unit.
    pub mean_x: f64,
    /// Deterministic random seed.
    pub seed: u64,
}

impl Default for SynthSpec {
    fn default() -> Self {
        Self {
            topology: Topology::Tree,
            n: 64,
            r_over_x: 0.1,
            mean_x: 0.05,
            seed: 0x00C0_FFEE,
        }
    }
}

/// Generate a synthetic balanced network from a specification.
#[must_use]
pub fn generate(spec: &SynthSpec) -> powerio::BalancedNetwork {
    match spec.topology {
        Topology::Tree => generate_tree(spec),
        Topology::Lattice2D => generate_lattice(spec),
        Topology::PegaseLike => generate_pegase_like(spec),
    }
}
