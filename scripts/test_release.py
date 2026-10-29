#!/usr/bin/env python3
"""Behavior tests: real temporary git histories with an in-process GitHub adapter.

Run with: python3 -m unittest discover -s scripts -p 'test_release.py'
No network calls, production refs, or production credentials are used.
"""
import contextlib
import copy
import io
import json
import os
import re
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import release


class LocalGitHub(release.Repository):
    """Replace only the GitHub transport; all release git operations are real."""

    def __init__(self, root, remote):
        super().__init__(root)
        self.remote = remote
        self.pulls = []
        self.statuses = []
        self.reject_status = False
        self.on_pull_read = None
        self.pull_reads = 0

    def repository(self):
        return "example/m4b-merge"

    def remote_git(self, *args):
        return self.run("git", "--git-dir", str(self.remote), *args)

    def api(self, endpoint, method="GET", fields=None):
        prefix = "repos/example/m4b-merge/"
        if not endpoint.startswith(prefix):
            raise AssertionError(endpoint)
        endpoint = endpoint[len(prefix):]
        if endpoint.startswith("git/ref/heads/"):
            branch = endpoint.removeprefix("git/ref/heads/")
            return {"object": {"sha": self.remote_git("rev-parse", f"refs/heads/{branch}")}}
        if endpoint.startswith("pulls?"):
            from urllib.parse import parse_qs
            query = parse_qs(endpoint.split("?", 1)[1])
            branch = query["head"][0].split(":", 1)[1]
            return [copy.deepcopy(p) for p in self.pulls if p["state"] == "open"
                    and p["head"]["ref"] == branch and p["base"]["ref"] == query["base"][0]]
        if endpoint.startswith("pulls/"):
            self.pull_reads += 1
            if self.on_pull_read:
                self.on_pull_read(self.pull_reads)
            number = int(endpoint.split("/")[1])
            return copy.deepcopy(self.pulls[number - 1])
        if endpoint.startswith("statuses/"):
            if self.reject_status:
                raise release.ReleaseError("Status API denied")
            self.statuses.append((endpoint.split("/")[1], fields))
            return fields
        if endpoint == "git/refs" and method == "POST":
            self.remote_git("update-ref", fields["ref"], fields["sha"], "0" * 40)
            return {"ref": fields["ref"]}
        if endpoint.startswith("git/ref/tags/"):
            tag = endpoint.removeprefix("git/ref/tags/")
            return {"object": {"type": "commit", "sha": self.remote_git("rev-parse", f"refs/tags/{tag}^{{commit}}")}}
        raise AssertionError((endpoint, method, fields))

    def run(self, *args, **kwargs):
        if args[:3] == ("gh", "pr", "create"):
            def option(name):
                return args[args.index(name) + 1]
            branch, base = option("--head"), option("--base")
            number = len(self.pulls) + 1
            pull = self.pull(branch, base, number)
            self.pulls.append(pull)
            return pull["html_url"]
        return super().run(*args, **kwargs)

    def pull(self, branch, base="main", number=1):
        repository = {"full_name": self.repository()}
        return {"number": number, "state": "open", "html_url": f"https://github.com/{self.repository()}/pull/{number}",
                "head": {"ref": branch, "sha": self.remote_git("rev-parse", f"refs/heads/{branch}"), "repo": repository},
                "base": {"ref": base, "sha": self.remote_git("rev-parse", f"refs/heads/{base}"), "repo": repository}}


class ReleaseBehavior(unittest.TestCase):
    def setUp(self):
        clean_env = {key: value for key, value in os.environ.items()
                     if not key.startswith("GIT_")}
        environment = patch.dict(os.environ, clean_env, clear=True)
        environment.start()
        # Registered first so the original environment is restored only after
        # the temporary repositories have been cleaned up.
        self.addCleanup(environment.stop)
        self.temp = tempfile.TemporaryDirectory(prefix="release-test-")
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.remote = root / "remote.git"
        self.checkout = root / "checkout"
        self.command(root, "git", "init", "--bare", str(self.remote))
        self.command(root, "git", "init", "-b", "main", str(self.checkout))
        self.command(self.checkout, "git", "config", "user.name", "Release Test")
        self.command(self.checkout, "git", "config", "user.email", "test@example.invalid")
        self.command(self.checkout, "git", "config", "commit.gpgsign", "false")
        self.command(self.checkout, "git", "config", "tag.gpgsign", "false")
        self.command(self.checkout, "git", "remote", "add", "origin", str(self.remote))
        (self.checkout / "Cargo.toml").write_text(
            '[package]\nname = "m4b-merge"\nversion = "1.0.1"\nedition = "2024"\n\n'
            '[dependencies]\nother = { version = "1.0.1" }\n')
        (self.checkout / "Cargo.lock").write_text(
            '# This file is automatically @generated by Cargo.\nversion = 4\n\n'
            '[[package]]\nname = "m4b-merge"\nversion = "1.0.1"\ndependencies = ["other"]\n\n'
            '[[package]]\nname = "other"\nversion = "1.0.1"\nsource = "registry+https://example.invalid"\n')
        self.commit("initial release")
        self.git("tag", "v1.0.1")
        self.git("push", "origin", "main", "--tags")
        self.initial_main = self.git("rev-parse", "HEAD")
        self.git("switch", "-c", "develop")
        self.add_source("feat: first feature", "one")
        self.repo = LocalGitHub(self.checkout, self.remote)
        self.repo.fetch()

    def test_fixture_isolates_inherited_git_environment_in_subprocess(self):
        guard = Path(self.temp.name) / "protected"
        self.command(guard.parent, "git", "init", str(guard))

        def snapshot(root):
            return {path.relative_to(root): path.read_bytes()
                    for path in root.rglob("*") if path.is_file()}

        before = snapshot(guard)
        env = os.environ.copy()
        env.update({
            "GIT_DIR": str(guard / ".git"),
            "GIT_WORK_TREE": str(guard),
            "GIT_COMMON_DIR": str(guard / ".git"),
            "GIT_INDEX_FILE": str(guard / ".git" / "index"),
            "GIT_CONFIG_COUNT": "1",
            "GIT_CONFIG_KEY_0": "core.bare",
            "GIT_CONFIG_VALUE_0": "true",
        })
        result = subprocess.run(
            ["python3", "-m", "unittest",
             "test_release.ReleaseBehavior.test_preparation_preserves_local_tags_and_uses_remote_versions"],
            cwd=Path(__file__).parent, env=env, text=True, capture_output=True)

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(snapshot(guard), before)

    @staticmethod
    def command(cwd, *args):
        result = subprocess.run(args, cwd=cwd, text=True, capture_output=True)
        if result.returncode:
            raise AssertionError(f"{args}: {result.stderr}\n{result.stdout}")
        return result.stdout.strip()

    def git(self, *args):
        return self.command(self.checkout, "git", *args)

    def commit(self, message):
        self.git("add", ".")
        self.git("commit", "-m", message)
        return self.git("rev-parse", "HEAD")

    def add_source(self, message, content):
        (self.checkout / "feature.txt").write_text(content + "\n")
        sha = self.commit(message)
        self.git("push", "origin", "develop")
        return sha

    def advance_remote(self, branch, message, content):
        # Advance a remote branch from a scratch clone, so the new tip is an
        # object the fixture checkout has not fetched.
        scratch = Path(self.temp.name) / f"scratch-{branch}"
        self.command(self.checkout, "git", "clone", str(self.remote), str(scratch))
        self.command(scratch, "git", "checkout", "-q", "-B", branch,
                     self.repo.remote_git("rev-parse", f"refs/heads/{branch}"))
        self.command(scratch, "git", "config", "user.name", "Remote Test")
        self.command(scratch, "git", "config", "user.email", "late@example.invalid")
        (scratch / "late.txt").write_text(content + "\n")
        self.command(scratch, "git", "add", "late.txt")
        self.command(scratch, "git", "commit", "-q", "-m", message)
        self.command(scratch, "git", "push", "origin", f"HEAD:refs/heads/{branch}")
        return self.repo.remote_git("rev-parse", f"refs/heads/{branch}")

    def push_commit_to(self, branch, message, content, suffix=""):
        # Advance a remote branch from a scratch clone, so the new tip is an
        # object the fixture checkout has not fetched.
        sequence = getattr(self, "_scratch_seq", 0) + 1
        self._scratch_seq = sequence
        scratch = Path(self.temp.name) / (f"scratch-{sequence}-"
            f"{branch.replace('/', '_')}" + (f"-{suffix}" if suffix else ""))
        self.command(self.checkout.parent, "git", "clone", "-q", str(self.remote), str(scratch))
        self.command(scratch, "git", "checkout", "-q", "-B", branch,
                     self.repo.remote_git("rev-parse", f"refs/heads/{branch}"))
        self.command(scratch, "git", "config", "user.name", "Remote Test")
        self.command(scratch, "git", "config", "user.email", "late@example.invalid")
        (scratch / "late.txt").write_text(content + "\n")
        self.command(scratch, "git", "add", "late.txt")
        self.command(scratch, "git", "commit", "-q", "-m", message)
        self.command(scratch, "git", "push", "origin", f"HEAD:refs/heads/{branch}")
        return self.repo.remote_git("rev-parse", f"refs/heads/{branch}")

    def land_preparation(self, **kwargs):
        result = self.repo.prepare(**kwargs)
        self.git("merge", "--ff-only", result["head"])
        self.git("push", "origin", "develop")
        for pull in self.repo.pulls:
            if pull["head"]["ref"] == result["branch"]:
                pull["state"] = "closed"
        return result

    def land_candidate(self, candidate):
        # GitHub's merge-commit method creates a final tested main merge SHA,
        # not a version bump or a merge-back into develop.
        with self.repo.worktree("origin/main") as path:
            self.repo.run("git", "merge", "--no-ff", "--no-edit", candidate["head"], cwd=path)
            final = self.repo.run("git", "rev-parse", "HEAD", cwd=path)
            self.repo.run("git", "push", "origin", f"{final}:refs/heads/main", cwd=path)
        self.repo.fetch()
        for pull in self.repo.pulls:
            if pull["head"]["ref"] == candidate["branch"]:
                pull["state"] = "closed"
        return final

    def event_for(self, candidate):
        number = next(p["number"] for p in self.repo.pulls if p["head"]["ref"] == candidate["branch"])
        return {"action": "opened", "number": number,
                "repository": {"full_name": self.repo.repository()},
                "pull_request": copy.deepcopy(self.repo.pulls[number - 1])}

    def test_preparation_preserves_local_tags_and_uses_remote_versions(self):
        self.git("tag", "-f", "v1.0.1", "develop")
        self.git("tag", "v99.0.0", "develop")
        local_tags = self.git("show-ref", "--tags")

        prepared = self.repo.prepare(dry_run=True)

        self.assertEqual(prepared["version"], "v1.1.0")
        self.assertEqual(self.git("show-ref", "--tags"), local_tags)
        self.assertEqual(self.repo.tag_target("v1.0.1"), self.initial_main)
        self.assertIsNone(self.repo.tag_target("v99.0.0"))
        self.assertEqual(self.repo.remote_git("rev-parse", "main"), self.initial_main)

    def test_two_promotions_without_main_to_develop_sync(self):
        first_prep = self.land_preparation()
        self.assertEqual(first_prep["version"], "v1.1.0")
        first = self.repo.candidate()
        self.assertEqual(self.repo.parents(first["head"]), [self.initial_main, first["source"]])
        self.assertEqual(self.repo.tree(first["head"]), self.repo.tree(first["source"]))
        first_main = self.land_candidate(first)
        self.assertNotEqual(first_main, first["head"])
        self.assertEqual(self.repo.tag(first_main), "v1.1.0")
        self.assertEqual(self.repo.tag(first_main), "v1.1.0")
        self.assertEqual(self.repo.check_publication(first_main), "v1.1.0")
        self.assertFalse(self.repo.ancestor(first_main, self.git("rev-parse", "develop")))
        self.add_source("fix: second feature", "two")
        second_prep = self.land_preparation()
        self.assertEqual(second_prep["version"], "v1.1.1")
        second = self.repo.candidate()
        self.assertEqual(second["base"], first_main)
        self.assertEqual(self.repo.tree(second["head"]), self.repo.tree(second["source"]))
        self.assertFalse(self.repo.ancestor(first_main, second["source"]))
        second_main = self.land_candidate(second)
        self.assertEqual(self.repo.tag(second_main), "v1.1.1")
        self.assertEqual(self.repo.tag(second_main), "v1.1.1")
        self.assertEqual(self.repo.remote_git("rev-parse", "v1.1.1"), second_main)
        self.assertEqual(self.repo.remote_git("rev-parse", "develop"), second["source"])

    def test_dry_run_builds_real_dag_and_never_pushes(self):
        before = self.repo.remote_git("show-ref")
        prepare = self.repo.prepare(dry_run=True)
        self.assertEqual(prepare["pending_pr"], "develop")
        self.assertEqual(self.repo.package(prepare["head"])[1], "1.1.0")
        self.assertEqual(before, self.repo.remote_git("show-ref"))
        self.assertEqual(self.repo.pulls, [])
        self.git("merge", "--ff-only", prepare["head"])
        self.git("push", "origin", "develop")
        before = self.repo.remote_git("show-ref")
        candidate = self.repo.candidate(dry_run=True)
        self.assertEqual(candidate["pending_pr"], "main")
        self.assertEqual(self.repo.parents(candidate["head"]), [candidate["base"], candidate["source"]])
        self.assertEqual(before, self.repo.remote_git("show-ref"))
        self.assertEqual(self.repo.pulls, [])

    def test_prepare_only_changes_root_versions_and_reuses_identical_pr(self):
        before = self.git("rev-parse", "develop")
        prepared = self.repo.prepare()
        repeated = self.repo.prepare()
        self.assertEqual(prepared, repeated)
        self.assertEqual(len(self.repo.pulls), 1)
        self.assertEqual(self.git("rev-parse", "develop"), before)
        self.assertEqual(self.repo.parents(prepared["head"]), [before])
        changed = self.git("diff", "--name-only", before, prepared["head"]).splitlines()
        self.assertEqual(changed, ["Cargo.lock", "Cargo.toml"])
        manifest = self.repo.blob(prepared["head"], "Cargo.toml")
        lock = self.repo.blob(prepared["head"], "Cargo.lock")
        self.assertIn('other = { version = "1.0.1" }', manifest)
        self.assertIn('name = "other"\nversion = "1.0.1"', lock)
        self.assertEqual(self.git("show", "-s", "--format=%an <%ae>", prepared["head"]),
                         "Release Test <test@example.invalid>")

    def test_auto_major_and_explicit_validation(self):
        self.add_source("feat!: incompatible interface\n\nBREAKING CHANGE: removed old option", "breaking")
        self.assertEqual(self.repo.prepare(dry_run=True)["version"], "v2.0.0")
        self.assertEqual(self.repo.prepare(bump="patch", dry_run=True)["version"], "v1.0.2")
        self.assertEqual(self.repo.prepare(version="3.4.5", dry_run=True)["version"], "v3.4.5")
        for version in ("1.0.1", "v2.0.0", "1.02.0", "2.0.0-rc.1", "$(touch nope)"):
            with self.subTest(version=version), self.assertRaises(release.ReleaseError):
                self.repo.prepare(version=version, dry_run=True)

    def test_manifest_lock_drift_is_rejected(self):
        lock = self.checkout / "Cargo.lock"
        lock.write_text(lock.read_text().replace('name = "m4b-merge"\nversion = "1.0.1"',
                                                'name = "m4b-merge"\nversion = "9.0.0"'))
        self.commit("break lock version")
        self.git("push", "origin", "develop")
        with self.assertRaisesRegex(release.ReleaseError, "versions must agree"):
            self.repo.prepare(dry_run=True)

    def test_candidate_reuses_pr_and_accepts_frozen_source_after_develop_advances(self):
        self.land_preparation()
        candidate = self.repo.candidate()
        self.assertEqual(candidate, self.repo.candidate())
        self.add_source("fix: next cycle", "next")
        self.repo.fetch()
        version, source = self.repo.verify(candidate["head"], candidate["base"])
        self.assertEqual(version, "1.1.0")
        self.assertEqual(source, candidate["source"])
        event = self.event_for(candidate)
        self.repo.policy(event, report_status=True)
        self.assertEqual(self.repo.statuses[-1][1]["state"], "success")

    def test_inherited_git_environment_cannot_retarget_commands(self):
        foreign = Path(self.temp.name) / "foreign"
        self.command(foreign.parent, "git", "init", str(foreign))
        self.command(foreign, "git", "config", "user.name", "Release Test")
        self.command(foreign, "git", "config", "user.email", "test@example.invalid")
        (foreign / "foreign.txt").write_text("must stay untouched\n")
        self.command(foreign, "git", "add", "foreign.txt")
        self.command(foreign, "git", "commit", "-q", "-m", "foreign")
        foreign_head = self.command(foreign, "git", "rev-parse", "HEAD")

        def assert_clean_results():
            self.assertEqual(self.repo.oid("main"), self.initial_main)
            self.assertTrue(self.repo.ancestor(self.initial_main, self.repo.remote_git("rev-parse", "refs/heads/develop")))
            self.assertEqual(self.repo.remote_git("rev-parse", "main"), self.initial_main)
            self.assertIsNotNone(self.repo.tag_target("v1.0.1"))
            self.assertEqual(self.repo.prepare(dry_run=True)["version"], "v1.1.0")

        with patch.dict(os.environ, {"GIT_DIR": str(foreign / ".git"),
                                     "GIT_WORK_TREE": str(foreign)}, clear=False):
            assert_clean_results()
        # The foreign target is a bare repo; assert its ref set through
        # --git-dir for-each-ref so a retargeted ref push is visible (a
        # non-bare rev-parse HEAD cannot detect one).
        foreign_bare = Path(self.temp.name) / "foreign-bare.git"
        self.command(foreign.parent, "git", "clone", "--bare", "-q", str(foreign), str(foreign_bare))

        def foreign_refs():
            return self.command(foreign_bare, "git", "--git-dir", str(foreign_bare), "for-each-ref")

        def assert_exercised_operations_leave_foreign_untouched():
            before = foreign_refs()
            self.assertEqual(self.repo.oid("main"), self.initial_main)
            self.assertTrue(self.repo.ancestor(self.initial_main,
                                               self.repo.remote_git("rev-parse", "refs/heads/develop")))
            self.assertEqual(self.repo.remote_git("rev-parse", "main"), self.initial_main)
            self.assertIsNotNone(self.repo.tag_target("v1.0.1"))
            self.assertEqual(self.repo.prepare(dry_run=True)["version"], "v1.1.0")
            self.assertEqual(foreign_refs(), before)

        # url.insteadOf rewrites the fixture remote URL for every command that
        # resolves it: oid, ancestor, tag_target and prepare all exercise it.
        with patch.dict(os.environ, {"GIT_CONFIG_COUNT": "1",
                                     "GIT_CONFIG_KEY_0": f"url.{foreign_bare}.insteadOf",
                                     "GIT_CONFIG_VALUE_0": str(self.remote)}, clear=False):
            assert_exercised_operations_leave_foreign_untouched()
        self.assertEqual(self.command(foreign, "git", "rev-parse", "HEAD"), foreign_head)

    def test_isolated_git_environment_keeps_transport_ambient_variables(self):
        # Ambient transport variables (GH_TOKEN, HOME, PATH) must survive
        # isolation while GIT_* does not, and no non-GIT_ variable may be
        # dropped: isolating to a subset of the ambient environment would
        # silently strip variables other release machinery depends on.
        with patch.dict(os.environ, {"GH_TOKEN": "ci-token", "HOME": "/home/ci",
                                     "PATH": "/usr/bin", "GIT_DIR": "/dev/null",
                                     "GIT_CONFIG_COUNT": "1",
                                     "GIT_CONFIG_KEY_0": "core.bare",
                                     "GIT_CONFIG_VALUE_0": "true"}, clear=True):
            expected = {key: value for key, value in os.environ.items()
                        if not key.startswith("GIT_")}
            self.assertEqual(release.isolated_git_environment(), expected)
            self.assertEqual(release.isolated_git_environment()["GH_TOKEN"], "ci-token")
            self.assertEqual(release.isolated_git_environment()["HOME"], "/home/ci")
            self.assertEqual(release.isolated_git_environment()["PATH"], "/usr/bin")
        # A hostile inherited GIT_DIR (the bare remote) must not retarget the
        # repository's git commands, and the in-process API transport must
        # still succeed.
        with patch.dict(os.environ, {"GIT_DIR": str(self.remote),
                                     "GIT_CONFIG_COUNT": "1",
                                     "GIT_CONFIG_KEY_0": "core.bare",
                                     "GIT_CONFIG_VALUE_0": "true"}, clear=False):
            self.assertEqual(self.repo.git("rev-parse", "--is-bare-repository"), "false")
            self.assertEqual(self.repo.live_ref("main"), self.initial_main)
            self.assertEqual(self.repo.live_ref("develop"),
                             self.repo.remote_git("rev-parse", "refs/heads/develop"))

    def test_stale_main_and_unforwarded_hotfix_fail_closed(self):
        self.land_preparation()
        candidate = self.repo.candidate()
        with self.repo.worktree("origin/main") as path:
            (path / "hotfix.txt").write_text("must preserve\n")
            self.repo.run("git", "add", ".", cwd=path)
            self.repo.run("git", "commit", "-m", "fix: main hotfix", cwd=path)
            self.repo.run("git", "push", "origin", "HEAD:refs/heads/main", cwd=path)
        self.repo.fetch()
        with self.assertRaisesRegex(release.ReleaseError, "Stale main"):
            self.repo.verify(candidate["head"], candidate["base"])
        with self.assertRaisesRegex(release.ReleaseError, "tree must exactly equal"):
            self.repo.candidate(dry_run=True)
        event = self.event_for(candidate)
        with self.assertRaises(release.ReleaseError):
            self.repo.policy(event, report_status=True)
        self.assertEqual(self.repo.statuses[-1][1]["state"], "failure")

    def test_conflict_requires_human_resolution_and_exact_tree_and_parents(self):
        # Both branches change the same file differently; candidate cannot choose a side.
        with self.repo.worktree("origin/main") as path:
            (path / "feature.txt").write_text("main-only\n")
            self.repo.run("git", "add", ".", cwd=path)
            self.repo.run("git", "commit", "-m", "fix: historical conflict", cwd=path)
            hotfix = self.repo.run("git", "rev-parse", "HEAD", cwd=path)
            self.repo.run("git", "push", "origin", "HEAD:refs/heads/main", cwd=path)
        self.land_preparation()
        with self.assertRaises(release.ReleaseError):
            self.repo.candidate(dry_run=True)
        # Reconcile intended main content into develop first, still without merging main.
        (self.checkout / "feature.txt").write_text("main-only plus source feature\n")
        self.commit("fix: forward port intended hotfix")
        self.git("push", "origin", "develop")
        self.repo.fetch()
        source = self.git("rev-parse", "develop")
        tree = self.repo.tree(source)
        with self.repo.worktree(hotfix) as path:
            with self.assertRaises(release.ReleaseError):
                self.repo.run("git", "merge", "--no-ff", "--no-edit", source, cwd=path)
            (path / "feature.txt").write_text("main-only plus source feature\n")
            self.repo.run("git", "add", "feature.txt", cwd=path)
            self.repo.run("git", "commit", "-m", "Human resolved normal merge", cwd=path)
            resolved = self.repo.run("git", "rev-parse", "HEAD", cwd=path)
        candidate = self.repo.candidate(resolved_commit=resolved, dry_run=True)
        self.assertEqual(candidate["head"], resolved)
        wrong_parents = self.git("commit-tree", tree, "-p", source, "-p", hotfix, "-m", "Wrong order")
        with self.assertRaisesRegex(release.ReleaseError, "two-parent merge"):
            self.repo.candidate(resolved_commit=wrong_parents, dry_run=True)
        wrong_tree = self.git("commit-tree", self.repo.tree(hotfix), "-p", hotfix, "-p", source, "-m", "Wrong tree")
        with self.assertRaisesRegex(release.ReleaseError, "tree must exactly equal"):
            self.repo.candidate(resolved_commit=wrong_tree, dry_run=True)
        with self.assertRaisesRegex(release.ReleaseError, "two-parent merge"):
            self.repo.candidate(resolved_commit=source, dry_run=True)

    def test_unexpected_existing_preparation_branch_is_never_overwritten(self):
        source = self.git("rev-parse", "develop")
        branch = f"release-prep/v1.1.0-{source[:12]}"
        self.git("push", "origin", f"{source}:refs/heads/{branch}")
        with self.assertRaisesRegex(release.ReleaseError, "unexpected content"):
            self.repo.prepare()
        self.assertEqual(self.repo.remote_git("rev-parse", branch), source)
        self.assertEqual(self.repo.pulls, [])

    def test_publication_requires_main_and_immutable_matching_tag(self):
        self.land_preparation()
        candidate = self.repo.candidate()
        with self.assertRaisesRegex(release.ReleaseError, "main history"):
            self.repo.check_publication(candidate["head"])
        final = self.land_candidate(candidate)
        self.git("tag", "v1.1.0", self.initial_main)
        self.git("push", "origin", "v1.1.0")
        self.repo.fetch()
        with self.assertRaisesRegex(release.ReleaseError, "different commit"):
            self.repo.check_publication(final)
        with self.assertRaisesRegex(release.ReleaseError, "different commit"):
            self.repo.tag(final)
        self.assertEqual(self.repo.remote_git("rev-parse", "v1.1.0"), self.initial_main)

    def test_absent_older_release_cannot_be_created(self):
        self.land_preparation()
        final = self.land_candidate(self.repo.candidate())
        self.git("tag", "v9.0.0", self.initial_main)
        self.git("push", "origin", "v9.0.0")
        self.repo.fetch()
        with self.assertRaisesRegex(release.ReleaseError, "superseded"):
            self.repo.check_publication(final)

    def test_policy_is_read_only_and_reports_literal_status(self):
        self.land_preparation()
        candidate = self.repo.candidate()
        event = self.event_for(candidate)
        before_refs = self.git("show-ref")
        before_worktree = self.git("status", "--porcelain")
        result = self.repo.policy(event, report_status=True)
        self.assertEqual(result["head"], candidate["head"])
        self.assertEqual(self.repo.statuses[-1][0], candidate["head"])
        self.assertEqual(self.repo.statuses[-1][1]["context"], "Release policy")
        self.assertEqual(self.repo.statuses[-1][1]["state"], "success")
        self.assertEqual(before_refs, self.git("show-ref"))
        self.assertEqual(before_worktree, self.git("status", "--porcelain"))
        self.repo.reject_status = True
        with self.assertRaisesRegex(release.ReleaseError, "Status API denied"):
            self.repo.policy(event, report_status=True)

    def test_policy_rejects_stale_head_unknown_event_and_forks(self):
        self.land_preparation()
        candidate = self.repo.candidate()
        event = self.event_for(candidate)
        mutations = [lambda value: value.update(action="closed"),
                     lambda value: value["pull_request"]["head"].update(sha=self.initial_main),
                     lambda value: value["pull_request"]["head"]["repo"].update(full_name="outsider/fork"),
                     lambda value: value["pull_request"]["head"].update(ref="develop"),
                     lambda value: value["pull_request"]["base"].update(ref="develop")]
        for mutate in mutations:
            bad = copy.deepcopy(event)
            mutate(bad)
            with self.subTest(event=bad), self.assertRaises(release.ReleaseError):
                self.repo.policy(bad, report_status=True)
            self.assertEqual(self.repo.statuses[-1][1]["state"], "failure")

    def test_policy_rechecks_head_immediately_before_success(self):
        self.land_preparation()
        candidate = self.repo.candidate()
        event = self.event_for(candidate)
        def advance_on_second_read(count):
            if count == 2:
                self.repo.pulls[event["number"] - 1]["head"]["sha"] = self.initial_main
        self.repo.on_pull_read = advance_on_second_read
        with self.assertRaisesRegex(release.ReleaseError, "Stale event"):
            self.repo.policy(event, report_status=True)
        self.assertEqual(self.repo.statuses[-1][1]["state"], "failure")

    def test_unmerged_foreign_version_tag_blocks_preparation(self):
        source = self.git("rev-parse", "develop")
        self.git("tag", "v1.1.0", source)
        self.git("push", "origin", "v1.1.0")
        self.assertFalse(self.repo.ancestor(source, self.initial_main))
        with self.assertRaisesRegex(release.ReleaseError, "already has a tag"):
            self.repo.prepare(dry_run=True)

    def test_policy_tolerates_develop_advancing_after_fetch(self):
        # Gap: develop advances between the full fetch inside policy() and
        # the live develop API read. The new tip is a remote object the
        # fixture has not fetched; the exact live SHA must be fetched
        # locally so exact_oid and the ancestry check can run.
        self.land_preparation()
        candidate = self.repo.candidate()
        event = self.event_for(candidate)
        advanced = set()
        live_ref = self.repo.live_ref

        def develop_then_advance(branch):
            if branch == "develop" and "develop" not in advanced:
                advanced.add("develop")
                self.advance_remote("develop", "fix: concurrent work", "late")
            return live_ref(branch)

        self.repo.live_ref = develop_then_advance
        result = self.repo.policy(event, report_status=True)
        self.assertEqual(result["source"], candidate["source"])
        self.assertEqual(self.repo.statuses[-1][1]["state"], "success")

    def test_policy_tolerates_develop_advancing_in_closing_guard(self):
        # Gap: develop advances between the closing guard's live develop
        # read and its ancestry check, with a genuinely new remote commit
        # the fixture has not fetched. The exact closing SHA must be
        # fetched locally so the guard verifies ancestry instead of
        # failing on an unfetched commit ID.
        self.land_preparation()
        candidate = self.repo.candidate()
        event = self.event_for(candidate)
        reads = {"main": 0, "develop": 0}
        live_ref = self.repo.live_ref

        def guarded_live_ref(branch, live_ref=live_ref):
            count = reads.get(branch, 0)
            reads[branch] = count + 1
            # The first live develop read succeeds; the second (closing)
            # read arms the advance so the guard observes the new tip.
            if count == 1 and branch == "develop":
                self.advance_remote("develop", "fix: concurrent work", "close")
            return live_ref(branch)

        self.repo.live_ref = guarded_live_ref
        result = self.repo.policy(event, report_status=True)
        self.assertEqual(result["source"], candidate["source"])
        self.assertEqual(self.repo.statuses[-1][1]["state"], "success")

    def test_policy_never_executes_head_owned_helper(self):
        scripts = self.checkout / "scripts"
        scripts.mkdir()
        marker = Path(self.temp.name) / "executed-untrusted-code"
        (scripts / "release.py").write_text(
            f"from pathlib import Path\nPath({str(marker)!r}).write_text('executed')\n")
        self.commit("feat: untrusted head helper")
        self.git("push", "origin", "develop")
        self.land_preparation()
        candidate = self.repo.candidate()
        self.repo.policy(self.event_for(candidate), report_status=True)
        self.assertFalse(marker.exists())
        self.assertEqual(self.repo.statuses[-1][1]["state"], "success")

    def test_origin_must_be_same_github_fetch_and_push_repository(self):
        guarded = release.Repository(self.checkout)
        with self.assertRaisesRegex(release.ReleaseError, "GitHub"):
            guarded.repository()
        self.git("remote", "set-url", "origin", "git@github.com:example/m4b-merge.git")
        self.assertEqual(guarded.repository(), "example/m4b-merge")
        self.git("remote", "set-url", "--push", "origin", "https://github.com/outsider/other.git")
        with self.assertRaisesRegex(release.ReleaseError, "same GitHub"):
            guarded.repository()

    def test_verify_cli_refreshes_a_stale_main_tracking_ref(self):
        self.land_preparation()
        candidate = self.repo.candidate()
        with self.repo.worktree("origin/main") as path:
            self.repo.run("git", "commit", "--allow-empty", "-m", "chore: advance main", cwd=path)
            advanced = self.repo.run("git", "rev-parse", "HEAD", cwd=path)
            self.repo.run("git", "push", "origin", f"{advanced}:refs/heads/main", cwd=path)
        self.git("update-ref", "refs/remotes/origin/main", candidate["base"])
        errors = io.StringIO()
        with patch.object(release, "Repository", return_value=self.repo), contextlib.redirect_stderr(errors):
            result = release.main(["verify", "--head", candidate["head"], "--base", candidate["base"]])
        self.assertEqual(result, 1)
        self.assertIn("Stale main base", errors.getvalue())
        self.assertEqual(self.repo.oid("origin/main"), advanced)

    def test_version_cli_outputs_only_validated_version(self):
        source = self.git("rev-parse", "develop")
        output = io.StringIO()
        with patch.object(release, "Repository", return_value=self.repo), contextlib.redirect_stdout(output):
            self.assertEqual(release.main(["version", "--ref", source]), 0)
        self.assertEqual(output.getvalue(), "v1.0.1\n")

    def remote_refset(self):
        return {ref: oid for oid, ref in
                (line.split() for line in self.repo.remote_git("show-ref").splitlines())}

    def fixture_refset(self):
        return {ref: oid for oid, ref, _ in
                (line.split() for line in self.git("for-each-ref").splitlines())}

    def test_unchanged_abort_arms_publish_nothing_when_remote_advances(self):
        # Live main/develop advancing between the fetched base and the push
        # must abort the operation before any ref is published or PR created.
        self.land_preparation()
        for name, operation, target, message in (
                ("candidate:main", self.repo.candidate, "main", "main advanced; retry"),
                ("prepare:main", self.repo.prepare, "main", "main advanced; retry"),
                ("prepare:develop", self.repo.prepare, "develop", "develop advanced; retry")):
            with self.subTest(arm=name):
                before = self.remote_refset()
                pulls = len(self.repo.pulls)
                live_ref = self.repo.live_ref

                def advancing_live_ref(branch, live_ref=live_ref):
                    # Advance on the first live read of the target branch: the
                    # remote ref moves from a scratch clone, out of process,
                    # before the owned push can run.
                    value = live_ref(branch)
                    if branch == target:
                        value = self.push_commit_to(target, "fix: inter-process advance",
                                                    f"gate-{name}")
                    return value

                self.repo.live_ref = advancing_live_ref
                try:
                    with self.assertRaisesRegex(release.ReleaseError, message):
                        operation()
                finally:
                    self.repo.live_ref = live_ref
                after = self.remote_refset()
                # The arm tip moved (the race is real, out-of-process) but no
                # owned ref was ever published: the ref set is unchanged and
                # every other ref kept its exact value.
                self.assertEqual(set(after), set(before))
                moved = f"refs/heads/{target}"
                for ref, oid in before.items():
                    if ref != moved:
                        self.assertEqual(after[ref], oid)
                self.assertNotEqual(after[moved], before[moved])

    def test_create_only_push_refuses_branch_created_between_check_and_push(self):
        self.land_preparation()
        source = self.git("rev-parse", "develop")
        branch = f"release/v1.1.0-{source[:12]}-{self.initial_main[:12]}"
        fixture = self.fixture_refset()
        # The raced ref must point at a genuinely foreign value: a create-only
        # push racing the empty-expectation lease with its own target value
        # would be a no-op and never raise.
        raced = source
        pulls = len(self.repo.pulls)
        real_run = self.repo.run

        def racing_run(*args, **kwargs):
            if args[:2] == ("git", "push") and any(
                    a.endswith(f":refs/heads/{branch}") for a in args):
                # The ref was absent at the remote_branch() check; a concurrent
                # operator creates it at a new main tip before the create-only
                # push reaches git.
                self.repo.remote_git("update-ref", f"refs/heads/{branch}", raced)
            return real_run(*args, **kwargs)

        self.repo.run = racing_run
        try:
            with self.assertRaisesRegex(release.ReleaseError, "stale info"):
                self.repo.candidate()
        finally:
            self.repo.run = real_run
        self.assertEqual(self.remote_refset()[f"refs/heads/{branch}"], raced)
        self.assertEqual(self.fixture_refset(), fixture)
        self.assertEqual(len(self.repo.pulls), pulls)

    def test_prepare_is_idempotent_when_develop_already_prepared(self):
        self.land_preparation()
        refs = self.remote_refset()
        working_head = self.git("rev-parse", "HEAD")
        result = self.repo.prepare(version="1.1.0")
        self.assertEqual(result["message"], "develop already contains the prepared version")
        self.assertEqual(result["version"], "v1.1.0")
        self.assertEqual(result["head"], self.git("rev-parse", "develop"))
        self.assertIsNone(result["branch"])
        self.assertIsNone(result["pull_request"])
        self.assertIsNone(result["pending_pr"])
        self.assertEqual(self.remote_refset(), refs)
        # No open PR may be created: the landed preparation's PR stays closed.
        self.assertEqual([p["state"] for p in self.repo.pulls], ["closed"])
        self.assertEqual(self.git("rev-parse", "HEAD"), working_head)

    def test_policy_repository_gate_fails_before_any_api_request(self):
        self.land_preparation()
        candidate = self.repo.candidate()
        event = self.event_for(candidate)
        event["repository"]["full_name"] = "outsider/fork"
        calls = []
        real_api = self.repo.api

        def recording_api(endpoint, method="GET", fields=None):
            calls.append((endpoint, method))
            return real_api(endpoint, method, fields)

        self.repo.api = recording_api
        try:
            with self.assertRaisesRegex(release.ReleaseError, "Event repository does not match origin"):
                self.repo.policy(event, report_status=True)
        finally:
            self.repo.api = real_api
        self.assertEqual(calls, [])
        self.assertEqual(self.repo.statuses, [])

    def test_policy_closing_guards_race_live_main_and_develop(self):
        self.land_preparation()
        candidate = self.repo.candidate()
        event = self.event_for(candidate)
        cases = [
            (None, None, "success"),
            ("main-advanced", "main advanced during policy verification", "failure"),
            ("develop-rebased", "Source no longer belongs to develop", "failure"),
        ]
        for mode, expected, state in cases:
            with self.subTest(arm=mode or "success"):
                reads = {"main": 0, "develop": 0}
                target = {"main-advanced": "main", "develop-rebased": "develop"}.get(mode)
                live_ref = self.repo.live_ref

                def guarded_live_ref(branch, live_ref=live_ref):
                    count = reads.get(branch, 0)
                    reads[branch] = count + 1
                    # The first live read of the target branch succeeds; the
                    # second read races it out of process so the closing
                    # guard observes the new value.
                    if count >= 1 and target and branch == target:
                        if target == "main":
                            self.push_commit_to("main", "fix: inter-process promotion",
                                                "policy-main-race")
                        else:
                            self.repo.remote_git("update-ref", "refs/heads/develop",
                                                 self.initial_main)
                    return live_ref(branch)

                pristine_main = self.repo.remote_git("rev-parse", "refs/heads/main")
                pristine_develop = self.repo.remote_git("rev-parse", "refs/heads/develop")
                self.repo.live_ref = guarded_live_ref
                try:
                    if expected is None:
                        result = self.repo.policy(event, report_status=True)
                        self.assertEqual(result["source"], candidate["source"])
                    else:
                        with self.assertRaisesRegex(release.ReleaseError, expected):
                            self.repo.policy(event, report_status=True)
                finally:
                    self.repo.live_ref = live_ref
                self.assertEqual(self.repo.statuses[-1][1]["state"], state)
                self.repo.remote_git("update-ref", "refs/heads/main", pristine_main)
                self.repo.remote_git("update-ref", "refs/heads/develop", pristine_develop)

    def test_ensure_pr_fails_closed_on_head_move_duplicate_and_mismatch(self):
        self.land_preparation()
        candidate = self.repo.candidate()
        branch, head = candidate["branch"], candidate["head"]
        archive = copy.deepcopy(self.repo.pulls)
        repository = self.repo.repository()

        def open_pull(number, sha):
            return {"number": number, "state": "open",
                    "html_url": f"https://github.com/{repository}/pull/{number}",
                    "head": {"ref": branch, "sha": sha, "repo": {"full_name": repository}},
                    "base": {"ref": "main", "sha": candidate["base"],
                             "repo": {"full_name": repository}}}

        mismatch = open_pull(2, head)
        mismatch["head"]["sha"] = self.initial_main
        cases = [
            ("head-moved", self.initial_main, [],
             "Release branch changed before PR creation"),
            ("duplicate", head, [open_pull(1, head), open_pull(2, head)],
             "More than one open release PR exists"),
            ("mismatch", head, [mismatch],
             "Existing PR does not match release branch and head"),
        ]
        try:
            for name, remote_head, pulls, message in cases:
                with self.subTest(arm=name):
                    # Re-anchor the remote branch on the expected head so the
                    # ls-remote gate is the only thing each arm exercises.
                    self.repo.remote_git("update-ref", f"refs/heads/{branch}", remote_head)
                    self.repo.pulls = copy.deepcopy(pulls)
                    with self.assertRaisesRegex(release.ReleaseError, message):
                        self.repo.ensure_pr(branch, "main", head,
                                             "chore(release): v1.1.0", "body")
        finally:
            self.repo.remote_git("update-ref", f"refs/heads/{branch}", head)
            self.repo.pulls = archive

    def test_policy_failure_status_description_is_truncated_to_the_github_cap(self):
        self.land_preparation()
        candidate = self.repo.candidate()
        event = self.event_for(candidate)
        long_failure = "policy verification: " + "x" * (500 - len("policy verification: "))
        real_verify = self.repo.verify

        def failing_verify(*args, **kwargs):
            raise release.ReleaseError(long_failure)

        self.repo.verify = failing_verify
        try:
            with self.assertRaisesRegex(release.ReleaseError, r"policy verification: x+"):
                self.repo.policy(event, report_status=True)
        finally:
            self.repo.verify = real_verify
        field = self.repo.statuses[-1][1]
        self.assertEqual(self.repo.statuses[-1][0], candidate["head"])
        self.assertEqual(field["state"], "failure")
        self.assertEqual(field["context"], "Release policy")
        self.assertLessEqual(len(field["description"]), release.STATUS_DESCRIPTION_LIMIT)
        self.assertEqual(field["description"], long_failure[:release.STATUS_DESCRIPTION_LIMIT])

    def test_main_dispatches_policy_check_publication_and_tag(self):
        self.land_preparation()
        candidate = self.repo.candidate()
        event = self.event_for(candidate)
        event_path = Path(self.temp.name) / "event.json"
        event_path.write_text(json.dumps(event))

        def dispatch(argv):
            output = io.StringIO()
            with patch.object(release, "Repository", return_value=self.repo), \
                    contextlib.redirect_stdout(output):
                self.assertEqual(release.main(argv), 0)
            return output.getvalue()

        # The policy arm needs the promotion PR open, so it runs before the
        # candidate lands and closes it; check-publication/tag need the
        # landed main.
        with self.subTest(argv="policy"):
            self.assertEqual(
                json.loads(dispatch(
                    ["policy", "--event", str(event_path), "--report-status"]))["version"],
                candidate["version"])
        final = self.land_candidate(candidate)
        for argv, expect in (
                (["check-publication", "--sha", final], "v1.1.0\n"),
                (["tag", "--sha", final], "v1.1.0\n")):
            with self.subTest(argv=argv[0]):
                self.assertEqual(dispatch(argv), expect)


class ReleaseWorkflowGuards(unittest.TestCase):
    """Guard conditions committed in .github/workflows/release.yml."""

    WORKFLOW = Path(__file__).resolve().parent.parent / ".github" / "workflows" / "release.yml"
    # The ref-guard job uses a plain `github.<context> op 'literal'` clause
    # joined by `&&`; the evaluator below is exactly that shape, so a guard
    # whose syntax outgrows it fails the test loudly instead of being
    # evaluated silently.
    CLAUSE = re.compile(r"^github\.(\w+) (==|!=) '([^']*)'$")

    @classmethod
    def ref_guard_expression(cls):
        # Read the guard as text so the suite needs no YAML parser: the
        # ref-guard job's if: is a single-line scalar in the committed
        # workflow, and the checks below keep that assumption loud instead
        # of letting it slip (a block scalar or a non-if first key fails
        # the test).
        #
        # The search is scoped to the top-level `jobs:` mapping, mirrored
        # from a YAML `jobs.ref-guard` lookup: the job must be a direct
        # child of `jobs:` (indent 1-3 under the indent-0 key), and the
        # block ends at the next top-level key, so a `ref-guard:` block
        # relocated outside `jobs:` cannot satisfy the guard.
        lines = cls.WORKFLOW.read_text().splitlines()
        jobs_start = None
        for i, line in enumerate(lines):
            if line == "jobs:":
                jobs_start = i
                break
        if jobs_start is None:
            raise AssertionError("jobs: mapping not found in release.yml")
        jobs_end = len(lines)
        for j in range(jobs_start + 1, len(lines)):
            line = lines[j]
            # A top-level key is unindented and not a comment.
            if line and not line[0].isspace() and not line.lstrip().startswith("#"):
                jobs_end = j
                break
        for i in range(jobs_start + 1, jobs_end):
            line = lines[i]
            stripped = line.strip()
            if stripped != "ref-guard:":
                continue
            # Direct child of the indent-0 `jobs:` mapping (any indent 1-3).
            indent = len(line) - len(line.lstrip(" "))
            if not 1 <= indent < 4:
                continue
            follows = [follow.strip() for follow in lines[i + 1:jobs_end]
                       if follow.strip()]
            if not follows:
                raise AssertionError("jobs.ref-guard has no body")
            stripped = follows[0]
            match = re.match(r"^if:(.*)$", stripped)
            if match is None:
                raise AssertionError(
                    "ref-guard job's first key is not its if: guard: "
                    f"{stripped!r}")
            expression = match.group(1).strip()
            if not expression or expression in ("|", ">", "|-", ">-"):
                raise AssertionError(
                    "ref-guard if: must be a single-line scalar: "
                    f"{stripped!r}")
            return expression
        raise AssertionError("jobs.ref-guard job not found in release.yml")

    @classmethod
    def evaluate_ref_guard(cls, event_name, ref):
        expression = cls.ref_guard_expression()
        for clause in (part.strip() for part in expression.split("&&")):
            match = cls.CLAUSE.match(clause)
            if match is None:
                raise AssertionError(f"ref-guard clause not in the asserted shape: {clause!r}")
            context, operator, literal = match.groups()
            value = {"event_name": event_name, "ref": ref}[context]
            if (operator == "==" and value != literal) or (operator == "!=" and value == literal):
                return False
        return True

    def test_release_workflow_ref_guard_rejects_non_main_dispatch(self):
        expression = self.ref_guard_expression()
        # The guard trips only on a manual dispatch from a ref that is not
        # main; a main dispatch must run the guarded refusal path off.
        clauses = [part.strip() for part in expression.split("&&")]
        self.assertIn("github.event_name == 'workflow_dispatch'", clauses)
        self.assertIn("github.ref != 'refs/heads/main'", clauses)
        # Non-main dispatch: the guard evaluates true, so the job refuses the
        # run.
        with self.subTest(ref="non-main"):
            self.assertTrue(
                self.evaluate_ref_guard("workflow_dispatch", "refs/heads/release/v2.0.0-20260101"))
        with self.subTest(ref="main"):
            self.assertFalse(
                self.evaluate_ref_guard("workflow_dispatch", "refs/heads/main"))
        # A push run (the normal publication path) is never dispatch-guarded.
        with self.subTest(event="push"):
            self.assertFalse(self.evaluate_ref_guard("push", "refs/heads/v2.0.0"))


if __name__ == "__main__":
    unittest.main()
