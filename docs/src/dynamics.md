# Dynamics and DYR

## Representation decision

PSS/E `.dyr` is a dynamic-model file, not a power-flow network. PowerIO keeps
that distinction explicit: `.raw` is parsed by `powerio::parse` to a
`PioValue::BalancedNetwork`, while `.dyr` is parsed by `powerio::parse_dyr`
to a standalone `DynamicCase`. Pair the two with `DynamicStudy`; do not insert
dynamic records into `BalancedNetwork` or treat the raw model key as a typed
generator reference.

This separate Rust API is intentional. It establishes a stable boundary for
loss-preserving DYR records and generic bus association without prematurely
committing the cross-language `PioValue`, PowerIO IR, C ABI, Julia, or Python
representations to an incomplete dynamic-model schema.

## Parse and pair

```rust,ignore
use powerio::{parse, parse_dyr, DynamicStudy, PioValue};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let raw = parse("case.raw")?;
    let PioValue::BalancedNetwork(network) = raw.value() else {
        return Err("case.raw did not produce a balanced network".into());
    };

    let dyr_text = std::fs::read_to_string("case.dyr")?;
    let dynamics = parse_dyr(&dyr_text)?;
    let study = DynamicStudy::new(network.clone(), dynamics);

    for unresolved in study.unresolved_buses() {
        eprintln!(
            "line {}: {} references missing bus {}",
            unresolved.line, unresolved.model_name, unresolved.bus_id
        );
    }
    Ok(())
}
```

`DynamicCase::records()` returns records in source order. Each record exposes
its bus ID, raw model name, optional device key, ordered parameter tokens,
source line, and record spelling. `DynamicCase::to_dyr_string()` returns the
original text exactly, including comments and formatting. `parse_dyr_bytes`
rejects invalid UTF-8 rather than replacing bytes.

## Linking guarantees and limitations

`DynamicStudy::unresolved_buses()` verifies only whether each generic DYR
bus ID occurs in the paired network. It does **not** assert that the optional
`device_id` refers to a generator, load, or other particular equipment: the
meaning of that field and the association rule depend on the model family.
Unknown and user-defined models are preserved rather than rejected or
silently dropped. Parameters remain strings until a schema-backed model view
exists.

Not implemented yet: typed machine/exciter/governor/stabilizer models,
model-aware device association, DYR-to-DYD/SWI/PowerWorld AUX conversion,
transient simulation, and dynamics exposure through PowerIO IR, C ABI, Julia,
or Python. This is the initial public API boundary, not a claim that those
features are complete.
