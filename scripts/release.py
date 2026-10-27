#!/usr/bin/env python3
"""Prepare source versions, promote frozen develop snapshots, and publish immutable tags.

Only prepare/candidate/tag mutate remotes. Policy and verify inspect git objects;
policy never checks out or runs code from a pull request.
"""
from __future__ import annotations

import argparse
from contextlib import contextmanager
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import tomllib


class ReleaseError(Exception):
    """An invariant or external command failed."""


VERSION = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\Z")
OID = re.compile(r"[0-9a-f]{40}\Z")
REPOSITORY = re.compile(r"(?:git@github\.com:|https://github\.com/)([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+?)(?:\.git)?/?\Z")


def semver(value):
    match = VERSION.fullmatch(value)
    if not match:
        raise ReleaseError(f"Invalid release version: {value!r}; expected X.Y.Z")
    return tuple(map(int, match.groups()))


def require(condition, message):
    if not condition:
        raise ReleaseError(message)


class Repository:
    def __init__(self, root="."):
        self.root = Path(root).resolve()

    def run(self, *args, cwd=None, ok=(0,)):
        result = subprocess.run(args, cwd=cwd or self.root, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if result.returncode not in ok:
            raise ReleaseError(f"{' '.join(args)}: {result.stderr.strip() or result.stdout.strip()}")
        return result.stdout.strip()

    def git(self, *args):
        return self.run("git", *args)

    def repository(self):
        url = self.git("remote", "get-url", "origin")
        match = REPOSITORY.fullmatch(url)
        require(match is not None, "origin must be a GitHub SSH or HTTPS repository")
        repository = match.group(1)
        push_url = self.git("remote", "get-url", "--push", "origin")
        push_match = REPOSITORY.fullmatch(push_url)
        require(push_match is not None and push_match.group(1).lower() == repository.lower(),
                "origin fetch and push must identify the same GitHub repository")
        return repository

    def api(self, endpoint, method="GET", fields=None):
        args = ["gh", "api", "--hostname", "github.com", endpoint]
        if method != "GET":
            args += ["--method", method]
        for key, value in (fields or {}).items():
            args += ["-f", f"{key}={value}"]
        value = self.run(*args)
        return json.loads(value) if value else None

    def oid(self, ref):
        require(not ref.startswith("-"), "Invalid git reference")
        value = self.git("rev-parse", "--verify", f"{ref}^{{commit}}")
        require(OID.fullmatch(value), "Expected a full commit object ID")
        return value

    def exact_oid(self, value):
        require(isinstance(value, str) and OID.fullmatch(value), "Expected a full commit object ID")
        require(self.oid(value) == value, "Commit object ID did not resolve exactly")
        return value

    def ancestor(self, older, newer):
        result = subprocess.run(["git", "merge-base", "--is-ancestor", older, newer],
                                cwd=self.root, capture_output=True, text=True)
        require(result.returncode in (0, 1), result.stderr.strip() or "Cannot inspect ancestry")
        return result.returncode == 0

    def fetch(self):
        self.repository()
        # Remote tags are authoritative, but local tags may be unpublished or
        # stale. Keep their objects in a private cache without touching refs/tags.
        self.git("fetch", "--prune", "--no-prune-tags", "--no-tags", "origin",
                 "+refs/heads/main:refs/remotes/origin/main",
                 "+refs/heads/develop:refs/remotes/origin/develop",
                 "+refs/tags/*:refs/m4b-release/tags/*")

    def blob(self, ref, filename):
        return self.git("show", f"{self.oid(ref)}:{filename}") + "\n"

    def package(self, ref):
        try:
            manifest = tomllib.loads(self.blob(ref, "Cargo.toml"))
            lock = tomllib.loads(self.blob(ref, "Cargo.lock"))
            package = manifest["package"]
            name, version = package["name"], package["version"]
            semver(version)
            matches = [p for p in lock["package"] if p["name"] == name and "source" not in p]
            require(len(matches) == 1 and matches[0]["version"] == version,
                    "Cargo.toml and root Cargo.lock package versions must agree")
            return name, version
        except (KeyError, TypeError, ValueError, tomllib.TOMLDecodeError) as error:
            raise ReleaseError(f"Invalid Cargo metadata at {ref}: {error}") from error

    def tags(self, ref):
        tags = []
        for tag in self.git("for-each-ref", f"--merged={ref}",
                            "--format=%(refname:strip=3)", "refs/m4b-release/tags/v*").splitlines():
            if VERSION.fullmatch(tag[1:]):
                tags.append((semver(tag[1:]), tag))
        return sorted(tags)

    def baseline(self, ref):
        tags = self.tags(ref)
        return tags[-1][0] if tags else (0, 0, 0)

    def tag_target(self, tag):
        ref = f"refs/m4b-release/tags/{tag}"
        result = subprocess.run(["git", "show-ref", "--verify", "--quiet", ref],
                                cwd=self.root, capture_output=True)
        require(result.returncode in (0, 1), "Cannot inspect release tag")
        return self.oid(ref) if result.returncode == 0 else None

    def fresh_version(self, ref, base):
        version = self.package(ref)[1]
        require(semver(version) > self.baseline(base),
                "Prepared version must be newer than every main-reachable release tag")
        require(self.tag_target(f"v{version}") is None, "Prepared version already has a tag")
        return version

    def parents(self, ref):
        return self.git("show", "-s", "--format=%P", ref).split()

    def tree(self, ref):
        return self.git("rev-parse", f"{ref}^{{tree}}")

    def verify(self, head, base, develop=None):
        head, base = self.exact_oid(head), self.exact_oid(base)
        require(base == self.oid("refs/remotes/origin/main"), "Stale main base; regenerate candidate")
        parents = self.parents(head)
        require(len(parents) == 2 and parents[0] == base,
                "Candidate must be a two-parent merge with current main as its first parent")
        source = parents[1]
        develop = self.exact_oid(develop) if develop else self.oid("refs/remotes/origin/develop")
        require(self.ancestor(source, develop), "Frozen source is not in develop history")
        require(not self.ancestor(source, base), "Frozen source was already promoted")
        require(self.tree(head) == self.tree(source), "Candidate tree must exactly equal frozen develop")
        version = self.fresh_version(source, base)
        require(self.package(head)[1] == version, "Candidate version differs from source")
        return version, source

    def live_ref(self, branch):
        repository = self.repository()
        value = self.api(f"repos/{repository}/git/ref/heads/{branch}")["object"]["sha"]
        require(OID.fullmatch(value), "GitHub returned an invalid branch object ID")
        return value

    def unchanged(self, base, source):
        require(self.live_ref("main") == base, "main advanced; regenerate candidate")
        live_develop = self.live_ref("develop")
        require(live_develop == source, "develop advanced while constructing candidate; retry")

    @contextmanager
    def worktree(self, ref):
        with tempfile.TemporaryDirectory(prefix="m4b-release-") as temporary:
            path = Path(temporary) / "checkout"
            self.git("worktree", "add", "--detach", str(path), ref)
            try:
                yield path
            finally:
                self.git("worktree", "remove", "--force", str(path))

    def remote_branch(self, branch):
        output = self.git("ls-remote", "--heads", "origin", f"refs/heads/{branch}")
        if not output:
            return None
        values = output.split()
        require(len(values) == 2 and values[1] == f"refs/heads/{branch}", "Ambiguous remote branch")
        self.git("fetch", "origin", f"refs/heads/{branch}")
        return self.exact_oid(values[0])

    def ensure_pr(self, branch, base, head, title, body):
        repository = self.repository()
        owner = repository.split("/")[0]
        require(self.git("ls-remote", "--heads", "origin", f"refs/heads/{branch}")
                == f"{head}\trefs/heads/{branch}", "Release branch changed before PR creation")
        endpoint = f"repos/{repository}/pulls?state=open&head={owner}:{branch}&base={base}"
        pulls = self.api(endpoint)
        require(len(pulls) <= 1, "More than one open release PR exists")
        if not pulls:
            self.run("gh", "pr", "create", "--repo", f"github.com/{repository}", "--head", branch,
                     "--base", base, "--title", title, "--body", body)
            pulls = self.api(endpoint)
        require(len(pulls) == 1, "Release PR was not found after creation")
        pull = pulls[0]
        require(pull["head"]["repo"]["full_name"].lower() == repository.lower()
                and pull["head"]["ref"] == branch and pull["head"]["sha"] == head
                and pull["base"]["ref"] == base, "Existing PR does not match release branch and head")
        return pull["html_url"]

    def publish_branch(self, branch, head, existing):
        require(re.fullmatch(r"release(?:-prep)?/v(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\."
                             r"(?:0|[1-9][0-9]*)-[0-9a-f]{12}(?:-[0-9a-f]{12})?", branch),
                "Only owned preparation or candidate refs may be created")
        if existing is None:
            # Absent-ref CAS: an empty expected old value can only create this
            # owned branch. It cannot update or delete any existing remote ref.
            self.git("push", "--force-with-lease=refs/heads/" + branch + ":", "origin",
                     f"{head}:refs/heads/{branch}")

    def candidate(self, resolved_commit=None, dry_run=False):
        self.fetch()
        base, source = self.oid("origin/main"), self.oid("origin/develop")
        version = self.fresh_version(source, base)
        branch = f"release/v{version}-{source[:12]}-{base[:12]}"
        existing = self.remote_branch(branch)
        if resolved_commit:
            head = self.exact_oid(resolved_commit)
        elif existing:
            head = existing
        else:
            with self.worktree(base) as checkout:
                self.run("git", "merge", "--no-ff", "--no-edit", source, cwd=checkout)
                head = self.run("git", "rev-parse", "HEAD", cwd=checkout)
        actual_version, actual_source = self.verify(head, base)
        require(actual_version == version and actual_source == source, "Candidate does not freeze current develop")
        if existing:
            require(self.parents(existing) == [base, source] and self.tree(existing) == self.tree(head),
                    "Existing candidate branch has unexpected content")
            # Preserve its original commit, whose timestamp may differ from a local merge.
            head = existing
        self.unchanged(base, source)
        url = None
        if not dry_run:
            self.publish_branch(branch, head, existing)
            self.unchanged(base, source)
            url = self.ensure_pr(branch, "main", head, f"chore(release): v{version}",
                                 f"Promote frozen develop `{source}` onto main `{base}`.\n\n"
                                 "Merge using a merge commit only. Keep native strict freshness; "
                                 "regenerate if main advances. No merge-back is needed.")
        return {"branch": branch, "version": f"v{version}", "head": head,
                "source": source, "base": base, "dry_run": dry_run, "pull_request": url,
                "pending_pr": "main" if dry_run else None}

    def prepare(self, bump="auto", version=None, dry_run=False):
        self.fetch()
        base, source = self.oid("origin/main"), self.oid("origin/develop")
        name, current = self.package(source)
        baseline = self.baseline(base)
        messages = self.git("log", "--format=%B%x00", f"{base}..{source}")
        require(bool(messages.strip()), "No unreleased develop commits")
        if version is None:
            if bump == "auto":
                if re.search(r"(?m)^[a-zA-Z][\w-]*(?:\([^\n)]*\))?!:|^BREAKING[ -]CHANGE:", messages):
                    bump = "major"
                elif re.search(r"(?m)^feat(?:\([^\n)]*\))?:", messages):
                    bump = "minor"
                else:
                    bump = "patch"
            major, minor, patch = baseline
            version = ".".join(map(str, {"major": (major + 1, 0, 0),
                                         "minor": (major, minor + 1, 0),
                                         "patch": (major, minor, patch + 1)}[bump]))
        require(semver(version) > baseline, "Requested version is not newer than released baseline")
        require(semver(version) >= semver(current), "Requested version would downgrade develop")
        require(self.tag_target(f"v{version}") is None, "Requested version already has a tag")
        branch = f"release-prep/v{version}-{source[:12]}"
        if current == version:
            self.unchanged(base, source)
            return {"version": f"v{version}", "source": source, "head": source, "branch": None,
                    "dry_run": dry_run, "pull_request": None, "pending_pr": None,
                    "message": "develop already contains the prepared version"}
        existing = self.remote_branch(branch)
        with self.worktree(source) as checkout:
            manifest = checkout / "Cargo.toml"
            lock = checkout / "Cargo.lock"
            manifest.write_text(update_manifest(manifest.read_text(), version))
            lock.write_text(update_lock(lock.read_text(), name, version))
            self.run("git", "add", "--", "Cargo.toml", "Cargo.lock", cwd=checkout)
            expected_tree = self.run("git", "write-tree", cwd=checkout)
            if existing:
                require(self.parents(existing) == [source] and self.tree(existing) == expected_tree,
                        "Existing preparation branch has unexpected content")
                head = existing
            else:
                self.run("git", "commit", "-m", f"chore(release): prepare v{version}", cwd=checkout)
                head = self.run("git", "rev-parse", "HEAD", cwd=checkout)
        require(self.package(head)[1] == version, "Preparation metadata does not agree")
        self.unchanged(base, source)
        url = None
        if not dry_run:
            self.publish_branch(branch, head, existing)
            self.unchanged(base, source)
            url = self.ensure_pr(branch, "develop", head, f"chore(release): prepare v{version}",
                                 "Update the root Cargo package and lockfile together. "
                                 "After this PR lands and CI passes, run `python3 scripts/release.py candidate`.")
        return {"branch": branch, "version": f"v{version}", "head": head,
                "source": source, "base": base, "dry_run": dry_run, "pull_request": url,
                "pending_pr": "develop" if dry_run else None}

    def check_publication(self, sha):
        sha = self.exact_oid(sha)
        main = self.oid("origin/main")
        require(self.ancestor(sha, main), "Publication SHA must belong to main history")
        version = self.package(sha)[1]
        target = self.tag_target(f"v{version}")
        if target:
            require(target == sha, "Existing version tag points at a different commit")
        else:
            require(semver(version) > self.baseline(main),
                    "Cannot publish an absent older or already superseded release")
        return f"v{version}"

    def tag(self, sha):
        self.fetch()
        tag = self.check_publication(sha)
        repository = self.repository()
        endpoint = f"repos/{repository}/git/refs"
        # GitHub creates refs atomically; it never overwrites an existing tag.
        try:
            self.api(endpoint, "POST", {"ref": f"refs/tags/{tag}", "sha": sha})
        except ReleaseError:
            target = self.api(f"repos/{repository}/git/ref/tags/{tag}")["object"]
            while target["type"] == "tag":
                target = self.api(f"repos/{repository}/git/tags/{target['sha']}")["object"]
            require(target["type"] == "commit" and target["sha"] == sha,
                    "Concurrent tag creation points at a different commit")
        return tag

    def policy(self, event, report_status=False):
        # Validate the status destination before using any event-controlled endpoint.
        repository = self.repository()
        require(event.get("repository", {}).get("full_name", "").lower() == repository.lower(),
                "Event repository does not match origin")
        pull = event.get("pull_request", {})
        head = pull.get("head", {}).get("sha", "")
        require(OID.fullmatch(head), "Event has no valid pull request head SHA")
        number = event.get("number")
        require(type(number) is int and number > 0, "Invalid pull request number")
        failure = None
        try:
            require(event.get("action") in {"opened", "reopened", "synchronize", "ready_for_review", "edited"},
                    "Unsupported or stale pull request event")
            live = self.api(f"repos/{repository}/pulls/{number}")
            self.validate_pull(pull, live, repository)
            self.fetch()
            self.git("fetch", "--no-tags", "origin", head)
            base = pull["base"]["sha"]
            require(self.live_ref("main") == base, "Event base is not current main")
            develop = self.live_ref("develop")
            version, source = self.verify(head, base, develop)
            expected_branch = f"release/v{version}-{source[:12]}-{base[:12]}"
            require(pull["head"]["ref"] == expected_branch, "Unexpected promotion branch name")
            # Re-read immediately before reporting, so a superseded event cannot claim green.
            self.validate_pull(pull, self.api(f"repos/{repository}/pulls/{number}"), repository)
            require(self.live_ref("main") == base, "main advanced during policy verification")
            require(self.ancestor(source, self.live_ref("develop")), "Source no longer belongs to develop")
        except (ReleaseError, OSError, KeyError, TypeError, ValueError) as error:
            failure = str(error)
        if report_status:
            self.api(f"repos/{repository}/statuses/{head}", "POST", {
                "state": "failure" if failure else "success", "context": "Release policy",
                "description": (failure or "Frozen promotion matches current main and prepared develop")[:140],
            })
        if failure:
            raise ReleaseError(failure)
        return {"head": head, "version": f"v{version}", "source": source}

    @staticmethod
    def validate_pull(event_pull, live, repository):
        require(live["state"] == "open", "Pull request is not open")
        for pull in (event_pull, live):
            require(pull["base"]["ref"] == "main", "Policy only accepts PRs into main")
            require(pull["head"]["repo"]["full_name"].lower() == repository.lower()
                    and pull["base"]["repo"]["full_name"].lower() == repository.lower(),
                    "Policy only accepts same-repository promotion PRs")
            require(pull["head"]["ref"].startswith("release/"), "Head is not a promotion branch")
        for key in ("head", "base"):
            require(event_pull[key]["sha"] == live[key]["sha"]
                    and event_pull[key]["ref"] == live[key]["ref"],
                    "Stale event: live PR head/base changed")


def replace_version(block, version):
    updated, count = re.subn(r'(?m)^(version\s*=\s*)"[^"\n]+"',
                             lambda match: match.group(1) + json.dumps(version), block)
    require(count == 1, "Expected one literal version in Cargo package table")
    return updated


def update_manifest(text, version):
    sections = re.split(r"(?m)(?=^\[)", text)
    matches = [i for i, section in enumerate(sections) if re.match(r"\[package\]\s*(?:#.*)?\n", section)]
    require(len(matches) == 1, "Expected one [package] table")
    index = matches[0]
    sections[index] = replace_version(sections[index], version)
    return "".join(sections)


def update_lock(text, name, version):
    sections = re.split(r"(?m)(?=^\[\[package\]\])", text)
    matches = []
    for index, section in enumerate(sections):
        if section.startswith("[[package]]"):
            package = tomllib.loads(section)["package"][0]
            if package["name"] == name and "source" not in package:
                matches.append(index)
    require(len(matches) == 1, "Expected one root package in Cargo.lock")
    index = matches[0]
    sections[index] = replace_version(sections[index], version)
    return "".join(sections)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    prepare = commands.add_parser("prepare")
    choice = prepare.add_mutually_exclusive_group()
    choice.add_argument("--bump", choices=("auto", "patch", "minor", "major"), default="auto")
    choice.add_argument("--version")
    prepare.add_argument("--dry-run", action="store_true")
    candidate = commands.add_parser("candidate")
    candidate.add_argument("--resolved-commit")
    candidate.add_argument("--dry-run", action="store_true")
    verify = commands.add_parser("verify")
    verify.add_argument("--head", required=True)
    verify.add_argument("--base", required=True)
    policy = commands.add_parser("policy")
    policy.add_argument("--event", type=Path, required=True)
    policy.add_argument("--report-status", action="store_true")
    version = commands.add_parser("version")
    version.add_argument("--ref", required=True)
    for command in ("check-publication", "tag"):
        commands.add_parser(command).add_argument("--sha", required=True)
    args = parser.parse_args(argv)
    repo = Repository()
    try:
        repo.repository()
        if args.command == "prepare":
            result = repo.prepare(args.bump, args.version, args.dry_run)
        elif args.command == "candidate":
            result = repo.candidate(args.resolved_commit, args.dry_run)
        elif args.command == "verify":
            repo.fetch()
            version, source = repo.verify(args.head, args.base)
            result = {"version": f"v{version}", "source": source, "head": args.head, "base": args.base}
        elif args.command == "policy":
            result = repo.policy(json.loads(args.event.read_text()), args.report_status)
        elif args.command == "version":
            result = "v" + repo.package(repo.exact_oid(args.ref))[1]
        elif args.command == "check-publication":
            repo.fetch()
            result = repo.check_publication(args.sha)
        else:
            result = repo.tag(args.sha)
        print(result if isinstance(result, str) else json.dumps(result, sort_keys=True))
        return 0
    except (ReleaseError, OSError, ValueError, KeyError, TypeError) as error:
        print(f"release: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
