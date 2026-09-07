use powerio_dist::dss::to_network_from_raw;
use powerio_dist::readiness::audit_electrical_readiness;

#[test]
fn geometry_lines_are_not_materialized_from_factory_defaults() {
    let text = r#"
clear
new circuit.c basekv=4.16 phases=3 bus1=sourcebus
new wiredata.w rac=0.1859 gmr=0.0313 radius=0.4635 runits=mi gmrunits=ft radunits=in
new linegeometry.g nconds=4 nphases=3 reduce=no
~ cond=1 wire=w x=2.5 h=29 units=ft
~ cond=2 wire=w x=0 h=29 units=ft
~ cond=3 wire=w x=7 h=29 units=ft
~ cond=4 wire=w x=4 h=25 units=ft
new line.l bus1=sourcebus bus2=b1 geometry=g length=1 units=m
"#;
    let source = powerio_core::Source::text(text.into(), Some("case.dss".into()));
    let module = powerio_dist::dss::parse_raw_from_source(&source).unwrap();
    let (net, diagnostics) = to_network_from_raw(&module);

    assert!(net.lines().is_empty(), "geometry line must not become a typed line");
    assert!(net.line_codes().iter().all(|c| !c.name.starts_with("_line_l")));
    assert!(diagnostics.iter().any(|d| d.code.as_str() == "READ.DSS.GEOMETRY_UNRESOLVED"));
    assert!(net.untyped_objects().iter().any(|o| o.class == "line" && o.name == "l"));

    let readiness = audit_electrical_readiness(&net);
    assert!(readiness.blockers().any(|b| b.code == "READINESS.DSS.GEOMETRY_DEFERRED"));
}

#[test]
fn one_conductor_geometry_is_not_fabricated_as_three_phase() {
    let text = r#"
clear
new circuit.c basekv=19.1 phases=1 bus1=sourcebus
new wiredata.w rac=1.093 gmr=0.00296 radius=0.00318 runits=km gmrunits=m radunits=m
new linegeometry.g nconds=1 nphases=1 reduce=no
~ cond=1 wire=w x=0 h=8.5 units=m
new line.l bus1=sourcebus.1 bus2=b1.1 geometry=g length=1 units=m
"#;
    let source = powerio_core::Source::text(text.into(), Some("swer.dss".into()));
    let module = powerio_dist::dss::parse_raw_from_source(&source).unwrap();
    let (net, diagnostics) = to_network_from_raw(&module);

    assert!(net.lines().is_empty());
    assert!(net.line_codes().iter().all(|c| c.n_conductors != 3 || !c.name.starts_with("_line_l")));
    assert!(diagnostics.iter().any(|d| d.code.as_str() == "READ.DSS.GEOMETRY_UNRESOLVED"));
}
