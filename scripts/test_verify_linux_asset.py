"""Acceptance-boundary tests for the paired parity bound.

These exercise the exact failure/success semantics publication relies on:
faster, slower, and inconclusive (too-uncertain) candidate samples on the
same shape the benchmark produces, plus the hard-fail half of the smoke
gate. They are not CLI forwarding tests: no subprocesses, no media, no
source-text assertions (the command layer is stubbed).
"""

import contextlib
import importlib.util
import json
import sys
import tempfile
from pathlib import Path
import random
import threading
import unittest
from unittest.mock import Mock, patch


SPEC = importlib.util.spec_from_file_location("verify_linux_asset",
                                              Path(__file__).with_name(
                                                  "verify_linux_asset.py"))
verifier = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(verifier)

SEED = 20260117
PAIRS = verifier.PAIRS


class ParityUpperBoundTests(unittest.TestCase):
    def samples(self, offset, scale=0.02, baseline_mean=100.0):
        """Deterministic baseline/candidate sample lists."""
        rng = random.Random(SEED)
        baseline = [baseline_mean * (1 + offset + scale * rng.uniform(-1, 1))
                    for _ in range(PAIRS)]
        candidate = [bv + offset * baseline_mean *
                     (1 + scale * rng.uniform(-1, 1))
                     for bv in baseline]
        return baseline, candidate

    def test_identical_samples_measure_zero(self):
        rng = random.Random(SEED)
        baseline = [100.0 + 5.0 * rng.uniform(-1, 1) for _ in range(PAIRS)]
        self.assertEqual(verifier.parity_upper_bound(baseline, baseline), 0.0)

    def test_consistently_faster_candidate_passes(self):
        baseline, candidate = self.samples(offset=-0.20)
        self.assertLess(verifier.parity_upper_bound(baseline, candidate), 0.0)

    def test_consistently_slower_candidate_fails(self):
        baseline, candidate = self.samples(offset=+0.20)
        self.assertGreater(verifier.parity_upper_bound(baseline, candidate), 0.0)

    def test_faster_mean_with_fast_outlier_fails(self):
        # Point estimate is faster, but one fast outlier (30.0 perf_counter
        # delta against 98/100) inflates the spread so the one-sided 95%
        # bound crosses zero: inconclusive means failure.
        baseline = [100.0 for _ in range(PAIRS)]
        candidate = [98.0 for _ in range(PAIRS - 1)] + [30.0]
        bound = verifier.parity_upper_bound(baseline, candidate)
        mean_ratio = sum(candidate) / sum(baseline)
        self.assertLess(mean_ratio, 1.0)
        self.assertGreater(bound, 0.0)

    def test_too_few_pairs_fails_closed(self):
        with self.assertRaises(ValueError):
            verifier.parity_upper_bound([1.0] * (PAIRS - 1), [1.0] * (PAIRS - 1))

    def test_unequal_sample_lengths_fails_closed(self):
        with self.assertRaises(ValueError):
            verifier.parity_upper_bound([1.0] * PAIRS, [1.0] * (PAIRS + 1))

    def test_nonpositive_timing_fails_closed(self):
        baseline = [1.0] * PAIRS
        candidate = [1.0] * (PAIRS - 1) + [0.0]
        with self.assertRaises(ValueError):
            verifier.parity_upper_bound(baseline, candidate)


class CheckVersionTests(unittest.TestCase):
    def check(self, reported, version="2.0.0"):
        with patch.object(verifier, "run_checked") as runner:
            runner.return_value = Mock(stdout=reported.encode())
            verifier.check_version(Path("m4b-merge"), version)
            return runner

    def test_matching_version_passes(self):
        runner = self.check("m4b-merge 2.0.0\n")
        runner.assert_called_once_with(["m4b-merge", "--version"])

    def test_matching_version_strips_v_prefix(self):
        self.check("m4b-merge 2.0.0\n", version="v2.0.0")

    def test_wrong_version_fails(self):
        with patch.object(verifier, "run_checked") as runner:
            runner.return_value = Mock(stdout=b"m4b-merge 1.9.0\n")
            with self.assertRaises(verifier.VerifierFailure):
                verifier.check_version(Path("m4b-merge"), "2.0.0")

    def test_missing_trailing_newline_fails(self):
        with patch.object(verifier, "run_checked") as runner:
            runner.return_value = Mock(stdout=b"m4b-merge 2.0.0")
            with self.assertRaises(verifier.VerifierFailure):
                verifier.check_version(Path("m4b-merge"), "2.0.0")


class CheckELFTests(unittest.TestCase):
    """readelf fixture tests: static musl pass and each hard rejection."""

    HEADER = "Class:            ELF64\n" \
             "Data:             2's complement, little endian\n" \
             "Machine:          Advanced Micro Devices X86-64\n" \
             "Type:             EXEC (static-pie executable)\n"
    DYNAMIC_OK = "  Dynamic Section at offset 0x0 contains 0 entries\n"
    # Padded single-digit indices ("  [ 1] .note...") plus double-digit
    # ("  [10] .text") — the section-name parse must handle both cells.
    SECTIONS = (
        "  [Nr] Name              Type            Address"
        "          Off    Size   ES Flg Lk Inf Al\n"
        "  [ 0]                   NULL            0000000"
        "0000000000 00000000 00000000 00      0   0  0\n"
        "  [ 1] .note.gnu.property NOTE            0000000"
        "0004002c00 00000000 00   A   0   0  8\n"
        "  [10] .text             PROGBITS        0000000"
        "0004054aa0 03b00000 00  AX  0   0  32\n"
        "  [11] .rodata           PROGBITS        0000000"
        "0007b54aa0 00000000 00   A  0   0  32\n"
        "  [21] .shstrtab         STRTAB          0000000"
        "0007b4ac0 00000000 00   A   0   0  1\n")

    def run_readelf(self, **overrides):
        outputs = {
            "--file-header": self.HEADER,
            "--program-headers": "Program Headers:\n",
            "--dynamic": self.DYNAMIC_OK,
            "--version-info": "No version information section found.\n",
            "--sections": self.SECTIONS,
        }
        outputs.update(overrides)

        def fake_run(command, env=None):
            return Mock(stdout=outputs[command[2]].encode())

        return patch.object(verifier, "run_checked", side_effect=fake_run)

    def check(self, expect_stripped=False, fail_message=None, **overrides):
        with self.run_readelf(**overrides):
            if fail_message is None:
                verifier.check_elf(Path("m4b-merge"), "amd64",
                                   expect_stripped)
            else:
                with self.assertRaisesRegex(
                        verifier.VerifierFailure, fail_message):
                    verifier.check_elf(Path("m4b-merge"), "amd64",
                                       expect_stripped)

    def test_static_musl_passes(self):
        self.check(expect_stripped=True)

    def test_interp_segment_fails(self):
        self.check(fail_message="INTERP", **{
            "--program-headers": (
                "Program Headers:\n"
                "  Type           Offset   VirtAddr           PhysAddr\n"
                "  [ 1] INTERP   0x00000040 0x00000000004002c0 0x000000"
                "004002c0\n")})

    def test_needed_entry_fails(self):
        self.check(fail_message="NEEDED", **{
            "--dynamic": (
                "  Dynamic Section at offset 0x3f94ab0 contains 2 entries:\n"
                " 0x0000000000000001 (NEEDED)             Shared library: "
                "[libc.so.6]\n")})

    def test_glibc_symbol_fails(self):
        self.check(fail_message="GLIBC", **{
            "--version-info": (
                "No version information section found. No matching "
                "symbol versions section: GLIBC_2.2.5\n")})

    def test_unstripped_padded_index_symtab_fails(self):
        # Padded single-digit index cell ("  [ 1] .symtab") must trip
        # the stripped check.
        self.check(expect_stripped=True, fail_message="symtab", **{
            "--sections": self.SECTIONS + "  [ 1] .symtab         SYMTAB"
                                          "        0004054aa0 003b0000"
                                          " 18   A  9  1  8\n"})

    def test_unstripped_double_digit_index_symtab_fails(self):
        # Double-digit index cell ("  [10] .symtab") parses identically.
        self.check(
            expect_stripped=True, fail_message="symtab", **{
                "--sections": self.SECTIONS.replace(
                    "[10] .text             PROGBITS",
                    "[10] .symtab         SYMTAB")})

    def test_symtab_scan_skipped_without_stripped_flag(self):
        # The .symtab scan only runs with --expect-stripped.
        self.check(expect_stripped=False, **{
            "--sections": self.SECTIONS + "  [ 1] .symtab         SYMTAB"
                                          "        0004054aa0 003b0000"
                                          " 18   A  9  1  8\n"})

    def test_machine_mismatch_fails(self):
        self.check(fail_message="Machine", **{
            "--file-header": self.HEADER.replace(
                "Advanced Micro Devices X86-64", "AArch64")})

    def test_elf32_header_fails(self):
        self.check(fail_message="ELF64", **{
            "--file-header": self.HEADER.replace("ELF64", "ELF32")})

    def test_big_endian_header_fails(self):
        self.check(fail_message="little-endian", **{
            "--file-header": self.HEADER.replace(
                "2's complement, little endian",
                "2's complement, big endian")})

    def test_relocatable_type_fails(self):
        self.check(fail_message="EXEC", **{
            "--file-header": self.HEADER.replace(
                "Type:             EXEC (static-pie executable)",
                "Type:             REL (relocatable)")})

    def test_arm64_static_musl_passes(self):
        # The arm64 publication requires the AArch64 machine string.
        with self.run_readelf(**{"--file-header": self.HEADER.replace(
                "Advanced Micro Devices X86-64", "AArch64")}):
            verifier.check_elf(Path("m4b-merge"), "arm64",
                               expect_stripped=True)


class SmokeChecksTests(unittest.TestCase):
    """Output and loopback assertions with no media, no subprocesses."""

    EXPECTED = "/books/B012345678?region=us"

    def setUp(self):
        verifier.LoopbackHandler.recorded.clear()

    def run_checks(self, pre_recorded=()):
        server = verifier.ThreadingHTTPServer(("127.0.0.1", 0),
                                              verifier.LoopbackHandler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        verifier.LoopbackHandler.recorded.extend(pre_recorded)
        try:
            def fake_merge(binary, input_dir, output_dir, num_cpus,
                           extra_args=()):
                # The real merge consumes the input files without
                # mutating them; materialize them for the mutation check.
                for index in range(2):
                    Path(input_dir, f"{index}.bin").touch()

            with patch.object(verifier, "generate_media",
                              return_value=Path("fixture")) as media, \
                 patch.object(verifier, "merge",
                              side_effect=fake_merge) as merge, \
                 patch.object(verifier, "verify_output") as verify:
                result = verifier.smoke_checks(Path("m4b-merge"), "2.0.0",
                                               "amd64", server)
        finally:
            server.shutdown()
            server.server_close()
            thread.join()
        return result, media, merge, verify, server

    def test_merges_each_codec_and_verifies_output(self):
        # AAC: merge against the input copy + loopback metadata; MP3: plain
        # merge. Output assertions pin the two runs.
        result, media, merge, verify, server = self.run_checks(
            pre_recorded=(self.EXPECTED,))
        self.assertEqual(result, 0)
        self.assertEqual(len(merge.call_args_list), 2)
        aac, mp3 = merge.call_args_list
        self.assertEqual(Path(aac.args[1]).name, "inputs-aac")
        self.assertEqual(Path(aac.args[2]).name, "aac-output")
        self.assertEqual(aac.args[3], 4)
        args = aac.kwargs["extra_args"]
        self.assertIn("--metadata-source", args)
        self.assertIn("audnexus", args)
        api_url = args[args.index("--api-url") + 1]
        self.assertEqual(api_url,
                         f"http://localhost:{server.server_address[1]}")
        self.assertIn("-a", args)
        self.assertIn("B012345678", args)
        self.assertEqual(Path(mp3.args[1]).name, "inputs-mp3")
        self.assertEqual(Path(mp3.args[2]).name, "mp3-output")
        self.assertEqual(mp3.args[3], 4)
        self.assertEqual(mp3.kwargs.get("extra_args", ()), ())
        self.assertEqual(len(verify.call_args_list), 2)
        for call in verify.call_args_list:
            self.assertEqual(call.args[1], 4.0)
            self.assertEqual(call.args[2], 0.25)

    def test_loopback_failure_names_path_and_requests(self):
        # The audnexus client was never invoked: the recorded list lacks
        # the expected path, and the failure must name both.
        with self.assertRaisesRegex(
                verifier.VerifierFailure,
                r"Loopback server never received /books/B012345678"
                r"\?region=us; requests: \['/other'\]"):
            self.run_checks(pre_recorded=("/other",))


class VerifyOutputTests(unittest.TestCase):
    """verify_output stream/codec/duration assertions with the ffprobe
    layer stubbed; the success path keeps the real find_m4b guard."""

    def verify(self, audio, duration):
        with tempfile.TemporaryDirectory(prefix="m4b-verify-out-") as temp:
            directory = Path(temp) / "out"
            directory.mkdir()
            (directory / "merged.m4b").write_bytes(b"m4b")
            with patch.object(verifier, "probe_output",
                              return_value=(audio, duration)):
                return verifier.verify_output(directory, 4.0, 0.25)

    def test_zero_audio_streams_fail(self):
        with self.assertRaisesRegex(verifier.VerifierFailure,
                                    "Expected one AAC audio stream"):
            self.verify([], 4.0)

    def test_two_audio_streams_fail(self):
        with self.assertRaisesRegex(verifier.VerifierFailure,
                                    "Expected one AAC audio stream"):
            self.verify([{"codec_name": "aac"}] * 2, 4.0)

    def test_non_aac_codec_fails(self):
        with self.assertRaisesRegex(verifier.VerifierFailure,
                                    "Expected one AAC audio stream"):
            self.verify([{"codec_name": "mp3"}], 4.0)

    def test_duration_just_outside_tolerance_fails(self):
        with self.assertRaisesRegex(verifier.VerifierFailure,
                                    "deviates from 4s beyond 0.25s"):
            self.verify([{"codec_name": "aac"}], 4.26)

    def test_duration_just_inside_tolerance_succeeds(self):
        # Exactly one .m4b in the real output directory: the find_m4b
        # guard passes and the 0.24s deviation is within the 0.25s
        # tolerance.
        self.assertIsNone(self.verify([{"codec_name": "aac"}], 4.24))


class FindM4bTests(unittest.TestCase):
    """Exactly-one M4B guard: zero and two outputs are hard failures
    that name the found paths."""

    def find(self, names):
        with tempfile.TemporaryDirectory(prefix="m4b-find-") as temp:
            directory = Path(temp)
            for name in names:
                (directory / name).write_bytes(b"m4b")
            return verifier.find_m4b(directory), directory

    def test_single_output_returned(self):
        found, _ = self.find(["merged.m4b"])
        self.assertEqual(found.name, "merged.m4b")

    def test_zero_outputs_fail(self):
        with self.assertRaisesRegex(
                verifier.VerifierFailure,
                r"Expected exactly one M4B output, got: \[\]$"):
            self.find([])

    def test_two_outputs_named_in_failure(self):
        with tempfile.TemporaryDirectory(prefix="m4b-find-") as temp:
            directory = Path(temp)
            for name in ("a.m4b", "b.m4b"):
                (directory / name).write_bytes(b"m4b")
            try:
                verifier.find_m4b(directory)
            except verifier.VerifierFailure as failure:
                self.assertIn(str(directory / "a.m4b"), str(failure))
                self.assertIn(str(directory / "b.m4b"), str(failure))
            else:
                self.fail("find_m4b accepted two outputs")


class SmokeInputMutationTests(unittest.TestCase):
    """A merge that deletes an input file must fail the smoke gate and
    name the mutated directory."""

    def test_merge_deleting_input_fails_naming_directory(self):
        verifier.LoopbackHandler.recorded.clear()
        server = verifier.ThreadingHTTPServer(("127.0.0.1", 0),
                                              verifier.LoopbackHandler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            def fake_merge(binary, input_dir, output_dir, num_cpus,
                           extra_args=()):
                input_dir = Path(input_dir)
                for index in range(2):
                    (input_dir / f"{index}.bin").touch()
                (input_dir / "0.bin").unlink()

            with patch.object(verifier, "generate_media",
                              return_value=Path("fixture")), \
                 patch.object(verifier, "merge",
                              side_effect=fake_merge), \
                 patch.object(verifier, "verify_output"):
                with self.assertRaisesRegex(
                        verifier.VerifierFailure,
                        r"smoke inputs in .+/inputs-aac were mutated"):
                    verifier.smoke_checks(Path("m4b-merge"), "2.0.0",
                                          "amd64", server)
        finally:
            server.shutdown()
            server.server_close()
            thread.join()


class BenchmarkCaseWarmupTests(unittest.TestCase):
    """Warmups are keyed by role: identical basenames never clobber
    each other's warmup output."""

    def test_warmups_keyed_by_role(self):
        with tempfile.TemporaryDirectory(prefix="m4b-bench-") as temp:
            temp_dir = Path(temp) / "runs"
            temp_dir.mkdir()
            for source_name in ("baseline-src", "candidate-src"):
                source = Path(temp) / source_name
                source.mkdir()
                (source / "m4b-merge").write_bytes(b"variant")
            workload = Path(temp) / "aac-copy"
            workload.mkdir()

            with patch.object(verifier, "merge") as merge, \
                 patch.object(verifier, "verify_output"):
                cases = list(verifier.benchmark_case(
                    workload, 30.0 * 256, temp_dir,
                    Path(temp, "baseline-src", "m4b-merge"),
                    Path(temp, "candidate-src", "m4b-merge")))

            self.assertEqual(len(cases), 2)
            warmups = [str(call.args[2]) for call in merge.call_args_list
                       if Path(call.args[2]).parent.name
                       .startswith("warmup-")]
            for num_cpus in (1, 4):
                label = f"aac-copy-cpus{num_cpus}"
                baseline = [p for p in warmups
                            if p.endswith(f"warmup-{label}/baseline")]
                candidate = [p for p in warmups
                             if p.endswith(f"warmup-{label}/candidate")]
                # One warmup per role per case, in distinct directories.
                self.assertEqual(len(baseline), 1)
                self.assertEqual(len(candidate), 1)
                self.assertNotIn(baseline[0], candidate)
            self.assertEqual(len(warmups), 4)


class RunBenchmarkTests(unittest.TestCase):
    """Hard-stop vs advisory scoping inside run_benchmark."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="m4b-bench-out-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        for variant in ("baseline", "candidate"):
            binary = self.root / variant / "m4b-merge"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(f"{variant}-binary".encode())
            binary.chmod(0o755)
        self.baseline = self.root / "baseline" / "m4b-merge"
        self.candidate = self.root / "candidate" / "m4b-merge"
        self.report = self.root / "evidence" / "benchmark.json"

    def run_case(self, parity=None, benchmark_case=None):
        workload = self.root / "workload"
        workload.mkdir(exist_ok=True)

        def fake_run_checked(command, env=None):
            if command[0] in ("ffmpeg", "ffprobe"):
                decoded = f"{command[0]} {command[1]}"
            else:
                decoded = "m4b-merge 2.0.0\n"
            return Mock(stdout=Mock(decode=Mock(return_value=decoded)))

        spec = {
            "run_checked": Mock(side_effect=fake_run_checked),
            "require_native": Mock(),
            "generate_media": Mock(return_value=Path("seed")),
            "build_fixtures": Mock(
                return_value=[(workload, 30.0 * 256)]),
            "merge": Mock(),
            "verify_output": Mock(),
        }
        if parity is not None:
            spec["parity_upper_bound"] = parity
        if benchmark_case is not None:
            spec["benchmark_case"] = benchmark_case
        with contextlib.ExitStack() as stack:
            for name, replacement in spec.items():
                stack.enter_context(
                    patch.object(verifier, name, replacement))
            result = verifier.run_benchmark(self.baseline, self.candidate,
                                            "v2.0.0", "amd64", self.report)
        spec["require_native"].assert_called_once_with("amd64")
        return result

    def test_inconclusive_bound_keeps_report_and_returns_zero(self):
        result = self.run_case(
            parity=Mock(side_effect=ValueError("bound cannot resolve")))
        self.assertEqual(result, 0)
        self.assertTrue(self.report.is_file())
        report = json.loads(self.report.read_text())
        self.assertFalse(report["passed"])
        # One inconclusive case per workload/cpus combination; every
        # measurement case is marked as unresolvable, not deleted.
        self.assertTrue(all(case == {"case": "inconclusive",
                                     "error": "bound cannot resolve"}
                            for case in report["cases"]))
        self.assertGreaterEqual(len(report["cases"]), 1)

    def test_case_generator_hard_failures_propagate_without_report(self):
        # A ValueError (probe) or VerifierFailure (functional merge
        # failure) raised by the case generator is outside the
        # parity-only try block: it must propagate and the report must
        # never be written.
        for error in (verifier.VerifierFailure("merge failed"),
                      ValueError("probe output unparsable")):
            with self.subTest(error=str(error)):

                def generator(workload, expected_duration, temp_dir,
                              baseline, candidate):
                    yield {"case": "workload-cpus1", "cpus": 1,
                           "baseline": [1.0] * PAIRS,
                           "candidate": [1.0] * PAIRS}
                    raise error

                with self.assertRaises(type(error)):
                    self.run_case(benchmark_case=generator)
                self.assertFalse(self.report.exists())


class MainDispatchTests(unittest.TestCase):
    """main() maps benchmark VerifierFailure to exit 1 and success to
    exit 0."""

    ARGV = ["verify_linux_asset.py", "benchmark",
            "--baseline", "baseline", "--candidate", "candidate",
            "--version", "v2.0.0", "--arch", "amd64",
            "--report", "report.json"]

    def test_benchmark_verifier_failure_maps_to_one(self):
        with patch.object(sys, "argv", self.ARGV), \
             patch.object(verifier, "run_benchmark",
                          side_effect=verifier.VerifierFailure("gate")):
            self.assertEqual(verifier.main(), 1)

    def test_successful_benchmark_dispatch_maps_to_zero(self):
        with patch.object(sys, "argv", self.ARGV), \
             patch.object(verifier, "run_benchmark",
                          return_value=0) as benchmark:
            self.assertEqual(verifier.main(), 0)
            benchmark.assert_called_once_with("baseline", "candidate",
                                              "v2.0.0", "amd64",
                                              "report.json")


class NativeGateTests(unittest.TestCase):
    """Host/architecture and binary shape guards with no subprocesses."""

    def test_machine_mismatch_fails(self):
        with patch.object(verifier, "platform") as platform:
            platform.system.return_value = "Linux"
            platform.machine.return_value = "aarch64"
            with self.assertRaisesRegex(
                    verifier.VerifierFailure,
                    r"Native gate requires x86_64 host, got aarch64"):
                verifier.require_native("amd64")

    def test_matching_native_host_passes(self):
        with patch.object(verifier, "platform") as platform:
            platform.system.return_value = "Linux"
            platform.machine.return_value = "x86_64"
            self.assertIsNone(verifier.require_native("amd64"))

    def test_directory_is_not_regular(self):
        with tempfile.TemporaryDirectory(prefix="m4b-gate-") as temp:
            directory = Path(temp) / "binary"
            directory.mkdir()
            with self.assertRaisesRegex(verifier.VerifierFailure,
                                        "not a regular file"):
                verifier.regular_executable(directory,
                                            "Benchmark binary")

    def test_symlink_is_not_regular(self):
        with tempfile.TemporaryDirectory(prefix="m4b-gate-") as temp:
            target = Path(temp) / "binary"
            target.write_bytes(b"bin")
            target.chmod(0o755)
            link = Path(temp) / "link"
            link.symlink_to(target)
            with self.assertRaisesRegex(verifier.VerifierFailure,
                                        "not a regular file"):
                verifier.regular_executable(link, "Candidate binary")

    def test_missing_exec_bit_fails(self):
        with tempfile.TemporaryDirectory(prefix="m4b-gate-") as temp:
            binary = Path(temp) / "binary"
            binary.write_bytes(b"bin")
            with self.assertRaisesRegex(verifier.VerifierFailure,
                                        "not executable"):
                verifier.regular_executable(binary, "Baseline binary")


if __name__ == "__main__":
    unittest.main()
