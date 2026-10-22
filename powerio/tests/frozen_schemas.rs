//! The committed current PowerIO IR schema matches the implementation, and
//! the historical schema archive remains intact.

use std::path::Path;

const SCHEMA_ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/schema");
const CURRENT_SCHEMA: &str = "pio-ir/2/0.11.6/schema.json";

fn assert_monitor_schema_preserved(
    published: &serde_json::Value,
    current: &serde_json::Value,
    path: &str,
) {
    fn discriminator(value: &serde_json::Value) -> Option<&serde_json::Value> {
        let kind = &value["properties"]["kind"]["const"];
        if kind.is_string() {
            return Some(kind);
        }
        let ty = &value["properties"]["type"]["const"];
        ty.is_string().then_some(ty)
    }

    match (published, current) {
        (serde_json::Value::Object(old), serde_json::Value::Object(new)) => {
            for (name, value) in old {
                let current_value = new.get(name).unwrap_or_else(|| {
                    panic!("{path}.{name}: published schema member was removed")
                });
                if name == "properties" {
                    let old_properties = value.as_object().expect("published properties object");
                    let new_properties = current_value
                        .as_object()
                        .expect("current properties object");
                    let required = current["required"].as_array();
                    for (property, old_schema) in old_properties {
                        let new_schema = new_properties.get(property).unwrap_or_else(|| {
                            panic!("{path}.properties.{property}: published property was removed")
                        });
                        assert_monitor_schema_preserved(
                            old_schema,
                            new_schema,
                            &format!("{path}.properties.{property}"),
                        );
                    }
                    for property in new_properties.keys() {
                        if !old_properties.contains_key(property) {
                            assert!(
                                !required.is_some_and(|fields| fields
                                    .iter()
                                    .any(|field| field == property)),
                                "{path}.properties.{property}: additive property became required"
                            );
                            // A new optional property is backward compatible.
                        }
                    }
                } else {
                    assert_monitor_schema_preserved(
                        value,
                        current_value,
                        &format!("{path}.{name}"),
                    );
                }
            }
            for name in new.keys() {
                if !old.contains_key(name) {
                    assert!(
                        matches!(
                            name.as_str(),
                            "description"
                                | "title"
                                | "examples"
                                | "default"
                                | "deprecated"
                                | "readOnly"
                                | "writeOnly"
                                | "$comment"
                        ),
                        "{path}.{name}: unapproved schema keyword was added"
                    );
                }
            }
        }
        (serde_json::Value::Array(old), serde_json::Value::Array(new))
            if path.ends_with(".oneOf") || path.ends_with(".anyOf") =>
        {
            let is_tagged = !old.is_empty() && old.iter().all(|item| discriminator(item).is_some());
            if is_tagged {
                for item in old {
                    let key = discriminator(item).expect("tagged published variant");
                    let matching = new
                        .iter()
                        .find(|candidate| discriminator(candidate) == Some(key))
                        .unwrap_or_else(|| panic!("{path}: published variant {key} was removed"));
                    assert_monitor_schema_preserved(item, matching, path);
                }
                for item in new {
                    assert!(
                        discriminator(item).is_some(),
                        "{path}: additive variant must have a kind/type discriminator"
                    );
                }
            } else {
                assert_eq!(old, new, "{path}: non-discriminated alternatives changed");
            }
        }
        (serde_json::Value::Array(old), serde_json::Value::Array(new))
            if path.ends_with(".enum") =>
        {
            for value in old {
                assert!(
                    new.contains(value),
                    "{path}: published enum value {value} was removed"
                );
            }
        }
        _ => assert_eq!(published, current, "{path}: published schema changed"),
    }
}

fn read_schema_file(relative: &str) -> String {
    let path = Path::new(SCHEMA_ROOT).join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{} is not checked in: {error}. Generate it with \
             `cargo run -p powerio --example generate_schemas --features schema -- docs/schema`",
            path.display()
        )
    })
}

/// The directory holds one PowerIO IR history, not separate package and module
/// schema families.
#[test]
fn the_schema_directory_contains_the_documented_powerio_ir_history() {
    fn collect_files(root: &Path, path: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                collect_files(root, &path, out);
            } else {
                out.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }

    let root = Path::new(SCHEMA_ROOT);
    let mut files = Vec::new();
    collect_files(root, root, &mut files);
    files.sort();
    assert_eq!(
        files,
        [
            "README.md",
            "pio-ir/0.1/schema.json",
            "pio-ir/0.2/schema.json",
            "pio-ir/0.9/schema.json",
            "pio-ir/1/schema.json",
            "pio-ir/2/0.11.1/schema.json",
            "pio-ir/2/0.11.3/schema.json",
            "pio-ir/2/0.11.4/schema.json",
            "pio-ir/2/0.11.5/schema.json",
            CURRENT_SCHEMA,
            "pio-ir/2/schema.json",
        ]
    );
}

/// Historical files preserve the identifiers that appeared in their releases.
#[test]
fn historical_schemas_preserve_their_original_identifiers() {
    for (path, expected_id) in [
        (
            "pio-ir/0.1/schema.json",
            "https://powerio.dev/schema/pio-package/0.1",
        ),
        (
            "pio-ir/0.2/schema.json",
            "https://powerio.dev/schema/pio-package/0.2",
        ),
        (
            "pio-ir/0.9/schema.json",
            "https://powerio.dev/schema/pio-package/0.9/schema.json",
        ),
        (
            "pio-ir/1/schema.json",
            "https://powerio.dev/schema/pio-module/1/schema.json",
        ),
        (
            "pio-ir/2/schema.json",
            "https://powerio.dev/schema/pio-ir/2/schema.json",
        ),
        (
            "pio-ir/2/0.11.1/schema.json",
            "https://powerio.dev/schema/pio-ir/2/0.11.1/schema.json",
        ),
        (
            "pio-ir/2/0.11.3/schema.json",
            "https://powerio.dev/schema/pio-ir/2/0.11.3/schema.json",
        ),
        (
            "pio-ir/2/0.11.4/schema.json",
            "https://powerio.dev/schema/pio-ir/2/0.11.4/schema.json",
        ),
        (
            "pio-ir/2/0.11.5/schema.json",
            "https://powerio.dev/schema/pio-ir/2/0.11.5/schema.json",
        ),
    ] {
        let schema: serde_json::Value = serde_json::from_str(&read_schema_file(path)).unwrap();
        assert_eq!(schema["$id"], expected_id, "historical schema {path}");
    }
}

/// The current schema uses the public PowerIO IR identity and version.
#[test]
fn the_current_powerio_ir_schema_is_committed() {
    let schema: serde_json::Value =
        serde_json::from_str(&read_schema_file(CURRENT_SCHEMA)).unwrap();
    assert_eq!(schema["$id"], powerio::IR_SCHEMA_ID);
    assert_eq!(
        schema["properties"]["schema"]["const"],
        powerio::IR_SCHEMA_NAME
    );
    assert_eq!(
        schema["properties"]["version"]["const"],
        powerio::IR_VERSION
    );
}

/// Each earlier IR version 2 schema file is checked against the current one.
/// A release can add structural types and definitions, and a list of allowed
/// string values (such as the distribution source formats) can gain values.
/// Every other existing definition, and the document rules, stay byte for
/// byte the same, so a document an earlier 0.11 release wrote still reads
/// with the same meaning. Renaming or reshaping an existing type fails here.
#[test]
fn ir_version_2_schemas_only_add() {
    for earlier in [
        "pio-ir/2/0.11.5/schema.json",
        "pio-ir/2/0.11.4/schema.json",
        "pio-ir/2/schema.json",
        "pio-ir/2/0.11.1/schema.json",
        "pio-ir/2/0.11.3/schema.json",
    ] {
        let mut published: serde_json::Value =
            serde_json::from_str(&read_schema_file(earlier)).unwrap();
        let mut current: serde_json::Value =
            serde_json::from_str(&read_schema_file(CURRENT_SCHEMA)).unwrap();
        let published_defs = published.as_object_mut().unwrap().remove("$defs").unwrap();
        let current_defs = current.as_object_mut().unwrap().remove("$defs").unwrap();
        published.as_object_mut().unwrap().remove("$id");
        current.as_object_mut().unwrap().remove("$id");
        assert_eq!(published, current, "{earlier}: document rules changed");

        for (name, definition) in published_defs.as_object().unwrap() {
            if name == "StoredValue" {
                for variant in definition["oneOf"].as_array().unwrap() {
                    let type_name = &variant["properties"]["type"]["const"];
                    let matching = current_defs[name]["oneOf"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|candidate| candidate["properties"]["type"]["const"] == *type_name);
                    assert_eq!(
                        matching,
                        Some(variant),
                        "{earlier}: type {type_name} changed"
                    );
                }
            } else if name == "SourceFormat" {
                // The public format enum is nonexhaustive. Adding one new
                // format must leave every existing spelling and schema rule
                // intact; this is not permission to change arbitrary records.
                let mut without_sincal = current_defs[name].clone();
                let alternatives = without_sincal["oneOf"].as_array_mut().unwrap();
                let before = alternatives.len();
                alternatives.retain(|v| v["const"] != "sincal");
                assert_eq!(before, alternatives.len() + 1);
                assert_eq!(
                    &without_sincal, definition,
                    "{earlier}: existing format changed"
                );
                let sincal = current_defs[name]["oneOf"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|v| v["const"] == "sincal")
                    .unwrap();
                assert_eq!(sincal["type"], "string");
            } else if name == "VoltageSource" {
                // IR 2 also admits a distinctly tagged nested source type.
                // Keep the legacy branch byte-for-byte equivalent as JSON;
                // do not generally permit arbitrary changes to record unions.
                let branches = current_defs[name]["anyOf"].as_array().unwrap();
                assert_eq!(branches.len(), 2);
                assert_eq!(&branches[0], definition, "{earlier}: legacy source changed");
                assert_eq!(branches[1]["$ref"], "#/$defs/ReferencedSourceWire");
                let new_source = &current_defs["ReferencedSourceWire"]["oneOf"][0];
                assert_eq!(
                    new_source["properties"]["type"]["const"],
                    "powerio.ReferencedVoltageSource"
                );
                assert_eq!(new_source["required"], serde_json::json!(["type", "value"]));
            } else if name == "MonitorScope" || name == "MonitorStatement" {
                assert_monitor_schema_preserved(
                    definition,
                    &current_defs[name],
                    &format!("{earlier}: record {name}"),
                );
            } else if let Some(values) = definition["enum"].as_array() {
                // A list of allowed values keeps every value it had; it may
                // gain values, and nothing else about it changes.
                let current = &current_defs[name];
                let current_values = current["enum"].as_array().unwrap_or_else(|| {
                    panic!("{earlier}: {name} is no longer a list of allowed values")
                });
                for value in values {
                    assert!(
                        current_values.contains(value),
                        "{earlier}: {name} no longer allows {value}"
                    );
                }
                let without_values = |definition: &serde_json::Value| {
                    let mut definition = definition.clone();
                    definition.as_object_mut().unwrap().remove("enum");
                    definition
                };
                assert_eq!(
                    without_values(current),
                    without_values(definition),
                    "{earlier}: {name} changed beyond gaining values"
                );
            } else {
                assert_eq!(
                    &current_defs[name], definition,
                    "{earlier}: definition {name} changed"
                );
            }
        }
    }
}

#[test]
fn monitor_schema_additions_preserve_published_variants() {
    let published: serde_json::Value =
        serde_json::from_str(&read_schema_file("pio-ir/2/0.11.5/schema.json")).unwrap();
    let current: serde_json::Value =
        serde_json::from_str(&read_schema_file(CURRENT_SCHEMA)).unwrap();
    for name in ["MonitorScope", "MonitorStatement"] {
        assert_monitor_schema_preserved(&published["$defs"][name], &current["$defs"][name], name);
    }
}

#[test]
fn monitor_schema_guard_rejects_new_restrictions_and_allows_additive_shapes() {
    let published = serde_json::json!({"type": "string"});
    for (keyword, value) in [
        ("maxLength", serde_json::json!(0)),
        ("pattern", serde_json::json!("^")),
        ("maximum", serde_json::json!(0)),
        ("maxItems", serde_json::json!(0)),
    ] {
        let mut narrowed = published.clone();
        narrowed
            .as_object_mut()
            .unwrap()
            .insert(keyword.to_owned(), value);
        assert!(
            std::panic::catch_unwind(|| {
                assert_monitor_schema_preserved(&published, &narrowed, "probe");
            })
            .is_err(),
            "new validation restriction {keyword} must not be silently accepted"
        );
    }

    let published_object = serde_json::json!({
        "type": "object",
        "properties": {"kind": {"const": "old", "type": "string"}},
        "required": ["kind"]
    });
    let additive_object = serde_json::json!({
        "type": "object",
        "properties": {
            "kind": {"const": "old", "type": "string"},
            "label": {"type": "string"}
        },
        "required": ["kind"]
    });
    assert_monitor_schema_preserved(&published_object, &additive_object, "probe");

    let restricted_object = serde_json::json!({
        "type": "object",
        "properties": {
            "kind": {"const": "old", "type": "string"},
            "label": {"type": "string"}
        },
        "required": ["kind", "label"]
    });
    assert!(
        std::panic::catch_unwind(|| {
            assert_monitor_schema_preserved(&published_object, &restricted_object, "probe");
        })
        .is_err(),
        "a newly added property must not become required"
    );

    let old_variants = serde_json::json!({"oneOf": [
        {"properties": {"kind": {"const": "old", "type": "string"}}, "required": ["kind"], "type": "object"}
    ]});
    let extended_variants = serde_json::json!({"oneOf": [
        {"properties": {"kind": {"const": "old", "type": "string"}}, "required": ["kind"], "type": "object"},
        {"properties": {"kind": {"const": "new", "type": "string"}}, "required": ["kind"], "type": "object"}
    ]});
    assert_monitor_schema_preserved(&old_variants, &extended_variants, "probe");
}

/// Every property of every value kind in the current document carries a type,
/// a `$ref`, or a composed schema.
#[test]
fn every_powerio_ir_property_is_typed() {
    fn walk(node: &serde_json::Value, path: &str, bad: &mut Vec<String>) {
        match node {
            serde_json::Value::Object(map) => {
                if map.get("type") == Some(&serde_json::Value::String("object".into()))
                    && let Some(serde_json::Value::Object(props)) = map.get("properties")
                {
                    for (name, prop) in props {
                        if let serde_json::Value::Object(keys) = prop
                            && keys.keys().all(|key| key == "description")
                        {
                            bad.push(format!("{path}.{name}"));
                        }
                    }
                }
                for (key, value) in map {
                    walk(value, &format!("{path}.{key}"), bad);
                }
            }
            serde_json::Value::Array(items) => {
                for (index, value) in items.iter().enumerate() {
                    walk(value, &format!("{path}[{index}]"), bad);
                }
            }
            _ => {}
        }
    }

    let schema: serde_json::Value =
        serde_json::from_str(&read_schema_file(CURRENT_SCHEMA)).unwrap();
    let mut bad = Vec::new();
    walk(&schema, "$", &mut bad);
    assert!(bad.is_empty(), "untyped properties: {bad:?}");
}
