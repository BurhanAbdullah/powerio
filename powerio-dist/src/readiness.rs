//! Electrical readiness checks for multiconductor distribution models.
//!
//! These checks sit deliberately below parsers and above numerical consumers:
//! they answer whether a [`MulticonductorNetwork`] is structurally safe to hand
//! to a solver, transformer, or writer. In particular, a missing linecode is
//! an electrical blocker, not a request to synthesize impedance data.

use std::collections::HashMap;

use crate::{Mat, MulticonductorNetwork};

/// Severity of an electrical-readiness finding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadinessSeverity {
    Warning,
    Error,
}

/// One structured finding produced by [`check_electrical_readiness`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadinessFinding {
    pub severity: ReadinessSeverity,
    pub code: &'static str,
    pub path: String,
    pub message: String,
}

/// Aggregate result of an electrical-readiness audit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ElectricalReadiness {
    pub findings: Vec<ReadinessFinding>,
}

impl ElectricalReadiness {
    /// True only when no electrical blocker was found.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.findings
            .iter()
            .all(|finding| finding.severity != ReadinessSeverity::Error)
    }

    /// Number of blocking findings.
    #[must_use]
    pub fn error_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == ReadinessSeverity::Error)
            .count()
    }

    fn error(&mut self, code: &'static str, path: impl Into<String>, message: impl Into<String>) {
        self.findings.push(ReadinessFinding {
            severity: ReadinessSeverity::Error,
            code,
            path: path.into(),
            message: message.into(),
        });
    }
}

/// Audit a distribution model before an operation that requires electrically
/// meaningful element parameters.
///
/// The audit is intentionally fail-closed. A reference to a linecode that is
/// absent from the model remains unresolved; no default impedance, conductor
/// count, or other electrical value is fabricated here.
#[must_use]
pub fn check_electrical_readiness(net: &MulticonductorNetwork) -> ElectricalReadiness {
    let mut result = ElectricalReadiness::default();

    if !net.base_frequency().is_finite() || net.base_frequency() <= 0.0 {
        result.error(
            "MODEL.NONFINITE_FREQUENCY",
            "/base_frequency",
            "base frequency must be finite and greater than zero",
        );
    }

    let buses = unique_ids(net.buses().iter().map(|bus| bus.id.as_str()));
    for duplicate in buses {
        result.error(
            "MODEL.DUPLICATE_BUS_ID",
            "/buses",
            format!("duplicate bus id '{duplicate}'"),
        );
    }

    let linecodes = unique_ids(net.line_codes().iter().map(|code| code.name.as_str()));
    for duplicate in linecodes {
        result.error(
            "MODEL.DUPLICATE_LINECODE_ID",
            "/linecodes",
            format!("duplicate linecode name '{duplicate}'"),
        );
    }

    let bus_terminals: HashMap<String, Vec<String>> = net
        .buses()
        .iter()
        .map(|bus| (fold(&bus.id), bus.terminals.clone()))
        .collect();
    let linecode_map: HashMap<String, &crate::DistLineCode> = net
        .line_codes()
        .iter()
        .map(|code| (fold(&code.name), code))
        .collect();

    for (index, line) in net.lines().iter().enumerate() {
        let path = format!("/lines/{index}");
        if !line.length.is_finite() || line.length <= 0.0 {
            result.error(
                "LINE.INVALID_LENGTH",
                format!("{path}/length"),
                "line length must be finite and greater than zero",
            );
        }

        let Some(code) = linecode_map.get(&fold(&line.linecode)) else {
            result.error(
                "LINE.UNRESOLVED_LINECODE",
                format!("{path}/linecode"),
                format!(
                    "linecode '{}' is not defined; electrical defaults are not synthesized",
                    line.linecode
                ),
            );
            continue;
        };

        let code_path = format!("{path}/linecode/{}", code.name);
        check_matrix(
            &mut result,
            &code_path,
            code.n_conductors,
            &code.r_series,
            "r_series",
        );
        check_matrix(
            &mut result,
            &code_path,
            code.n_conductors,
            &code.x_series,
            "x_series",
        );

        check_terminal_map(
            &mut result,
            &path,
            "bus_from",
            &line.bus_from,
            &line.terminal_map_from,
            &bus_terminals,
        );
        check_terminal_map(
            &mut result,
            &path,
            "bus_to",
            &line.bus_to,
            &line.terminal_map_to,
            &bus_terminals,
        );

        if line.terminal_map_from.len() != line.terminal_map_to.len() {
            result.error(
                "LINE.TERMINAL_MAP_ARITY",
                path.clone(),
                "from/to terminal maps must contain the same number of conductors",
            );
        }
        if line.terminal_map_from.len() != code.n_conductors {
            result.error(
                "LINE.CONDUCTOR_COUNT_MISMATCH",
                path,
                format!(
                    "terminal maps contain {} conductors but linecode '{}' declares {}",
                    line.terminal_map_from.len(),
                    code.name,
                    code.n_conductors
                ),
            );
        }
    }

    result
}

fn check_terminal_map(
    result: &mut ElectricalReadiness,
    path: &str,
    bus_field: &str,
    bus_id: &str,
    terminals: &[String],
    buses: &HashMap<String, Vec<String>>,
) {
    let Some(bus_terminals) = buses.get(&fold(bus_id)) else {
        result.error(
            "LINE.UNKNOWN_BUS",
            format!("{path}/{bus_field}"),
            format!("bus '{bus_id}' is not defined"),
        );
        return;
    };
    for terminal in terminals {
        if !bus_terminals.iter().any(|candidate| candidate == terminal) {
            result.error(
                "LINE.UNKNOWN_TERMINAL",
                format!("{path}/{bus_field}"),
                format!("terminal '{terminal}' is not present on bus '{bus_id}'"),
            );
        }
    }
}

fn check_matrix(
    result: &mut ElectricalReadiness,
    path: &str,
    expected: usize,
    matrix: &Mat,
    field: &str,
) {
    if matrix.len() != expected || matrix.iter().any(|row| row.len() != expected) {
        result.error(
            "LINECODE.MATRIX_SHAPE",
            format!("{path}/{field}"),
            format!("matrix must be {expected}x{expected}"),
        );
    }
    if matrix.iter().flatten().any(|value| !value.is_finite()) {
        result.error(
            "LINECODE.NONFINITE_MATRIX",
            format!("{path}/{field}"),
            "impedance matrix contains a non-finite value",
        );
    }
}

fn unique_ids<'a, I>(ids: I) -> Vec<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut seen = HashMap::<String, String>::new();
    let mut duplicates = Vec::new();
    for id in ids {
        let folded = fold(id);
        if seen.insert(folded, id.to_owned()).is_some() {
            duplicates.push(id.to_owned());
        }
    }
    duplicates
}

fn fold(value: &str) -> String {
    value.to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DistBus, DistLine, DistLineCode};

    fn base_network() -> MulticonductorNetwork {
        let mut net = MulticonductorNetwork::default();
        net.buses_mut().push(DistBus::new(
            "source",
            vec!["1".into(), "2".into(), "3".into()],
        ));
        net.buses_mut().push(DistBus::new(
            "load",
            vec!["1".into(), "2".into(), "3".into()],
        ));
        net.line_codes_mut().push(DistLineCode::new(
            "lc",
            vec![
                vec![0.1, 0.01, 0.01],
                vec![0.01, 0.1, 0.01],
                vec![0.01, 0.01, 0.1],
            ],
            vec![
                vec![0.2, 0.02, 0.02],
                vec![0.02, 0.2, 0.02],
                vec![0.02, 0.02, 0.2],
            ],
        ));
        net.lines_mut().push(DistLine::new(
            "l1",
            "source",
            "load",
            vec!["1".into(), "2".into(), "3".into()],
            vec!["1".into(), "2".into(), "3".into()],
            "lc",
            100.0,
        ));
        net
    }

    #[test]
    fn valid_three_phase_line_is_ready() {
        assert!(check_electrical_readiness(&base_network()).is_ready());
    }

    #[test]
    fn missing_linecode_is_a_blocker() {
        let mut net = base_network();
        net.lines_mut()[0].linecode = "missing".into();
        let report = check_electrical_readiness(&net);
        assert!(!report.is_ready());
        assert!(report
            .findings
            .iter()
            .any(|f| f.code == "LINE.UNRESOLVED_LINECODE"));
    }

    #[test]
    fn terminal_not_on_bus_is_a_blocker() {
        let mut net = base_network();
        net.lines_mut()[0].terminal_map_to[0] = "99".into();
        let report = check_electrical_readiness(&net);
        assert!(!report.is_ready());
        assert!(report
            .findings
            .iter()
            .any(|f| f.code == "LINE.UNKNOWN_TERMINAL"));
    }

    #[test]
    fn malformed_impedance_matrix_is_a_blocker() {
        let mut net = base_network();
        net.line_codes_mut()[0].x_series[0].pop();
        let report = check_electrical_readiness(&net);
        assert!(!report.is_ready());
        assert!(report
            .findings
            .iter()
            .any(|f| f.code == "LINECODE.MATRIX_SHAPE"));
    }
}
