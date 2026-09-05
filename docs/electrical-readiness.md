# Electrical readiness contract

`powerio_dist::check_electrical_readiness` is a fail-closed structural gate for multiconductor distribution models.

The purpose is not to replace a numerical solver. It answers a narrower question first:

> **Does the typed model contain enough internally consistent electrical structure to be safely consumed without inventing missing electrical data?**

## Blocking conditions

The audit currently rejects:

- non-finite or non-positive base frequency;
- duplicate bus identifiers (case-insensitive);
- duplicate linecode identifiers (case-insensitive);
- linecodes referenced by a line but absent from the model;
- non-finite or non-positive line lengths;
- unknown line endpoints;
- terminal names absent from their endpoint buses;
- unequal from/to terminal-map arity;
- terminal-map conductor counts that disagree with the linecode;
- impedance matrices whose dimensions disagree with the declared conductor count;
- non-finite impedance-matrix entries.

## Fail-closed semantics

An unresolved linecode is deliberately represented as a blocker. The readiness audit does **not** synthesize a linecode, infer impedance values, or fabricate a conductor count. This is important for OpenDSS inputs because a parser may encounter a `Line` object whose geometry or linecode is deferred elsewhere in the source.

The intended pipeline is:

```text
source/parser
    |
    v
canonical multiconductor model
    |
    v
check_electrical_readiness()
    |
    +---- errors ----> refuse numerical/emission consumer
    |
    `---- clean -----> transformation / matrix / writer
```

This keeps structural validation separate from format parsing while giving downstream consumers a common, reusable preflight contract.

## Why this is separate from transformation checks

A transformation such as multiconductor-to-balanced lowering has its own domain-specific assumptions. Readiness checks are lower-level invariants that should remain useful even when the caller is not performing that transformation—for example, before matrix assembly or cross-format emission.

The API therefore returns structured findings rather than a preformatted error string. Callers can render them for CLI, Python, or UI consumers without coupling the model layer to one presentation format.
