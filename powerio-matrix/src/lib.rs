//! Sparse matrix and graph projections from PowerIO networks.
//!
//! Outputs include signed incidence, weighted bus Laplacian, MATPOWER Bp/Bpp,
//! Y bus, PTDF, LODF, adjacency, LACPF, and petgraph views. Calculations take the
//! dense [`IndexedNetwork`] view of a [`BalancedNetwork`]. Parsing and emitting
//! belong to the top level `powerio` facade; this crate owns derived matrix and
//! graph calculations.

// Re-export the balanced model types used by matrix signatures. Parsing,
// emitting, conversion, and display operations stay on their owning crate and
// the top level facade. `Error` and `Result` are this crate's own: the variants
// below are raised here and nowhere in the hub.
pub use powerio_tx::{
    BalancedNetwork, Branch, Bus, BusId, BusType, ConnectivityReport, Extras, GenCost, Generator,
    Hvdc, IndexCore, IndexedNetwork, Load, POWER_MODELS_ANGLE_BOUND_PAD, Shunt, SourceFormat,
    Storage,
};

// Internal compatibility paths used throughout the matrix implementation.
pub(crate) use powerio_tx::{indexed, network};

pub mod diagnostics;
pub mod error;
pub use error::{ElementCounts, Error, PiecewiseCostInvalidity, Result, ScenarioMismatch};

/// Compressed sparse row matrix used by the projection calculations.
pub type SparseMatrix = sprs::CsMat<f64>;

mod ac_jacobian;
mod acopf;
mod dc_operators;
mod dcopf;
pub mod io;
pub mod matrix;
mod opf;
pub mod pipeline;

pub use ac_jacobian::{PowerFlowJacobian, VoltageCoordinates, calc_power_flow_jacobian};
pub use acopf::{
    AcBranchData, AcBusData, AcGeneratorData, AcOpfAssemblyOptions, AcOpfPreparation,
    AcPfAssemblyOptions, AcPfBusData, AcPfGeneratorData, AcPfPreparation, AcStorageData,
    NodalAcGeneratorData, PreparedAcBusSpecification, build_ac_opf_preparation,
    build_ac_pf_preparation,
};
pub use dc_operators::{DcOperators, ReferenceConstrainedSystem};
pub use dcopf::{
    DcBranchParameters, DcGeneratorParameters, DcOpfAssemblyOptions, DcOpfBundleMetadata,
    DcOpfBundleOptions, DcOpfMatrices, DcOpfOutputs, DcOpfPreparation, NodalGeneratorParameters,
    Units, build_dc_opf_preparation, calc_dc_opf_matrices, emit_dcopf_bundle,
};
pub use opf::{AnalysisBranchSource, PiecewiseLinearCost, PreparedObjective};

pub use matrix::multiconductor::{
    AugmentedSystem, DistNode, MulticonductorAdmittance, MulticonductorNodeIndex, NodeRef,
    calc_multiconductor_admittance_matrix,
};
pub use matrix::{
    BranchSusceptanceFormula, BuildOptions, GroundedIndexMap, MatrixStats, Scheme,
    SensitivityMatrices, SensitivityMatrixMetadata, SensitivityMetadata, SensitivityOptions,
    SensitivitySolver, SensitivitySolverPath, ZeroImpedanceRule, ZeroImpedanceSkips,
    calc_adjacency_matrix, calc_admittance_matrix, calc_bdoubleprime_matrix, calc_bprime_matrix,
    calc_diagonal, calc_lacpf_matrix, calc_lodf, calc_ptdf, calc_ptdf_lodf,
    calc_ptdf_lodf_with_options, calc_reference_indicator, calc_susceptance_diagonal,
    calc_unit_vector, calc_weighted_laplacian, calc_zero_impedance_skips, check_sddm, ground_at,
    ground_at_each,
};
pub use pipeline::{
    MatrixKind, Pipeline, PipelineOutputs, RhsKind, calc_matrix, calc_matrix_stats_for_kind,
    calc_zero_impedance_skips_for_kind, sanitize_stem, select_zero_impedance_rule_for_kind,
};

#[cfg(feature = "gridfm")]
pub use io::gridfm::{
    GridfmDataset, GridfmOptions, GridfmOutputs, GridfmSnapshot, GridfmTables, build_gridfm_batch,
    build_gridfm_dataset, emit_gridfm_batch, emit_gridfm_dataset, number_snapshots,
    to_gridfm_record_batches, to_gridfm_record_batches_single,
};
