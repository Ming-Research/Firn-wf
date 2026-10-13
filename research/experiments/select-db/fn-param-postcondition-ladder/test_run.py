"""CI-only instrument checks; fake subprocess results never run Whitefoot."""

from contextlib import redirect_stderr, redirect_stdout
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import run as ladder


DIAGNOSTIC = '''case.wf:9:3: error[FN-9]: UndischargedPostcondition
  source:   return apply(window: window);
  concrete_function: forward::<fn preserve>
  postcondition: case.wf:6:3 "ensures window^.len == entry(window)^.len;"
'''


class InstrumentChecks(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.cases = []
        for identifier in ("00-full", "01-reduced", *sorted(ladder.CONTROLS)):
            (self.root / f"{identifier}.wf").write_text("witness input")
            self.cases.append({"id": identifier, "kind": "bundle", "path": f"{identifier}.wf",
                               "delta": "fixture", "chain": identifier not in ladder.CONTROLS})
        self.path = self.root / "cases.json"
        self.write_cases()
        self.compilers = []
        for digit in ("a", "b"):
            commit = digit * 40
            tag = f"wf-{commit[:12]}"
            directory = self.root / tag
            directory.mkdir()
            (directory / "whitefoot-release.json").write_text(json.dumps({"tag": tag, "commit": commit}))
            (directory / "whitefootc").write_text("not executed: subprocess.run is mocked")
            (directory / "whitefootc").chmod(0o700)
            self.compilers.append((tag, commit, str(directory)))

    def write_cases(self):
        self.path.write_text(json.dumps(self.cases))

    def arguments(self):
        arguments = ["--cases", str(self.path), "--output", str(self.root / "output"), "--chain-steps", "2"]
        for compiler in self.compilers:
            arguments.extend(["--compiler", *compiler])
        return arguments

    def execute(self, mock_run):
        with patch.object(ladder.subprocess, "run", mock_run), patch.dict(ladder.os.environ, {}, clear=True):
            with redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
                return ladder.main(self.arguments())

    def test_rejection_does_not_skip_remaining_cases_or_compilers(self):
        calls = []
        def fake_run(argv, **keywords):
            calls.append(argv)
            self.assertEqual(keywords["cwd"], self.root)
            self.assertFalse(keywords["check"])
            self.assertEqual(argv[1], "--check")
            self.assertEqual(len(argv), 3)
            self.assertTrue(Path(argv[2]).is_file())
            return subprocess.CompletedProcess(argv, 1, DIAGNOSTIC.encode())
        self.assertEqual(self.execute(fake_run), 0)
        self.assertEqual(len(calls), len(self.cases) * 2)
        rows = json.loads((self.root / "output/rows.json").read_text())
        self.assertTrue(all(row["exit_code"] == 1 and row["adapter_fn9"] for row in rows))
        self.assertTrue(all((self.root / "output" / row["diagnostic"]).is_file() for row in rows))

    def test_timeout_is_separate_from_adapter_fn9(self):
        def fake_run(argv, **keywords):
            raise subprocess.TimeoutExpired(argv, 60, output=DIAGNOSTIC.encode())
        self.assertEqual(self.execute(fake_run), 0)
        rows = json.loads((self.root / "output/rows.json").read_text())
        self.assertTrue(all(row["exit_code"] == 124 and row["timed_out"] and not row["adapter_fn9"] for row in rows))

    def test_only_same_block_adapter_length_failure_matches(self):
        self.assertTrue(ladder.matched_adapter_fn9(DIAGNOSTIC))
        self.assertTrue(ladder.matched_adapter_fn9(DIAGNOSTIC.replace(
            "forward::<fn preserve>", "pkg::store::command_held_entries_run_due::<u64, fn preserve>")))
        for altered in (DIAGNOSTIC.replace("forward::<fn preserve>", "preserve"),
                        DIAGNOSTIC.replace("forward::<fn preserve>", "preserve::<fn forward>"),
                        DIAGNOSTIC.replace("return apply", "return concrete"),
                        DIAGNOSTIC.replace("entry(window)", "entry(other)"),
                        DIAGNOSTIC.replace("UndischargedPostcondition", "SomethingElse"),
                        '  doc "' + DIAGNOSTIC.replace("\n", " ") + '";',
                        DIAGNOSTIC.split("  concrete_function:")[0] +
                        "other.wf:1:1: error[FN-8]: Other\n" +
                        "  concrete_function:" + DIAGNOSTIC.split("  concrete_function:")[1]):
            with self.subTest(diagnostic=altered):
                self.assertFalse(ladder.matched_adapter_fn9(altered))

    def test_missing_or_corrupt_metadata_and_inputs_are_rejected(self):
        original = list(self.cases)
        variants = [original[1:], original[:1] + original[2:], original[:-1], original + [original[0]],
                    [{**original[0], "path": "../outside.wf"}, *original[1:]],
                    [{**original[0], "path": "missing.wf"}, *original[1:]],
                    [{**original[0], "chain": "true"}, *original[1:]],
                    [{**original[0], "kind": "other"}, *original[1:]],
                    [{key: value for key, value in original[0].items() if key != "delta"}, *original[1:]]]
        for cases in variants:
            with self.subTest(cases=cases):
                self.path.write_text(json.dumps(cases))
                with self.assertRaises(ValueError):
                    ladder.load_cases(self.path, chain_steps=2)
        self.path.write_text("{")
        with self.assertRaises(ValueError):
            ladder.load_cases(self.path, chain_steps=2)
        self.path.unlink()
        with self.assertRaises(OSError):
            ladder.load_cases(self.path, chain_steps=2)

    def test_graph_argv_and_missing_graph_body(self):
        directory = self.root / "graph"
        directory.mkdir()
        graph = directory / "modules.wfg"
        graph.write_text("pkg::store: [];\nentry witness = pkg::store::main;\n")
        self.cases[0].update(kind="graph", path="graph/modules.wfg")
        self.write_cases()
        with self.assertRaises(ValueError):
            ladder.load_cases(self.path, chain_steps=2)
        module = directory / "store"
        module.mkdir()
        (module / "module.wfm").write_text("interface")
        (module / "body.wf").write_text("body")
        cases, root = ladder.load_cases(self.path, chain_steps=2)
        compiler = ladder.load_compiler(*self.compilers[0])
        self.assertEqual(ladder.command_for(compiler, cases[0], root),
                         [compiler["binary"], "--graph", str(graph), "--check-modules"])

    def test_wrong_or_missing_provenance_is_rejected(self):
        tag, commit, directory = self.compilers[0]
        manifest_path = Path(directory) / "whitefoot-release.json"
        for manifest in ({"tag": "wrong", "commit": commit}, {"tag": tag, "commit": commit[:12]},
                         {"tag": tag, "commit": "c" * 40}, {}, []):
            manifest_path.write_text(json.dumps(manifest))
            with self.subTest(manifest=manifest), self.assertRaises(ValueError):
                ladder.load_compiler(tag, commit, directory)
        manifest_path.unlink()
        with self.assertRaises(OSError):
            ladder.load_compiler(tag, commit, directory)
        manifest_path.write_text(json.dumps({"tag": tag, "commit": commit}))
        (Path(directory) / "whitefootc").unlink()
        with self.assertRaises(ValueError):
            ladder.load_compiler(tag, commit, directory)

    def test_wrong_argv_or_missing_input_diagnostic_fails_recording(self):
        for diagnostic in (b"error: unexpected argument '--invalid'\nUsage: whitefootc\n",
                           b"No such file or directory"):
            calls = []
            def fake_run(argv, **keywords):
                calls.append(argv)
                return subprocess.CompletedProcess(argv, 2, diagnostic)
            with self.subTest(diagnostic=diagnostic):
                self.assertEqual(self.execute(fake_run), 2)
                self.assertEqual(len(calls), len(self.cases) * 2)

    def test_summary_preserves_actual_order_and_nonmonotonic_results(self):
        compiler = {"tag": "one"}
        rows = [{"compiler": "one", "step": f"{index:02d}", "chain": True,
                 "exit_code": code, "adapter_fn9": fn9}
                for index, code, fn9 in ((0, 1, True), (1, 0, False), (2, 1, True), (3, 2, False))]
        rows.insert(0, {"compiler": "one", "step": "accepted-baseline", "chain": False,
                        "exit_code": 0, "adapter_fn9": False})
        rows.append({"compiler": "one", "step": "ordinal-before", "chain": False,
                     "exit_code": 1, "adapter_fn9": True})
        summary = ladder.summarize(rows, [compiler])[0]
        self.assertEqual(summary["first_passing_chain_step"], "01")
        self.assertEqual(summary["last_failing_adapter_fn9_chain_step"], "02")
        self.assertEqual(summary["failures_outside_adapter_fn9"], ["03"])
        self.assertEqual(summary["failing_adapter_fn9_controls"], ["ordinal-before"])
        for row in rows:
            row["first_diagnostic_line"] = "fixture"
        self.assertIn("controls failing with that diagnostic: ordinal-before", ladder.markdown(rows, [summary]))
