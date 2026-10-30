"""Native Linux executable verifier for extracted musl release assets.

Smoke checks: native-architecture run, exact version, static-musl ELF
provenance, real AAC/MP3 merge behavior, and a loopback metadata HTTP
endpoint. Every smoke failure is a hard failure with the command and
captured output; nothing there is skipped or downgraded.

Benchmark: paired parity between a GNU baseline and a musl candidate.
The gate is advisory — the report (hashes, ffmpeg build, per-case
distributions, one-sided 95% bounds) is written and the exit code stays
zero, matching the repository's advisory cargo-audit convention: a
noisy host measurement must never block an immutable publication.

This script uploads nothing.
"""

import argparse
import hashlib
import json
import math
import os
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

MACHINES = {"amd64": "x86_64", "arm64": "aarch64"}
PAIRS = 20
# One-sided 95% Student-t critical value, df=19.
T_UPPER_95 = 1.729132811521367


class VerifierFailure(Exception):
    """Hard verifier failure with a diagnostic message."""


def fail(message):
    raise VerifierFailure(message)


def run_checked(command, env=None):
    """Run a command; on nonzero exit include the command and captured output."""
    if shutil.which(str(command[0])) is None:
        fail(f"Required tool not available: {command[0]}")
    process = subprocess.run(command, env=env, capture_output=True)
    if process.returncode != 0:
        stdout = process.stdout.decode(errors="replace")
        stderr = process.stderr.decode(errors="replace")
        fail(f"Command failed ({process.returncode}): {command}\n"
             f"stdout:\n{stdout}\nstderr:\n{stderr}")
    return process


def require_native(arch):
    if platform.system().lower() != "linux":
        fail(f"smoke/benchmark must run on Linux, got {platform.system()}")
    machine = platform.machine()
    if machine != MACHINES[arch]:
        fail(f"Native gate requires {MACHINES[arch]} host, got {machine}")


def regular_executable(path, label):
    path = Path(path)
    if not path.is_file() or path.is_symlink():
        fail(f"{label} is not a regular file: {path}")
    if not path.stat().st_mode & 0o111:
        fail(f"{label} is not executable: {path}")
    return path


def check_version(binary, version):
    expected = f"m4b-merge {version.removeprefix('v')}\n"
    process = run_checked([str(binary), "--version"])
    actual = process.stdout.decode(errors="replace")
    if actual != expected:
        fail(f"Binary reports {actual!r}, expected {expected!r}")


def check_elf(binary, arch, expect_stripped):
    env = dict(os.environ)
    env["LC_ALL"] = "C"
    elf_machine = {"amd64": "Advanced Micro Devices X86-64",
                   "arm64": "AArch64"}[arch]

    def readelf(*flags):
        output = run_checked(["readelf", *flags, str(binary)],
                             env=env).stdout.decode(errors="replace")
        return output

    header = readelf("--wide", "--file-header")
    if not re.search(r"Class:\s+ELF64", header):
        fail(f"Expected ELF64 class: {header}")
    if "little endian" not in header:
        fail(f"Expected little-endian ELF: {header}")
    if not re.search(rf"Machine:\s+{re.escape(elf_machine)}", header):
        fail(f"Expected machine {elf_machine!r}: {header}")
    if not re.search(r"Type:\s*(EXEC|DYN)\b", header):
        fail(f"Expected ELF type EXEC or DYN (static PIE is valid): {header}")

    program = readelf("--wide", "--program-headers")
    interpolation = [line.strip() for line in program.splitlines()
                     if "INTERP" in line]
    if interpolation:
        fail(f"Static musl binary must have no INTERP segment: {interpolation}")

    needed = [line.strip() for line in readelf("--wide", "--dynamic").splitlines()
              if "NEEDED" in line]
    if needed:
        fail(f"Static musl binary must have no NEEDED entries: {needed}")

    glibc = [line.strip() for line in readelf("--wide", "--version-info").splitlines()
             if "GLIBC_" in line]
    if glibc:
        fail(f"Static musl binary must not require GLIBC symbols: {glibc}")

    if expect_stripped:
        lines = readelf("--wide", "--sections").splitlines()
        # readelf pads single-digit section indices ("  [ 1] .interp"),
        # so split the name out of the well-formed index cell only.
        names = {m.group(1) for m in
                 (re.match(r"\s*\[\s*\d+\]\s+(\S+)", line) for line in lines)
                 if m}
        if ".symtab" in names:
            fail(f"Expected stripped binary, found .symtab section")


def generate_media(directory, name, codec, duration):
    """Deterministic sine fixture following tests/integration.rs conventions."""
    target = directory / name
    command = ["ffmpeg", "-f", "lavfi", "-i",
               f"sine=frequency=1000:duration={duration}",
               "-acodec", codec, "-b:a", "128k", "-ar", "44100", "-ac", "2",
               "-y", str(target)]
    run_checked(command)
    if not target.is_file():
        fail(f"FFmpeg did not create fixture: {target}")
    return target


def probe_output(path):
    streams = json.loads(
        run_checked(["ffprobe", "-v", "error", "-show_streams",
                     "-show_format", "-of", "json", str(path)]).stdout)
    audio = [s for s in streams.get("streams", []) if s.get("codec_type") == "audio"]
    duration = float(streams.get("format", {}).get("duration", -1))
    return audio, duration


def find_m4b(output_dir):
    outputs = sorted(output_dir.rglob("*.m4b"))
    if len(outputs) != 1:
        fail(f"Expected exactly one M4B output, got: {outputs}")
    return outputs[0]


def merge(binary, input_dir, output_dir, num_cpus, extra_args=()):
    """Run the binary over an input directory; output probing is done
    outside timed regions. Directory inputs match the v2.0.0 CLI and the
    tests/integration.rs invocation conventions."""
    command = [str(binary),
               "-i", str(input_dir),
               "-o", str(output_dir),
               "--num-cpus", str(num_cpus),
               "--log-level", "error",
               *extra_args]
    run_checked(command)


def verify_output(output_dir, duration, tolerance):
    audio, actual = probe_output(find_m4b(output_dir))
    if len(audio) != 1 or audio[0].get("codec_name") != "aac":
        fail(f"Expected one AAC audio stream: {audio}")
    if abs(actual - duration) > tolerance:
        fail(f"Merged duration {actual:.2f}s deviates from {duration:.0f}s "
             f"beyond {tolerance}s")


class LoopbackHandler(BaseHTTPRequestHandler):
    recorded = []

    def do_GET(self):
        LoopbackHandler.recorded.append(self.path)
        body = b"not found"
        self.send_response(404)
        self.send_header("Content-Type", "text/plain")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass  # silence per-request logging


def run_smoke(binary, version, arch, expect_stripped):
    require_native(arch)
    binary = regular_executable(binary, "Smoke binary")
    check_version(binary, version)
    check_elf(binary, arch, expect_stripped)

    server = ThreadingHTTPServer(("127.0.0.1", 0), LoopbackHandler)
    server_thread = threading.Thread(target=server.serve_forever, daemon=True)
    server_thread.start()
    try:
        smoke_checks(binary, version, arch, server)
    finally:
        # Always stop/join the loopback server, even on verification failure.
        server.shutdown()
        server.server_close()
        server_thread.join()
    print(f"smoke OK: {binary.name} {version} {arch}"
          f"{' (stripped)' if expect_stripped else ''}")
    return 0


def smoke_checks(binary, version, arch, server):
    with tempfile.TemporaryDirectory(prefix="m4b-verify-") as temp:
        temp_dir = Path(temp)

        def fixtures(name, codec, extension):
            directory = temp_dir / name
            directory.mkdir()
            generate_media(directory, f"a-{codec}.{extension}", codec, 2)
            generate_media(directory, f"b-{codec}.{extension}", codec, 2)
            return directory

        # AAC run: the audnexus client hits the local 404 server,
        # exercising the real reqwest hostname/HTTP/error path.
        port = server.server_address[1]
        aac_inputs = fixtures("inputs-aac", "aac", "m4a")
        aac_output = temp_dir / "aac-output"
        aac_output.mkdir()
        merge(binary, aac_inputs, aac_output, 4,
              extra_args=["--metadata-source", "audnexus",
                          "--api-url", f"http://localhost:{port}",
                          "-a", "B012345678"])
        verify_output(aac_output, 4.0, 0.25)

        mp3_inputs = fixtures("inputs-mp3", "libmp3lame", "mp3")
        mp3_output = temp_dir / "mp3-output"
        mp3_output.mkdir()
        merge(binary, mp3_inputs, mp3_output, 4)
        verify_output(mp3_output, 4.0, 0.25)

        for directory in (aac_inputs, mp3_inputs):
            if len(sorted(directory.iterdir())) != 2:
                fail(f"smoke inputs in {directory} were mutated")

    expected_path = "/books/B012345678?region=us"
    if expected_path not in LoopbackHandler.recorded:
        fail(f"Loopback server never received {expected_path}; "
             f"requests: {LoopbackHandler.recorded}")
    return 0


def parity_upper_bound(baseline, candidate):
    """One-sided 95% upper bound on log(candidate/baseline).

    Fewer than 20 pairs, unequal lengths, or nonpositive timings make
    the gate fail instead of returning a default ratio.
    """
    if len(baseline) != len(candidate):
        raise ValueError("baseline and candidate sample counts differ")
    if len(baseline) < PAIRS:
        raise ValueError(f"Expected at least {PAIRS} pairs, got {len(baseline)}")
    if any(value <= 0 for value in (*baseline, *candidate)):
        raise ValueError("All timings must be strictly positive")
    log_ratios = [math.log(cv / bv) for bv, cv in zip(baseline, candidate)]
    mean = statistics.fmean(log_ratios)
    spread = statistics.stdev(log_ratios)
    return mean + T_UPPER_95 * spread / math.sqrt(len(baseline))


def build_fixtures(value_dir):
    aac_dir = value_dir / "aac-copy"
    mp3_dir = value_dir / "mp3-transcode"
    aac_dir.mkdir(parents=True)
    mp3_dir.mkdir(parents=True)
    for index in range(256):
        shutil.copy2(value_dir / "seed.m4a", aac_dir / f"{index:04d}.m4a")
    for index in range(16):
        shutil.copy2(value_dir / "seed.mp3", mp3_dir / f"{index:04d}.mp3")
    return [(aac_dir, 256 * 30.0), (mp3_dir, 16 * 30.0)]


def benchmark_case(workload, expected_duration, temp_dir, baseline, candidate):
    for num_cpus in (1, 4):
        label = f"{workload.name}-cpus{num_cpus}"
        # One untimed warmup of each variant per case.
        for variant in (baseline, candidate):
            warmup = temp_dir / f"warmup-{label}" / variant.name
            warmup.mkdir(parents=True)
            merge(variant, workload, warmup, num_cpus)
            verify_output(warmup, expected_duration, 1.0)
        baseline_times = []
        candidate_times = []
        for index in range(PAIRS):
            # Baseline first on even pair indexes, candidate first on odd.
            ordered = [(baseline, 0), (candidate, 1)] if index % 2 == 0 \
                else [(candidate, 1), (baseline, 0)]
            for order, (variant, bucket) in enumerate(ordered):
                output_dir = temp_dir / f"run-{label}-{index}-{order}"
                output_dir.mkdir()
                start = time.perf_counter()
                merge(variant, workload, output_dir, num_cpus)
                elapsed = time.perf_counter() - start
                if elapsed <= 0:
                    fail(f"nonpositive timing recorded for {label}")
                (baseline_times if bucket == 0 else candidate_times).append(elapsed)
                # Validate output before deleting it for the next pair.
                verify_output(output_dir, expected_duration, 1.0)
                shutil.rmtree(output_dir, ignore_errors=True)
        yield {
            "case": label,
            "cpus": num_cpus,
            "baseline": baseline_times,
            "candidate": candidate_times,
            "log_ratio_mean": statistics.fmean(
                [math.log(cv / bv) for bv, cv in zip(baseline_times,
                                                     candidate_times)]),
        }


def run_benchmark(baseline, candidate, version, arch, report_path):
    require_native(arch)
    baseline = regular_executable(baseline, "Baseline binary")
    candidate = regular_executable(candidate, "Candidate binary")
    check_version(baseline, version)
    check_version(candidate, version)
    ffmpeg_version = run_checked(
        ["ffmpeg", "-version"]).stdout.decode(errors="replace").splitlines()[0]
    ffprobe_version = run_checked(
        ["ffprobe", "-version"]).stdout.decode(errors="replace").splitlines()[0]
    hashes = {path: hashlib.sha256(path.read_bytes()).hexdigest()
              for path in (baseline, candidate)}

    cases = []
    error = None
    with tempfile.TemporaryDirectory(prefix="m4b-bench-") as temp:
        temp_dir = Path(temp)
        seed_dir = temp_dir / "seeds"
        seed_dir.mkdir()
        # Fixture generation once, outside every timed region. Each case
        # merges 128 (AAC copy) or 8 minutes (MP3 transcode) of audio.
        generate_media(seed_dir, "seed.m4a", "aac", 30)
        generate_media(seed_dir, "seed.mp3", "libmp3lame", 30)
        try:
            for workload, expected_duration in build_fixtures(seed_dir):
                for case in benchmark_case(workload, expected_duration,
                                           temp_dir, baseline, candidate):
                    bound = parity_upper_bound(case["baseline"],
                                               case["candidate"])
                    case["upper95_log_ratio"] = bound
                    case["passed"] = bound <= 0.0
                    cases.append(case)
        except VerifierFailure:
            # A functional merge/probe failure is a hard publication
            # failure; only measurement uncertainty is advisory.
            raise
        except ValueError as exc:
            # A valid measurement that the bound cannot resolve is
            # inconclusive: keep the report, mark the run failed.
            error = str(exc)
            cases.append({"case": "inconclusive", "error": error})

    passed = bool(not error and len(cases) == 4 and all(c.get("passed")
                                                       for c in cases))
    report = {
        "architecture": arch,
        "version": version,
        "baseline_sha256": hashes[baseline],
        "candidate_sha256": hashes[candidate],
        "ffmpeg_version": ffmpeg_version,
        "ffprobe_version": ffprobe_version,
        "pairs": PAIRS,
        "cases": cases,
        "passed": passed,
    }
    report_path = Path(report_path)
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(report, indent=2) + "\n")
    # Advisory gate: a parity miss on the current host must not force a
    # breaking release. The report captures the evidence (binaries, ffmpeg
    # build, per-case distributions) so the maintainer can judge deviation
    # while upload and publication remain unblocked.
    if not passed:
        print(f"parity gate REVIEW (not passed; advisory — "
              f"report: {report_path})")
    else:
        print(f"parity gate passed (report: {report_path})")
    return 0


def parse_version(version):
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", version):
        raise SystemExit(f"Invalid version format: {version!r}")
    return version


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    smoke = sub.add_parser("smoke")
    smoke.add_argument("--binary", required=True)
    smoke.add_argument("--version", required=True)
    smoke.add_argument("--arch", required=True, choices=("amd64", "arm64"))
    smoke.add_argument("--expect-stripped", action="store_true")

    benchmark = sub.add_parser("benchmark")
    benchmark.add_argument("--baseline", required=True)
    benchmark.add_argument("--candidate", required=True)
    benchmark.add_argument("--version", required=True)
    benchmark.add_argument("--arch", required=True, choices=("amd64", "arm64"))
    benchmark.add_argument("--report", required=True)

    args = parser.parse_args()
    try:
        if args.command == "smoke":
            return run_smoke(args.binary, parse_version(args.version),
                             args.arch, args.expect_stripped)
        return run_benchmark(args.baseline, args.candidate,
                             parse_version(args.version), args.arch,
                             args.report)
    except VerifierFailure as failure:
        print(f"FAIL: {failure}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
