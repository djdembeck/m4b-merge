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


if __name__ == "__main__":
    unittest.main()
