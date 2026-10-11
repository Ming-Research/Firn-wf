#!/usr/bin/env python3
"""Branch-only counts experiment for halo-req-sweep-count; run only in CI.

Reuse firn-workload's unchanged rate-limiter-flexible script and request mode.
Python is needed because redis-benchmark cannot read custom per-collection INFO
rows or validate completeness and summarize their distributions. The temporary
workflow owns this driver; remove both when this experiment branch is retired.
"""
import argparse
import csv
import io
import math
import os
from pathlib import Path
import re
import socket
import statistics
import subprocess
import sys
import tempfile
import time
import unittest

HALO_REVISION = "15709ec77a382ba26b7d6f43ef88256ba2f7b109"
KINDS = ("strings", "tables", "closures", "upvalues")
COUNTERS = ("slots_visited", "freed_objects", "live_objects")
METADATA = ("halo_sweep_collections", "halo_sweep_samples",
            "halo_sweep_missed_collections", "halo_sweep_dropped_samples")
ROW_FIELDS = ("collection", "generation") + COUNTERS + tuple(
    f"{kind}_{counter}" for kind in KINDS for counter in COUNTERS)


def number(value):
    if not re.fullmatch(r"[0-9]+", value):
        raise ValueError(f"expected nonnegative integer, got {value!r}")
    return int(value)


def parse_info(text):
    fields = {}
    for line in text.splitlines():
        if not line or line.startswith("#"):
            continue
        if ":" not in line:
            raise ValueError(f"not an INFO field: {line!r}")
        key, value = line.split(":", 1)
        if key in fields:
            raise ValueError(f"duplicate INFO field: {key}")
        fields[key] = value
    for key in METADATA:
        if key not in fields:
            raise ValueError(f"missing INFO field: {key}")
    metadata = {key: number(fields[key]) for key in METADATA}
    count = metadata["halo_sweep_collections"]
    if not count or count != metadata["halo_sweep_samples"]:
        raise ValueError("no collections or collection/sample count mismatch")
    if metadata["halo_sweep_missed_collections"] or metadata["halo_sweep_dropped_samples"]:
        raise ValueError("incomplete collection history: missed/dropped samples")
    # Require exact row identities, including no undocumented extra rows.
    row_keys = {key for key in fields if key.startswith("halo_sweep_sample_")}
    expected = {f"halo_sweep_sample_{i}" for i in range(1, count + 1)}
    if row_keys != expected:
        raise ValueError("missing or extra collection row")
    rows = []
    for i in range(1, count + 1):
        row = {}
        for pair in fields[f"halo_sweep_sample_{i}"].split(","):
            if "=" not in pair:
                raise ValueError("invalid collection row field")
            key, value = pair.split("=", 1)
            if key in row:
                raise ValueError(f"duplicate collection counter: {key}")
            row[key] = number(value)
        if set(row) != set(ROW_FIELDS):
            raise ValueError("missing or extra collection counter")
        if row["collection"] != i or row["generation"] != 0:
            raise ValueError("unexpected collection order or engine generation")
        for counter in COUNTERS:
            if row[counter] != sum(row[f"{kind}_{counter}"] for kind in KINDS):
                raise ValueError(f"per-kind {counter} does not sum to total")
        if row["live_objects"] + row["freed_objects"] == 0:
            raise ValueError("ratio denominator is zero")
        rows.append(row)
    return metadata, rows


def ratio(row):
    return row["slots_visited"] / (row["live_objects"] + row["freed_objects"])


def accepted(row):
    # Integer comparison, avoiding rounding at the 1.5 acceptance boundary.
    return 2 * row["slots_visited"] <= 3 * (row["live_objects"] + row["freed_objects"])


def summarize(info, out):
    _, rows = parse_info(info)
    with (out / "collections.csv").open("w", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=ROW_FIELDS + ("ratio", "accepted"))
        writer.writeheader()
        for row in rows:
            writer.writerow(dict(row, ratio=f"{ratio(row):.9f}", accepted=accepted(row)))
    summary = io.StringIO()
    summary.write(f"collections,{len(rows)}\n")
    summary.write("kind,metric,samples,undefined,mean,median,p90,max,sum\n")
    for kind in ("total",) + KINDS:
        for counter in COUNTERS:
            key = counter if kind == "total" else f"{kind}_{counter}"
            values = [row[key] for row in rows]
            ordered = sorted(values)
            summary.write(f"{kind},{counter},{len(values)},0,{statistics.mean(values):.6f},"
                          f"{statistics.median(values):.6f},"
                          f"{ordered[math.ceil(.9 * len(values)) - 1]},{max(values)},{sum(values)}\n")
    for kind in ("total",) + KINDS:
        prefix = "" if kind == "total" else f"{kind}_"
        denominators = [row[f"{prefix}live_objects"] + row[f"{prefix}freed_objects"] for row in rows]
        values = [row[f"{prefix}slots_visited"] / denominator for row, denominator in zip(rows, denominators) if denominator]
        undefined = len(rows) - len(values)
        if values:
            ordered = sorted(values)
            summary.write(f"{kind},ratio,{len(values)},{undefined},{statistics.mean(values):.9f},{statistics.median(values):.9f},"
                          f"{ordered[math.ceil(.9 * len(values)) - 1]:.9f},{max(values):.9f},n/a\n")
        else:
            summary.write(f"{kind},ratio,0,{undefined},n/a,n/a,n/a,n/a,n/a\n")
    violations = [row["collection"] for row in rows if not accepted(row)]
    summary.write("acceptance," + ("FAIL" if violations else "PASS") +
                  f",violations={len(violations)},rule=2*slots_visited<=3*(live_objects+freed_objects)\n")
    if violations:
        summary.write("violating_collections," + ",".join(map(str, violations)) + "\n")
    summary.write("percentiles,median=arithmetic midpoint,p90=nearest rank\n")
    result = summary.getvalue()
    (out / "summary.csv").write_text(result)
    return result


def redis_cli(port, *args):
    return subprocess.run(["redis-cli", "--raw", "-p", str(port), *args],
                          check=True, capture_output=True, text=True, timeout=30).stdout


def validate_default_sources(module, constructor, collector, sources):
    if not re.search(r"public const collection_floor: u64 = 65536_u64;", module):
        raise ValueError("pinned Halo collection floor is not 64 KiB")
    if "threshold: collection_floor, gc_stress: stress, collections: 0_u64, gc_pause: 200_u64" not in constructor:
        raise ValueError("pinned Halo constructor does not have default threshold/pause")
    if "return imax(total, collection_floor);" not in collector:
        raise ValueError("pinned Halo collector no longer applies the default floor")
    # This experiment must not tune the collector or request extra collections.
    override = re.compile(r"\b(?:set_gc_floor|set_gc_pause|set_gc_stress|collect_at_next_safepoint)\s*\(")
    for path, source in sources:
        if override.search(source):
            raise ValueError(f"firn changes default GC settings or forces a collection: {path}")


def validate_defaults():
    heap = Path("deps/halo-wf/lib/halo/heap")
    validate_default_sources((heap / "module.wfm").read_text(),
                             (heap / "slabs.wf").read_text(),
                             (heap / "gc.wf").read_text(),
                             ((source, source.read_text()) for source in Path("firn").rglob("*.wf")))


def run(opts):
    if os.environ.get("GITHUB_ACTIONS") != "true":
        raise ValueError("the collection workload runs only in GitHub Actions")
    revision = subprocess.run(["git", "-C", "deps/halo-wf", "rev-parse", "HEAD"],
                              check=True, capture_output=True, text=True).stdout.strip()
    if revision != HALO_REVISION:
        raise ValueError(f"unexpected Halo revision: {revision}")
    validate_defaults()
    opts.out.mkdir(parents=True, exist_ok=True)
    # Let the OS choose an unused port. The hosted job starts no other server.
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    environment = dict(os.environ, WF_DRIVERS="2")
    with (opts.out / "server.log").open("w") as log:
        server = subprocess.Popen([str(opts.firn.resolve()), str(port), "0", "-", "0"],
                                  env=environment, stdout=log, stderr=subprocess.STDOUT)
        try:
            for _ in range(400):
                if server.poll() is not None:
                    raise ValueError("firn exited before readiness; inspect server.log")
                try:
                    if redis_cli(port, "PING").strip() == "PONG":
                        break
                except (subprocess.CalledProcessError, subprocess.TimeoutExpired):
                    pass
                time.sleep(.05)
            else:
                raise ValueError("firn did not answer PING")
            # Keep the ordinary client and its error checks; discard its timing CSV.
            subprocess.run([str(opts.client.resolve()), "--port", str(port),
                            "--workload", "limiter-script", "--connections", "50",
                            "--threads", "2", "--requests", str(opts.requests)],
                           check=True, stdout=subprocess.DEVNULL, timeout=300)
            info = redis_cli(port, "INFO", "halo_sweep")
            (opts.out / "halo-sweep.info").write_text(info)
            firn_revision = subprocess.run(["git", "rev-parse", "HEAD"], check=True,
                                           capture_output=True, text=True).stdout.strip()
            metadata = (f"firn_revision,{firn_revision}\nhalo_revision,{revision}\nworkload,rate-limiter-flexible-v11.2.1-script\n"
                        f"connections,50\nrequests,{opts.requests}\nserver_drivers,2\n"
                        "gc_settings,Halo DEFAULT floor=65536 initial_threshold=65536 pause=200\n"
                        "gc_settings_basis,pinned source constructor and floor asserted; no firn GC override calls\n"
                        "scope,fresh-server setup plus completed workload; no forced collection\n")
            (opts.out / "metadata.csv").write_text(metadata)
            print(metadata + summarize(info, opts.out), end="")
        finally:
            if server.poll() is None:
                server.terminate()
                try:
                    server.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait(timeout=10)


def fixture():
    row = {key: 0 for key in ROW_FIELDS}
    row.update(collection=1, slots_visited=15, live_objects=6, freed_objects=4,
               strings_slots_visited=10, strings_live_objects=4, strings_freed_objects=3,
               tables_slots_visited=5, tables_live_objects=2, tables_freed_objects=1)
    metadata = dict(zip(METADATA, (1, 1, 0, 0)))
    return "\n".join(f"{key}:{value}" for key, value in metadata.items()) + "\n" + \
        "halo_sweep_sample_1:" + ",".join(f"{key}={value}" for key, value in row.items()) + "\n"


class ParserTests(unittest.TestCase):
    def test_default_source_guard_detects_settings_changes(self):
        module = "public const collection_floor: u64 = 65536_u64;"
        constructor = "threshold: collection_floor, gc_stress: stress, collections: 0_u64, gc_pause: 200_u64"
        collector = "return imax(total, collection_floor);"
        validate_default_sources(module, constructor, collector, [])
        cases = (
            (module.replace("65536", "1048576"), constructor, collector, []),
            (module, constructor.replace("collection_floor", "1048576_u64"), collector, []),
            (module, constructor.replace("200_u64", "400_u64"), collector, []),
            (module, constructor, collector.replace("collection_floor", "0_u64"), []),
            (module, constructor, collector, [("fixture.wf", "set_gc_pause(engine: e, percent: 200_u64);")]),
            (module, constructor, collector, [("fixture.wf", "collect_at_next_safepoint(engine: e);")]),
        )
        for i, args in enumerate(cases):
            with self.subTest(case=i), self.assertRaises(ValueError):
                validate_default_sources(*args)

    def test_boundary_and_violation(self):
        _, rows = parse_info(fixture())
        self.assertTrue(accepted(rows[0]))
        wrong = fixture().replace("slots_visited=15", "slots_visited=16").replace(
            "strings_slots_visited=10", "strings_slots_visited=11")
        _, rows = parse_info(wrong)
        self.assertFalse(accepted(rows[0]))
        self.assertEqual(ratio(rows[0]), 1.6)

    def test_summary_percentiles_and_zero_kind_denominators(self):
        base = fixture()
        headers, sample = base.split("halo_sweep_sample_1:", 1)
        headers = headers.replace("halo_sweep_collections:1", "halo_sweep_collections:4").replace(
            "halo_sweep_samples:1", "halo_sweep_samples:4")
        info = headers
        for i, slots in enumerate((10, 11, 12, 15), 1):
            row = sample.replace("collection=1", f"collection={i}").replace(
                "slots_visited=15", f"slots_visited={slots}").replace(
                "strings_slots_visited=10", f"strings_slots_visited={slots - 5}")
            info += f"halo_sweep_sample_{i}:" + row
        with tempfile.TemporaryDirectory() as directory:
            output = summarize(info, Path(directory))
            self.assertIn("total,slots_visited,4,0,12.000000,11.500000,15,15,48\n", output)
            self.assertIn("total,ratio,4,0,1.200000000,1.150000000,1.500000000,1.500000000,n/a\n", output)
            self.assertIn("closures,ratio,0,4,n/a,n/a,n/a,n/a,n/a\n", output)
            self.assertIn("upvalues,ratio,0,4,n/a,n/a,n/a,n/a,n/a\n", output)
            records = list(csv.DictReader((Path(directory) / "collections.csv").read_text().splitlines()))
            self.assertEqual(len(records), 4)
            self.assertEqual(records[-1]["ratio"], "1.500000000")
            self.assertIn("acceptance,PASS,violations=0", output)

    def test_reject_incomplete_and_wrong_inputs(self):
        base = fixture()
        cases = {
            "missing metadata": base.replace("halo_sweep_samples:1\n", ""),
            "missing row": base.split("halo_sweep_sample_1:")[0],
            "missing counter": base.replace(",tables_live_objects=2", ""),
            "wrong kind sum": base.replace("strings_live_objects=4", "strings_live_objects=5"),
            "missed snapshot": base.replace("halo_sweep_missed_collections:0", "halo_sweep_missed_collections:1"),
            "dropped snapshot": base.replace("halo_sweep_dropped_samples:0", "halo_sweep_dropped_samples:1"),
            "mismatch count": base.replace("halo_sweep_collections:1", "halo_sweep_collections:2"),
            "no collections": base.replace("halo_sweep_collections:1", "halo_sweep_collections:0"),
            "wrong sequence": base.replace("collection=1", "collection=2"),
            "unexpected reset": base.replace("generation=0", "generation=1"),
            "duplicate metadata": base + "halo_sweep_samples:1\n",
            "duplicate row counter": base.replace("collection=1", "collection=1,collection=1"),
            "negative counter": base.replace("freed_objects=4", "freed_objects=-4"),
            "zero denominator": re.sub(r"((?:live|freed)_objects=)[0-9]+", r"\g<1>0", base),
        }
        for name, text in cases.items():
            with self.subTest(name=name), self.assertRaises(ValueError):
                parse_info(text)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    parser.add_argument("--firn", type=Path)
    parser.add_argument("--client", type=Path)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--requests", type=int, default=10000)
    opts = parser.parse_args()
    if opts.self_test:
        result = unittest.TextTestRunner(verbosity=2).run(unittest.defaultTestLoader.loadTestsFromTestCase(ParserTests))
        return 0 if result.wasSuccessful() else 1
    if opts.firn is None or opts.client is None or opts.out is None or opts.requests < 50:
        parser.error("run needs --firn, --client, --out and --requests >= 50")
    run(opts)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"halo-sweep-count: {error}", file=sys.stderr)
        sys.exit(1)
