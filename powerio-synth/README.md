# powerio-synth

Deterministic synthetic balanced-network generation for PowerIO.

The crate emits the public `powerio::BalancedNetwork` model and is intentionally
independent of matrix construction. Synthetic cases can therefore be reused by
matrix tests, OPF preparation, serialization, benchmarking, and downstream
validation without making `powerio-matrix` own the generator implementation.

## Example

```rust
use powerio_synth::{generate, SynthSpec, Topology};

let case = generate(&SynthSpec {
    topology: Topology::Lattice2D,
    n: 64,
    ..SynthSpec::default()
});

assert_eq!(case.buses().len(), 64);
```

Generation is deterministic: the same `SynthSpec` and seed produce the same
case.

## Topologies

- `Tree`: random recursive spanning tree.
- `Lattice2D`: square 2-D grid, rounding `n` up to the nearest square.
- `PegaseLike`: spanning tree plus approximately `n / 3` random cross-edges.
