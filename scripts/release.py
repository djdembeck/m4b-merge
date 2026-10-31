#!/usr/bin/env python3
"""Reserve exact-source release identities and verify direct develop promotions."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys


class ReleaseError(Exception):
    pass


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


def isolated_git_environment():
    return {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}


class Repository:
    def __init__(self, root="."):
        self.root = Path(root).resolve()

    @staticmethod
    def proc(args, cwd):
        return subprocess.run(args, cwd=cwd, env=isolated_git_environment(), text=True,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE)

    def run(self, *args, ok=(0,)):
        result = self.proc(args, self.root)
        require(result.returncode in ok, f"{' '.join(args)}: {result.stderr.strip() or result.stdout.strip()}")
        return result.stdout.strip()

    def git(self, *args):
        return self.run("git", *args)

    def repository(self):
        match = REPOSITORY.fullmatch(self.git("remote", "get-url", "origin"))
        push = REPOSITORY.fullmatch(self.git("remote", "get-url", "--push", "origin"))
        require(match and push and match.group(1).lower() == push.group(1).lower(),
                "origin fetch/push must identify the same GitHub repository")
        return match.group(1)

    def api(self, endpoint, method="GET", fields=None):
        args = ["gh", "api", "--hostname", "github.com", endpoint]
        if method != "GET":
            args += ["--method", method]
        for key, value in (fields or {}).items():
            args += ["-f", f"{key}={value}"]
        value = self.run(*args)
        return json.loads(value) if value else None

    def oid(self, ref):
        require(isinstance(ref, str) and not ref.startswith("-"), "Invalid git reference")
        value = self.git("rev-parse", "--verify", f"{ref}^{{commit}}")
        require(OID.fullmatch(value), "Expected a full commit object ID")
        return value

    def exact_oid(self, value):
        require(isinstance(value, str) and OID.fullmatch(value), "Expected a full commit object ID")
        require(self.oid(value) == value, "Commit object ID did not resolve exactly")
        return value

    def ancestor(self, older, newer):
        result = self.proc(["git", "merge-base", "--is-ancestor", older, newer], self.root)
        require(result.returncode in (0, 1), result.stderr.strip() or "Cannot inspect ancestry")
        return result.returncode == 0

    def fetch(self):
        self.repository()
        self.git("fetch", "--prune", "--no-prune-tags", "--no-tags", "origin",
                 "+refs/heads/main:refs/remotes/origin/main",
                 "+refs/heads/develop:refs/remotes/origin/develop",
                 "+refs/tags/*:refs/m4b-release/tags/*")

    def tags(self, ref):
        tags = []
        for tag in self.git("for-each-ref", f"--merged={ref}", "--format=%(refname:strip=3)",
                            "refs/m4b-release/tags/v*").splitlines():
            if VERSION.fullmatch(tag[1:]):
                tags.append((semver(tag[1:]), tag))
        return sorted(tags)

    def tag_target(self, tag):
        ref = f"refs/m4b-release/tags/{tag}"
        result = self.proc(["git", "show-ref", "--verify", "--quiet", ref], self.root)
        require(result.returncode in (0, 1), "Cannot inspect release tag")
        return self.oid(ref) if result.returncode == 0 else None

    def api_tag_target(self, tag):
        """Resolve a remote tag to its commit, peeling annotated tag objects."""
        repository = self.repository()
        try:
            target = self.api(f"repos/{repository}/git/ref/tags/{tag}")["object"]
        except ReleaseError as error:
            require("HTTP 404" in str(error), str(error))
            return None
        for _ in range(10):
            if target["type"] != "tag":
                break
            target = self.api(f"repos/{repository}/git/tags/{target['sha']}")["object"]
        require(target["type"] == "commit", f"Tag {tag} does not resolve to a commit")
        return target["sha"]

    def select(self, sha, version=None, bump="auto", automatic=False):
        sha = self.exact_oid(sha)
        main = self.oid("origin/main")
        require(self.ancestor(sha, main), "Publication source must belong to main history")
        tags = self.tags(sha)
        exact = [tag for _, tag in tags if self.tag_target(tag) == sha]
        require(len(exact) <= 1, "Ambiguous exact-source release reservations")
        if version:
            version = "v" + version.removeprefix("v")
            semver(version[1:])
            require(not exact or exact == [version], "Source already reserved as a different version")
            target = self.tag_target(version)
            require(target is None or target == sha, "Version is reserved by another source")
            if target:
                return version
        elif exact:
            return exact[0]
        if automatic:
            for _, tag in self.tags(main):
                target = self.tag_target(tag)
                if target != sha and self.ancestor(sha, target):
                    return None  # Superseded automatic candidate; no reservation yet.
        baseline, previous = tags[-1] if tags else ((0, 0, 0), None)
        if version:
            require(semver(version[1:]) > baseline, "New version must exceed the reachable reservation baseline")
            return version
        if not previous:
            return "v0.1.0"
        if bump == "auto":
            messages = self.git("log", "--format=%B", f"refs/m4b-release/tags/{previous}..{sha}")
            if re.search(r"(?m)^BREAKING[ -]CHANGE:|^[a-zA-Z][\w-]*(?:\([^\n)]*\))?!:", messages):
                bump = "major"
            elif re.search(r"(?m)^feat(?:\([^\n)]*\))?:", messages):
                bump = "minor"
            else:
                bump = "patch"
        major, minor, patch = baseline
        require(bump in ("major", "minor", "patch"), "Invalid bump level")
        parts = {"major": (major + 1, 0, 0), "minor": (major, minor + 1, 0), "patch": (major, minor, patch + 1)}[bump]
        selected = "v" + ".".join(map(str, parts))
        require(self.tag_target(selected) in (None, sha), "Version is reserved by another source")
        return selected

    def reserve(self, sha, version=None, bump="auto", automatic=False):
        self.fetch()
        tag = self.select(sha, version, bump, automatic)
        if tag is None:
            return None
        repository = self.repository()
        try:
            self.api(f"repos/{repository}/git/refs", "POST", {"ref": f"refs/tags/{tag}", "sha": sha})
        except ReleaseError:
            # Create-only REST ref writes cannot move a concurrent reservation.
            target = self.api(f"repos/{repository}/git/ref/tags/{tag}")["object"]
            while target["type"] == "tag":
                target = self.api(f"repos/{repository}/git/tags/{target['sha']}")["object"]
            require(target["type"] == "commit" and target["sha"] == sha,
                    "Concurrent reservation points at a different source")
        return tag

    def release(self, tag, source, publish=False):
        semver(tag.removeprefix("v"))
        require(isinstance(source, str) and OID.fullmatch(source), "Expected a full commit object ID")
        repository = self.repository()
        require(self.api_tag_target(tag) == source, f"Tag {tag} does not point at the frozen source")
        try:
            existing = self.api(f"repos/{repository}/releases/tags/{tag}")
        except ReleaseError as error:
            require("HTTP 404" in str(error), str(error))
            require(not publish, "Cannot finalize a missing draft release")
            self.run("gh", "release", "create", tag, "--repo", repository, "--verify-tag", "--draft", "--title", f"Release {tag}", "--notes", "")
            return
        require(existing["tag_name"] == tag, "Release identity mismatch")
        if publish and existing.get("draft"):
            self.run("gh", "release", "edit", tag, "--repo", repository, "--draft=false", "--latest=false")

    def live_ref(self, branch):
        value = self.api(f"repos/{self.repository()}/git/ref/heads/{branch}")["object"]["sha"]
        require(OID.fullmatch(value), "GitHub returned an invalid branch SHA")
        return value

    def verify(self, head, base):
        head, base = self.exact_oid(head), self.exact_oid(base)
        require(head == self.oid("origin/develop"), "Promotion source must be the current develop head")
        require(base == self.oid("origin/main"), "Promotion base must be current main")
        require(not self.ancestor(head, base), "Develop source was already promoted")
        # Main-only source commits are forbidden. Prior merge-only promotions
        # may be absent from develop ancestry, but must introduce no unique code.
        unique = self.git("rev-list", "--no-merges", base, "--not", head)
        require(not unique, "Main contains independent source commits; forward-port them through develop")
        merge = self.proc(["git", "merge-tree", "--write-tree", base, head], self.root)
        require(merge.returncode == 0, "Develop promotion has merge conflicts")
        expected_tree = self.git("rev-parse", f"{head}^{{tree}}")
        require(merge.stdout.splitlines()[0] == expected_tree, "Promotion merge tree differs from develop source")
        return head

    def policy(self, event, report_status=False):
        repository = self.repository()
        require(event.get("repository", {}).get("full_name", "").lower() == repository.lower(), "Event repository differs from origin")
        pull = event.get("pull_request", {})
        head = pull.get("head", {}).get("sha", "")
        require(OID.fullmatch(head), "Event has no full PR head SHA")
        number = event.get("number")
        require(type(number) is int and number > 0, "Invalid pull request number")
        failure = None
        try:
            require(event.get("action") in {"opened", "reopened", "synchronize", "ready_for_review", "edited"}, "Unsupported PR event")
            live = self.api(f"repos/{repository}/pulls/{number}")
            for value in (pull, live):
                require(value["base"]["ref"] == "main" and value["head"]["ref"] == "develop", "Only direct develop to main promotions are accepted")
                require(value["head"]["repo"]["full_name"].lower() == repository.lower() and value["base"]["repo"]["full_name"].lower() == repository.lower(), "Promotion must be same-repository")
            require(live["state"] == "open" and live["head"]["sha"] == head, "PR is closed or its head changed")
            self.fetch()
            base = self.oid("origin/main")
            self.verify(head, base)
            closing = self.api(f"repos/{repository}/pulls/{number}")
            require(closing["state"] == "open" and closing["head"] == live["head"] and closing["base"]["ref"] == "main", "PR changed during verification")
            require(self.live_ref("main") == base and self.live_ref("develop") == head, "Promotion refs changed during verification")
        except (ReleaseError, OSError, KeyError, TypeError, ValueError) as error:
            failure = str(error)
        if report_status:
            self.api(f"repos/{repository}/statuses/{head}", "POST", {"state": "failure" if failure else "success", "context": "Promotion guard", "description": (failure or "Direct develop promotion has the expected source and merge tree")[:140]})
        if failure:
            raise ReleaseError(failure)
        return {"head": head, "base": base}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for command in ("select", "reserve"):
        action = commands.add_parser(command)
        action.add_argument("--sha", required=True)
        action.add_argument("--version")
        action.add_argument("--bump", choices=("auto", "patch", "minor", "major"), default="auto")
        action.add_argument("--automatic", action="store_true")
    finish = commands.add_parser("release")
    finish.add_argument("--tag", required=True)
    finish.add_argument("--source", required=True)
    finish.add_argument("--publish", action="store_true")
    guard = commands.add_parser("policy")
    guard.add_argument("--event", type=Path, required=True)
    guard.add_argument("--report-status", action="store_true")
    args = parser.parse_args(argv)
    repo = Repository()
    try:
        repo.repository()
        if args.command == "release":
            repo.release(args.tag, args.source, args.publish)
        elif args.command == "policy":
            output = repo.policy(json.loads(args.event.read_text()), args.report_status)
            print(json.dumps(output, sort_keys=True))
        else:
            if args.command == "reserve":
                output = repo.reserve(args.sha, args.version, args.bump, args.automatic)
            else:
                repo.fetch()
                output = repo.select(args.sha, args.version, args.bump, args.automatic)
            print(output or "")
        return 0
    except (ReleaseError, OSError, ValueError, KeyError, TypeError) as error:
        print(f"release: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
