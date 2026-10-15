"""Regenerate the committed LinDist3Flow voltage oracle with OpenDSS."""

from __future__ import annotations

import argparse
import json
import math
import os
import sys
from pathlib import Path

from opendssdirect import dss


ROOT = Path(__file__).resolve().parents[2]
ORACLE = ROOT / "tests/data/dist/micro/lindist3flow_oracle.json"
ABSOLUTE_TOLERANCE = 1e-6


def dss_path(path: Path) -> str:
    return '"' + str(path.resolve()).replace('"', '""') + '"'


def generated_values(case: Path) -> dict[str, float]:
    dss.Basic.ClearAll()
    dss(f"Redirect {dss_path(case)}")
    dss("Solve")
    if not dss.Solution.Converged():
        raise RuntimeError("OpenDSS solve did not converge")

    names = dss.Circuit.AllNodeNames()
    components = dss.Circuit.AllBusVolts()
    voltages = {
        name: (components[2 * index], components[2 * index + 1])
        for index, name in enumerate(names)
    }
    source_re, source_im = voltages["source.1"]
    load_re, load_im = voltages["loadbus.1"]

    if dss.Circuit.SetActiveElement("Line.l1") <= 0:
        raise RuntimeError("OpenDSS did not expose Line.l1")
    line_powers = dss.CktElement.Powers()
    if dss.Circuit.SetActiveElement("Load.ld1") <= 0:
        raise RuntimeError("OpenDSS did not expose Load.ld1")
    load_powers = dss.CktElement.Powers()

    return {
        "source_voltage_magnitude_v": math.hypot(source_re, source_im),
        "load_voltage_re_v": load_re,
        "load_voltage_im_v": load_im,
        "load_voltage_magnitude_v": math.hypot(load_re, load_im),
        "line_sending_active_power_w": 1000.0 * line_powers[0],
        "line_sending_reactive_power_var": 1000.0 * line_powers[1],
        "load_active_power_w": 1000.0 * load_powers[0],
        "load_reactive_power_var": 1000.0 * load_powers[1],
    }


def append_result(mark: str) -> None:
    output = os.environ.get("PIO_RESULTS_TSV")
    if output:
        with open(output, "a", encoding="utf-8") as stream:
            stream.write(f"lindist3flow_oracle.dss\topendss-lindist3flow\t{mark}\n")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--count", action="store_true")
    args = parser.parse_args()
    if args.count:
        print(1)
        return 0

    expected = json.loads(ORACLE.read_text(encoding="utf-8"))
    case = ORACLE.with_name(expected["case"])
    actual = generated_values(case)
    failures = []
    for name, value in actual.items():
        difference = abs(value - expected[name])
        if difference > ABSOLUTE_TOLERANCE:
            failures.append(
                f"{name}: expected {expected[name]:.15g}, got {value:.15g}, "
                f"difference {difference:.6g}"
            )

    mark = "ok" if not failures else "FAIL"
    append_result(mark)
    print(f"{case.relative_to(ROOT)}: {mark}")
    for failure in failures:
        print(f"  {failure}")
    return int(bool(failures))


if __name__ == "__main__":
    sys.exit(main())
