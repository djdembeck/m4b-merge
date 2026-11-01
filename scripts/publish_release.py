"""Publish-only image and asset reconciliation; never replace existing outputs."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess


DIGEST = re.compile(r"sha256:[0-9a-f]{64}\Z")
# 40-hex commit SHA and vN.N.N release version; shared by extract(),
# assets(), and parse_asset_label() (anchor semantics via fullmatch).
SHA = re.compile(r"[0-9a-f]{40}")
VERSION = re.compile(r"v[0-9]+\.[0-9]+\.[0-9]+")
ASSET_LABEL = re.compile(
    r"m4b-publish-v1;sha=(" + SHA.pattern + r");"
    r"tag=(" + VERSION.pattern + r");sha256=([0-9a-f]{64})")


# Basename allowlist for --asset. Individual publication calls name their
# expected set explicitly; this is not a default release inventory.
ASSET_NAMES = ("m4b-merge-linux",
               "m4b-merge-linux-musl",
               "m4b-merge-linux-arm64",
               "m4b-merge-macos",
               "m4b-merge-windows.exe")


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


def extract(args):
    """Copy the exact /usr/local/bin/m4b-merge bytes out of a digest-pinned,
    provenance-verified image. All validation happens before the local
    output is created; the helper-owned container is always removed."""
    digest_value = digest(args.digest)
    if not SHA.fullmatch(args.sha):
        raise ValueError(f"Invalid source SHA: {args.sha!r}")
    if not VERSION.fullmatch(args.version):
        raise ValueError(f"Invalid release version: {args.version!r}")
    output = Path(args.output)
    if output.exists() or output.is_symlink():
        raise ValueError(f"Refusing to use existing output path: {output}")
    ref = f"{args.image}@{digest_value}"
    value = image(args.image, ref, args.sha, args.version, args.arch)
    if value != digest_value:
        raise ValueError(f"Image digest mismatch at {ref}")
    output.parent.mkdir(parents=True, exist_ok=True)
    # The id in creation stdout is a single word; strip the remaining
    # whitespace before embedding it in the docker cp source address.
    cid = run("docker", "create", "--entrypoint", "/usr/local/bin/m4b-merge", ref)
    cid = cid.decode().strip()
    try:
        run("docker", "cp", f"{cid}:/usr/local/bin/m4b-merge", str(output))
        if not output.is_file() or not os.access(output, os.X_OK):
            raise ValueError(f"Extracted binary is not a regular executable file: {output}")
    except BaseException:
        # Remove only the partial output this invocation started; a
        # pre-existing path was rejected above, and cleanup must not mask
        # the original failure.
        output.unlink(missing_ok=True)
        raise
    finally:
        subprocess.run(["docker", "rm", cid], check=False, capture_output=True)


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
    match = ASSET_LABEL.fullmatch(label)
    if match is None:
        raise ValueError(f"Invalid release asset provenance label: {label!r}")
    return match.groups()


def asset_digest(asset, repo):
    """Use the API-provided digest; hash the downloaded bytes for older
    assets that predate GitHub's digest field."""
    actual = asset.get("digest")
    if actual is None:
        data = run("gh", "api", "--hostname", "github.com", "-H",
                   "Accept: application/octet-stream",
                   f"repos/{repo}/releases/assets/{asset['id']}")
        actual = "sha256:" + hashlib.sha256(data).hexdigest()
    return actual


def release_listing(repo, tag):
    """Complete authenticated release listing; drafts included, errors propagate.

    GET releases/tags/{tag} is a published-release lookup only, so a draft's
    assets must be reconciled through the paginated listing instead."""
    records, page = [], 1
    while True:
        batch = json.loads(run("gh", "api", "--hostname", "github.com",
                               f"repos/{repo}/releases?per_page=100&page={page}"))
        if not isinstance(batch, list):
            raise ValueError(f"Unexpected release listing response on page {page}")
        records.extend(batch)
        if len(batch) < 100:
            break
        page += 1
    matches = [record for record in records if record.get("tag_name") == tag]
    if len(matches) > 1:
        raise ValueError(f"Multiple release records share tag {tag}; failing closed")
    if not matches:
        raise ValueError(f"No release record for tag {tag} after a complete listing; the draft is missing")
    return matches[0]


def artifacts(root, names):
    """Resolve an explicit, ordered asset inventory to exactly one regular,
    non-symlink file per name. Missing, duplicated, or unreadable matches
    fail before any upload; the directory layout is artifacts/<artifact>/
    <filename>, where only the filename row is matched."""
    paths = []
    for name in names:
        matches = sorted(Path(root).glob(f"*/{name}"))
        if len(matches) != 1:
            raise ValueError(f"Expected exactly one built artifact {name}")
        path = matches[0]
        if not path.is_file() or path.is_symlink():
            raise ValueError(f"Invalid built artifact {name}")
        paths.append(path)
    return paths


def assets(args):
    if not SHA.fullmatch(args.sha):
        raise ValueError(f"Invalid source SHA: {args.sha!r}")
    if not VERSION.fullmatch(args.tag):
        raise ValueError(f"Invalid release tag: {args.tag!r}")
    expected = list(getattr(args, "asset", None) or [])
    if not expected:
        raise ValueError("Expected at least one --asset name")
    if any(name not in ASSET_NAMES for name in expected):
        first = next(name for name in expected if name not in ASSET_NAMES)
        raise ValueError(f"Invalid asset name: {first}")
    if len(set(expected)) != len(expected):
        raise ValueError("Duplicate asset name in --asset list")
    release = release_listing(args.repo, args.tag)
    existing = {}
    for asset in release["assets"]:
        if asset["name"] in existing:
            raise ValueError(f"Duplicate release asset {asset['name']}")
        existing[asset["name"]] = asset
    pending = []
    # Validate all existing assets before uploading anything. On older releases
    # without GitHub's digest field, hash the actual downloaded asset bytes.
    for name, path in zip(expected, artifacts(args.artifacts, expected)):
        expected_digest = "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()
        asset = existing.get(name)
        if asset is None:
            pending.append((path, asset_label(args.sha, args.tag, expected_digest)))
            continue
        label = asset.get("label") or ""
        if label:
            bound_sha, bound_tag, bound_digest = parse_asset_label(label)
            if bound_sha != args.sha or bound_tag != args.tag:
                raise ValueError(f"Existing asset provenance differs from this publication: {name}")
            actual = asset_digest(asset, args.repo)
            if actual != "sha256:" + bound_digest:
                raise ValueError(f"Existing asset digest differs from provenance label: {name}")
            continue
        actual = asset_digest(asset, args.repo)
        if actual != expected_digest:
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
    command.add_argument("--asset", action="append", dest="asset",
                         choices=ASSET_NAMES, required=True)
    command = commands.add_parser("extract")
    command.add_argument("--image", required=True)
    command.add_argument("--digest", required=True)
    command.add_argument("--sha", required=True)
    command.add_argument("--version", required=True)
    command.add_argument("--arch", required=True, choices=("amd64", "arm64"))
    command.add_argument("--output", required=True)
    args = parser.parse_args()
    {"stage": stage, "manifest": manifest, "assets": assets,
     "extract": extract}[args.command](args)


if __name__ == "__main__":
    main()
