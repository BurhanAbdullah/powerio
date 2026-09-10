//! BMOPFTools element geometry and schema-valid GeoJSON storage.
use crate::geo::{CoordinateSpace, DistCoordsKind, DistGeoMeta, DistLocation};
use crate::model::MulticonductorNetwork;
use serde_json::{Value, json};

fn point(value: &Value) -> Option<DistLocation> {
    let p = value.as_array()?;
    if p.len() != 2 {
        return None;
    }
    let (x, y) = (p[0].as_f64()?, p[1].as_f64()?);
    (x.is_finite() && y.is_finite()).then_some(DistLocation {
        x,
        y,
        kind: Some(DistCoordsKind::Source),
    })
}

fn read_meta(meta: &Value, collection: &Value) -> Result<DistGeoMeta, String> {
    if let Some(declared) = collection.get("powerio_geo") {
        return serde_json::from_value(declared.clone())
            .map_err(|error| format!("invalid powerio_geo metadata: {error}"));
    }
    let crs = meta.get("crs").or_else(|| collection.get("crs"));
    let crs = match crs {
        None => "EPSG:4326",
        Some(value) => value
            .as_str()
            .or_else(|| value.pointer("/properties/name").and_then(Value::as_str))
            .filter(|name| !name.trim().is_empty())
            .ok_or_else(|| "invalid geometry coordinate reference system".to_owned())?,
    };
    let wgs84 = matches!(
        crs.to_ascii_uppercase().as_str(),
        "EPSG:4326" | "OGC:CRS84" | "WGS84" | "URN:OGC:DEF:CRS:OGC:1.3:CRS84"
    );
    let fallback_space = if wgs84 {
        CoordinateSpace::Geographic {
            crs: Some(crs.into()),
        }
    } else {
        CoordinateSpace::Projected {
            crs: Some(crs.into()),
        }
    };
    Ok(DistGeoMeta {
        space: fallback_space,
        kind: Some(DistCoordsKind::Source),
    })
}

pub(super) fn read(
    doc: &serde_json::Map<String, Value>,
    net: &mut MulticonductorNetwork,
    diagnostics: &mut crate::collect::Diagnostics,
) {
    let collection = doc
        .get("extras")
        .and_then(|v| v.get("geojson"))
        .unwrap_or(&Value::Null);
    let geo_meta = match read_meta(doc.get("meta").unwrap_or(&Value::Null), collection) {
        Ok(meta) => meta,
        Err(reason) => {
            diagnostics.push(
                &crate::diagnostics::codes::READ_BMOPF_RETAINED_SOURCE_ONLY,
                format!("{reason}; geometry retained without assigning coordinates"),
            );
            return;
        }
    };
    if !collection.is_null()
        && (collection["type"] != "FeatureCollection" || !collection["features"].is_array())
    {
        diagnostics.push(
            &crate::diagnostics::codes::READ_BMOPF_RETAINED_SOURCE_ONLY,
            "invalid GeoJSON feature collection; retained without assigning its coordinates",
        );
    }
    let geographic = matches!(geo_meta.space, CoordinateSpace::Geographic { .. });
    let buses: std::collections::HashMap<_, _> = net
        .buses()
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id.clone(), i))
        .collect();
    let lines: std::collections::HashMap<_, _> = net
        .lines()
        .iter()
        .enumerate()
        .map(|(i, l)| (l.name.clone(), i))
        .collect();
    let mut changed = false;
    let mut apply = |kind: &str, id: &str, geo: &Value| {
        let mut valid = false;
        match (kind, geo["type"].as_str()) {
            ("bus", Some("Point")) => {
                if let Some(location) = point(&geo["coordinates"])
                    && (!geographic
                        || ((-180.0..=180.0).contains(&location.x)
                            && (-90.0..=90.0).contains(&location.y)))
                    && let Some(&row) = buses.get(id)
                {
                    net.buses_mut()[row].location = Some(location);
                    valid = true;
                }
            }
            ("line", Some("LineString")) => {
                if let Some(points) = geo["coordinates"]
                    .as_array()
                    .and_then(|items| items.iter().map(point).collect::<Option<Vec<_>>>())
                    && points.len() >= 2
                    && (!geographic
                        || points.iter().all(|p| {
                            (-180.0..=180.0).contains(&p.x) && (-90.0..=90.0).contains(&p.y)
                        }))
                    && let Some(&row) = lines.get(id)
                {
                    net.lines_mut()[row].route = Some(points);
                    valid = true;
                }
            }
            _ => {}
        }
        if valid {
            changed = true;
        } else {
            diagnostics.push(&crate::diagnostics::codes::READ_BMOPF_RETAINED_SOURCE_ONLY,format!("{kind} {id}: geometry is invalid or names no matching element; retained without assigning coordinates"));
        }
    };
    if let Some(features) = collection["features"]
        .as_array()
        .filter(|_| collection["type"] == "FeatureCollection")
    {
        for f in features {
            if let (Some(kind), Some(id)) = (
                f["properties"]["kind"].as_str(),
                f["properties"]["id"].as_str(),
            ) {
                apply(kind, id, &f["geometry"]);
            } else {
                apply("feature", "(missing kind or id)", &f["geometry"]);
            }
        }
    }
    for kind in ["bus", "line"] {
        if let Some(items) = doc.get(kind).and_then(Value::as_object) {
            for (id, element) in items {
                if let Some(geo) = element.get("geo") {
                    apply(kind, id, geo);
                }
            }
        }
    }
    if changed {
        *net.geo_mut() = Some(geo_meta);
    }
}

pub(super) fn collection(net: &MulticonductorNetwork) -> Option<Value> {
    let default_meta = DistGeoMeta {
        space: CoordinateSpace::Unknown,
        kind: None,
    };
    let meta = net.geo().as_ref().unwrap_or(&default_meta);
    let crs = match &meta.space {
        CoordinateSpace::Geographic { crs } => Some(crs.as_deref().unwrap_or("EPSG:4326")),
        CoordinateSpace::Projected { crs } => crs.as_deref(),
        _ => None,
    };
    let mut features = Vec::new();
    for bus in net.buses() {
        if let Some(p) = bus.location.filter(|p| p.x.is_finite() && p.y.is_finite()) {
            features.push(json!({"type":"Feature","properties":{"kind":"bus","id":bus.id},"geometry":{"type":"Point","coordinates":[p.x,p.y]}}));
        }
    }
    for line in net.lines() {
        if let Some(route) = line
            .route
            .as_ref()
            .filter(|r| r.len() >= 2 && r.iter().all(|p| p.x.is_finite() && p.y.is_finite()))
        {
            features.push(json!({"type":"Feature","properties":{"kind":"line","id":line.name,"bus_from":line.bus_from,"bus_to":line.bus_to},"geometry":{"type":"LineString","coordinates":route.iter().map(|p|[p.x,p.y]).collect::<Vec<_>>()}}));
        }
    }
    if features.is_empty() {
        return None;
    }
    features.sort_by(|a, b| {
        let key = |v: &Value| format!("{}:{}", v["properties"]["kind"], v["properties"]["id"]);
        key(a).cmp(&key(b))
    });
    let mut out = json!({"type":"FeatureCollection","features":features,"powerio_geo":meta});
    if let Some(crs) = crs.filter(|value| *value != "EPSG:4326") {
        out["crs"] = json!(crs);
    }
    Some(out)
}
