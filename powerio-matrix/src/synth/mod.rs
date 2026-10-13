//! Compatibility re-exports for the historical `powerio_matrix::synth` API.
//!
//! The implementation now lives in the standalone `powerio-synth` crate,
//! but existing downstream imports must continue to resolve in this module.

pub use powerio_synth::{
    SynthSpec, Topology, generate, generate_lattice, generate_pegase_like, generate_tree,
};
