"""Exercise the publisher's actual Docker/gh command consumers without publication."""

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location("publish_release", Path(__file__).with_name("publish_release.py"))
publisher = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(publisher)
SHA = "a" * 40
AMD = "sha256:" + "1" * 64
ARM = "sha256:" + "2" * 64
IMAGE = "registry.example/m4b"
VERSION = "v1.2.3"


class PublisherTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.responses = {}
        self.state = self.root / "responses.json"
        self.calls = self.root / "calls.jsonl"
        adapter = self.root / "docker"
        adapter.write_text("""#!/usr/bin/env python3
import json, os, pathlib, sys
command = [pathlib.Path(sys.argv[0]).name, *sys.argv[1:]]
with open(os.environ['PUBLISH_CALLS'], 'a') as stream:
    stream.write(json.dumps(command) + '\\n')
responses = json.loads(pathlib.Path(os.environ['PUBLISH_RESPONSES']).read_text())
response = responses.get(json.dumps(command))
if response is None:
    sys.stderr.write('Unexpected adapter command: ' + repr(command))
    sys.exit(99)
sys.stdout.write(response.get('stdout', ''))
sys.stderr.write(response.get('stderr', ''))
sys.exit(response.get('code', 0))
""")
        adapter.chmod(0o755)
        (self.root / "gh").symlink_to(adapter)
        self.addCleanup(patch.stopall)
        patch.dict(os.environ, {"PATH": f"{self.root}:{os.environ['PATH']}",
                               "PUBLISH_RESPONSES": str(self.state),
                               "PUBLISH_CALLS": str(self.calls)}).start()
        self.args = argparse.Namespace(image=IMAGE, sha=SHA, version=VERSION,
                                       arch="amd64", required=False, amd64=AMD, arm64=ARM,
                                       repo="owner/repo", tag=VERSION,
                                       artifacts=str(self.root / "artifacts"))

    def response(self, command, value=None, code=0, error=""):
        self.responses[json.dumps(command)] = {
            "stdout": json.dumps(value) if value is not None else "",
            "code": code, "stderr": error}
        self.state.write_text(json.dumps(self.responses))

    def inspect(self, ref, value=None, field="Manifest", error=""):
        self.response(["docker", "buildx", "imagetools", "inspect", ref,
                       "--format", "{{json ." + field + "}}"], value,
                      1 if error else 0, error)

    def native(self, ref, value, arch="amd64", sha=SHA):
        self.inspect(ref, {"digest": value})
        self.inspect(f"{IMAGE}@{value}", {"os": "linux", "architecture": arch,
                     "config": {"Labels": {"org.opencontainers.image.revision": sha,
                                           "org.opencontainers.image.version": VERSION}}}, "Image")

    def missing(self, ref):
        self.inspect(ref, error=f"ERROR: {ref}: not found")

    def called(self):
        return [json.loads(line) for line in self.calls.read_text().splitlines()]

    def prepare_manifest(self):
        for arch, value in (("amd64", AMD), ("arm64", ARM)):
            self.native(f"{IMAGE}@{value}", value, arch)
            self.native(f"{IMAGE}:{VERSION}-{arch}", value, arch)
        self.inspect(f"{IMAGE}:{VERSION}", {"manifests": [
            {"digest": value, "platform": {"os": "linux", "architecture": arch}}
            for arch, value in (("amd64", AMD), ("arm64", ARM))]})

    def test_stage_reuses_verified_digest(self):
        self.native(f"{IMAGE}:sha-{SHA}-amd64", AMD)
        output = self.root / "output"
        with patch.dict(os.environ, {"GITHUB_OUTPUT": str(output)}):
            publisher.stage(self.args)
        self.assertEqual(output.read_text(), f"exists=true\ndigest={AMD}\n")

    def test_only_explicit_not_found_allows_build(self):
        ref = f"{IMAGE}:sha-{SHA}-amd64"
        self.missing(ref)
        output = self.root / "output"
        with patch.dict(os.environ, {"GITHUB_OUTPUT": str(output)}):
            publisher.stage(self.args)
        self.assertEqual(output.read_text(), "exists=false\ndigest=\n")
        for error in ("unauthorized", "connection refused", "rate limit exceeded"):
            self.inspect(ref, error=error)
            with self.assertRaises(subprocess.CalledProcessError):
                publisher.stage(self.args)

    def test_required_stage_cannot_be_missing(self):
        self.args.required = True
        self.missing(f"{IMAGE}:sha-{SHA}-amd64")
        with self.assertRaises(subprocess.CalledProcessError):
            publisher.stage(self.args)

    def test_wrong_revision_is_not_reused(self):
        self.native(f"{IMAGE}:sha-{SHA}-amd64", AMD, sha="b" * 40)
        with self.assertRaisesRegex(ValueError, "provenance"):
            publisher.stage(self.args)

    def test_complete_manifest_is_noop(self):
        self.prepare_manifest()
        publisher.manifest(self.args)
        self.assertFalse(any("create" in command for command in self.called()))

    def test_partial_manifest_uses_captured_digests(self):
        self.prepare_manifest()
        self.missing(f"{IMAGE}:{VERSION}-arm64")
        self.missing(f"{IMAGE}:{VERSION}")
        architecture = ["docker", "buildx", "imagetools", "create", "--prefer-index=false",
                        "--tag", f"{IMAGE}:{VERSION}-arm64", f"{IMAGE}@{ARM}"]
        canonical = ["docker", "buildx", "imagetools", "create", "--tag",
                     f"{IMAGE}:{VERSION}", f"{IMAGE}@{AMD}", f"{IMAGE}@{ARM}"]
        self.response(architecture)
        self.response(canonical)
        publisher.manifest(self.args)
        self.assertEqual([cmd for cmd in self.called() if "create" in cmd], [architecture, canonical])

    def test_conflicting_architecture_never_overwritten(self):
        self.prepare_manifest()
        self.native(f"{IMAGE}:{VERSION}-amd64", "sha256:" + "3" * 64)
        with self.assertRaisesRegex(ValueError, "differs"):
            publisher.manifest(self.args)
        self.assertFalse(any("create" in cmd for cmd in self.called()))

    def test_conflicting_index_prevents_partial_writes(self):
        self.prepare_manifest()
        self.missing(f"{IMAGE}:{VERSION}-arm64")
        self.inspect(f"{IMAGE}:{VERSION}", {"manifests": []})
        with self.assertRaisesRegex(ValueError, "manifest differs"):
            publisher.manifest(self.args)
        self.assertFalse(any("create" in cmd for cmd in self.called()))

    def prepare_assets(self):
        self.args.repo = "owner/repo"
        self.args.tag = VERSION
        self.args.artifacts = str(self.root / "artifacts")
        self.asset_list = []
        for number, name in enumerate(("m4b-merge-linux", "m4b-merge-macos", "m4b-merge-windows.exe")):
            path = Path(self.args.artifacts) / name / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(name.encode())
            self.asset_list.append({"id": number, "name": name,
                                    "digest": "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()})
        self.release_response()

    def release_response(self):
        self.response(["gh", "api", "--hostname", "github.com",
                       f"repos/owner/repo/releases/tags/{VERSION}"], {"assets": self.asset_list})

    def test_assets_match_and_windows_extension_preserved(self):
        self.prepare_assets()
        publisher.assets(self.args)
        self.assertFalse(any("upload" in cmd for cmd in self.called()))

    def test_partial_upload_retry_reuses_labeled_asset_after_rebuild(self):
        self.prepare_assets()
        canonical = b"previous runner's canonical linux binary"
        linux = self.asset_list[0]
        linux.update({"label": publisher.asset_label(SHA, VERSION,
                      "sha256:" + hashlib.sha256(canonical).hexdigest()),
                      "digest": "sha256:" + hashlib.sha256(canonical).hexdigest()})
        (Path(self.args.artifacts) / linux["name"] / linux["name"]).write_bytes(b"different rebuild bytes")
        self.asset_list.pop()  # Windows upload was the point of interruption.
        self.release_response()
        windows = Path(self.args.artifacts) / "m4b-merge-windows.exe" / "m4b-merge-windows.exe"
        digest = "sha256:" + hashlib.sha256(windows.read_bytes()).hexdigest()
        label = publisher.asset_label(SHA, VERSION, digest)
        upload = ["gh", "release", "upload", VERSION, f"{windows}#{label}",
                  "--repo", "owner/repo"]
        self.response(upload)
        publisher.assets(self.args)
        self.assertEqual([cmd for cmd in self.called() if "upload" in cmd], [upload])

    def test_labeled_asset_wrong_source_or_digest_fails_before_upload(self):
        for label, digest in (
                (publisher.asset_label("b" * 40, VERSION, "sha256:" + "1" * 64),
                 "sha256:" + "1" * 64),
                (publisher.asset_label(SHA, VERSION, "sha256:" + "1" * 64),
                 "sha256:" + "2" * 64),
                ("m4b-publish-v1;sha=invalid", "sha256:" + "1" * 64)):
            with self.subTest(label=label, digest=digest):
                self.prepare_assets()
                self.asset_list[0]["label"] = label
                self.asset_list[0]["digest"] = digest
                self.asset_list.pop()
                self.release_response()
                with self.assertRaises(ValueError):
                    publisher.assets(self.args)
                self.assertFalse(any("upload" in cmd for cmd in self.called()))
                self.calls.write_text("")

    def test_assets_fill_only_missing_with_explicit_repository(self):
        self.prepare_assets()
        self.asset_list.pop()
        self.release_response()
        path = Path(self.args.artifacts) / "m4b-merge-windows.exe" / "m4b-merge-windows.exe"
        label = publisher.asset_label(SHA, VERSION,
                                     "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest())
        upload = ["gh", "release", "upload", VERSION, f"{path}#{label}", "--repo", "owner/repo"]
        self.response(upload)
        publisher.assets(self.args)
        self.assertEqual([cmd for cmd in self.called() if "upload" in cmd], [upload])


    def test_legacy_asset_download_is_hashed_before_reuse(self):
        self.prepare_assets()
        asset = self.asset_list[0]
        asset.pop("digest")
        self.release_response()
        command = ["gh", "api", "--hostname", "github.com", "-H",
                   "Accept: application/octet-stream", "repos/owner/repo/releases/assets/0"]
        self.response(command)
        self.responses[json.dumps(command)]["stdout"] = asset["name"]
        self.state.write_text(json.dumps(self.responses))
        publisher.assets(self.args)
        self.assertIn(command, self.called())
        self.assertFalse(any("upload" in cmd for cmd in self.called()))

    def test_registry_transport_failure_never_creates_version_tags(self):
        self.prepare_manifest()
        self.inspect(f"{IMAGE}:{VERSION}", error="connection refused")
        with self.assertRaises(subprocess.CalledProcessError):
            publisher.manifest(self.args)
        self.assertFalse(any("create" in cmd for cmd in self.called()))

    def test_asset_conflict_prevents_all_uploads(self):
        self.prepare_assets()
        self.asset_list.pop(0)
        self.asset_list[-1]["digest"] = "sha256:" + "f" * 64
        self.release_response()
        with self.assertRaisesRegex(ValueError, "differs"):
            publisher.assets(self.args)
        self.assertFalse(any("upload" in cmd for cmd in self.called()))


if __name__ == "__main__":
    unittest.main()
