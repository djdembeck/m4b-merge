"""Publish-only image and asset reconciliation; never replace existing outputs."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess


DIGEST = re.compile(r"sha256:[0-9a-f]{64}\Z")


def run(*args):
    return subprocess.run(args, check=True, capture_output=True).stdout


def inspect(ref, field="Manifest", missing=False):
    try:
        return json.loads(run("docker", "buildx", "imagetools", "inspect", ref,
                              "--format", "{{json ." + field + "}}"))
    except subprocess.CalledProcessError as error:
        message = error.stderr.decode(errors="replace").lower()
        # Only an explicit registry manifest-not-found response permits creation.
        # Authentication, transport, throttling and malformed responses fail closed.
        absent = "manifest unknown" in message or f"{ref.lower()}: not found" in message
        if missing and absent and not any(word in message for word in
                                         ("unauthorized", "denied", "timeout", "forbidden")):
            return None
        raise


def digest(value):
    if not isinstance(value, str) or not DIGEST.fullmatch(value):
        raise ValueError(f"Invalid registry digest: {value!r}")
    return value


def image(image_name, ref, sha, version, arch, missing=False):
    manifest = inspect(ref, missing=missing)
    if manifest is None:
        return None
    if "manifests" in manifest:
        raise ValueError(f"Expected one native image at {ref}")
    value = digest(manifest["digest"])
    config = inspect(f"{image_name}@{value}", "Image")
    labels = config.get("config", {}).get("Labels", {})
    if (config.get("os"), config.get("architecture"),
        labels.get("org.opencontainers.image.revision"),
        labels.get("org.opencontainers.image.version")) != ("linux", arch, sha, version):
        raise ValueError(f"Image provenance mismatch at {ref}")
    return value


def stage(args):
    ref = f"{args.image}:sha-{args.sha}-{args.arch}"
    value = image(args.image, ref, args.sha, args.version, args.arch, missing=not args.required)
    output = f"exists={'true' if value else 'false'}\ndigest={value or ''}\n"
    if os.environ.get("GITHUB_OUTPUT"):
        with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as stream:
            stream.write(output)
    else:
        print(output, end="")


def manifest(args):
    expected = {}
    # Validate both captured immutable build digests before writing any version tag.
    for arch in ("amd64", "arm64"):
        value = digest(getattr(args, arch))
        image(args.image, f"{args.image}@{value}", args.sha, args.version, arch)
        expected[arch] = value
    current = {}
    for arch, value in expected.items():
        ref = f"{args.image}:{args.version}-{arch}"
        current[arch] = image(args.image, ref, args.sha, args.version, arch, missing=True)
        if current[arch] is not None and current[arch] != value:
            raise ValueError(f"Existing version image differs at {ref}")
    target = f"{args.image}:{args.version}"
    existing = inspect(target, missing=True)
    if existing is not None:
        descriptors = existing.get("manifests", [])
        actual = [(part.get("platform", {}).get("os"),
                   part.get("platform", {}).get("architecture"), part.get("digest"))
                  for part in descriptors]
        if sorted(actual) != sorted(("linux", arch, value) for arch, value in expected.items()):
            raise ValueError(f"Existing multi-arch manifest differs at {target}")
    for arch, value in expected.items():
        if current[arch] is None:
            run("docker", "buildx", "imagetools", "create", "--prefer-index=false",
                "--tag", f"{args.image}:{args.version}-{arch}", f"{args.image}@{value}")
    if existing is None:
        run("docker", "buildx", "imagetools", "create", "--tag", target,
            *(f"{args.image}@{value}" for value in expected.values()))


def asset_label(sha, tag, asset_digest):
    return f"m4b-publish-v1;sha={sha};tag={tag};sha256={asset_digest.removeprefix('sha256:')}"


def parse_asset_label(label):
    match = re.fullmatch(r"m4b-publish-v1;sha=([0-9a-f]{40});tag=(v[0-9]+\.[0-9]+\.[0-9]+);sha256=([0-9a-f]{64})", label)
    if match is None:
        raise ValueError(f"Invalid release asset provenance label: {label!r}")
    return match.groups()


def assets(args):
    if not re.fullmatch(r"[0-9a-f]{40}", args.sha):
        raise ValueError(f"Invalid source SHA: {args.sha!r}")
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", args.tag):
        raise ValueError(f"Invalid release tag: {args.tag!r}")
    release = json.loads(run("gh", "api", "--hostname", "github.com",
                             f"repos/{args.repo}/releases/tags/{args.tag}"))
    existing = {}
    for asset in release["assets"]:
        if asset["name"] in existing:
            raise ValueError(f"Duplicate release asset {asset['name']}")
        existing[asset["name"]] = asset
    pending = []
    # Validate all existing assets before uploading anything. On older releases
    # without GitHub's digest field, hash the actual downloaded asset bytes.
    for name in ("m4b-merge-linux", "m4b-merge-macos", "m4b-merge-windows.exe"):
        matches = list(Path(args.artifacts).glob(f"*/{name}"))
        if len(matches) != 1:
            raise ValueError(f"Expected exactly one built artifact {name}")
        path = matches[0]
        expected = "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()
        asset = existing.get(name)
        if asset is None:
            pending.append((path, asset_label(args.sha, args.tag, expected)))
            continue
        label = asset.get("label") or ""
        if label:
            bound_sha, bound_tag, bound_digest = parse_asset_label(label)
            if bound_sha != args.sha or bound_tag != args.tag:
                raise ValueError(f"Existing asset provenance differs from this publication: {name}")
            actual = asset.get("digest")
            if actual is None:
                data = run("gh", "api", "--hostname", "github.com", "-H",
                           "Accept: application/octet-stream",
                           f"repos/{args.repo}/releases/assets/{asset['id']}")
                actual = "sha256:" + hashlib.sha256(data).hexdigest()
            if actual != "sha256:" + bound_digest:
                raise ValueError(f"Existing asset digest differs from provenance label: {name}")
            continue
        actual = asset.get("digest")
        if actual is None:
            data = run("gh", "api", "--hostname", "github.com", "-H",
                       "Accept: application/octet-stream",
                       f"repos/{args.repo}/releases/assets/{asset['id']}")
            actual = "sha256:" + hashlib.sha256(data).hexdigest()
        if actual != expected:
            raise ValueError(f"Existing asset differs from tested build: {name}")
    for path, label in pending:
        run("gh", "release", "upload", args.tag, f"{path}#{label}", "--repo", args.repo)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for name in ("stage", "manifest"):
        command = commands.add_parser(name)
        command.add_argument("--image", required=True)
        command.add_argument("--sha", required=True)
        command.add_argument("--version", required=True)
        if name == "stage":
            command.add_argument("--arch", required=True, choices=("amd64", "arm64"))
            command.add_argument("--required", action="store_true")
        else:
            command.add_argument("--amd64", required=True)
            command.add_argument("--arm64", required=True)
    command = commands.add_parser("assets")
    command.add_argument("--repo", required=True)
    command.add_argument("--tag", required=True)
    command.add_argument("--sha", required=True)
    command.add_argument("--artifacts", required=True)
    args = parser.parse_args()
    {"stage": stage, "manifest": manifest, "assets": assets}[args.command](args)


if __name__ == "__main__":
    main()
