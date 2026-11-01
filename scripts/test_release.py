#!/usr/bin/env python3
"""Real git-history regressions; only the remote GitHub transport is replaced."""
import copy
import json
import re
from pathlib import Path
import subprocess
import tempfile
import unittest

import release


class LocalGitHub(release.Repository):
    def __init__(self, root, remote):
        super().__init__(root)
        self.remote = remote
        self.pulls = []
        self.statuses = []
        self.releases = {}
        self.extra_records = []
        self.release_commands = []
        self.list_error = None
        self.list_calls = 0
        self.latest = None

    def repository(self):
        return "example/m4b-merge"

    def remote_git(self, *args):
        return self.run("git", "--git-dir", str(self.remote), *args)

    def api(self, endpoint, method="GET", fields=None):
        path = endpoint.removeprefix("repos/example/m4b-merge/")
        if path == "git/refs" and method == "POST":
            self.remote_git("update-ref", fields["ref"], fields["sha"], "0" * 40)
            return {"ref": fields["ref"]}
        if path.startswith("git/ref/"):
            ref = path.removeprefix("git/ref/")
            return {"object": {"type": "commit", "sha": self.remote_git("rev-parse", f"refs/{ref}^{{commit}}")}}
        if path.startswith("pulls/"):
            return copy.deepcopy(self.pulls[int(path.split("/")[1]) - 1])
        if path.startswith("statuses/"):
            self.statuses.append(fields)
            return fields
        if path == "releases/latest":
            record = (self.releases.get(self.latest) or next(
                (r for r in self.extra_records if r.get("tag_name") == self.latest), None)) \
                if self.latest is not None else None
            if record is None:
                raise release.ReleaseError("HTTP 404: Not Found")
            return copy.deepcopy(record)
        if path.startswith("releases?"):
            # GitHub doc semantics: the list includes drafts for authorized
            # users, so the store (dict) plus injected records are served.
            params = dict(part.split("=", 1) for part in path[len("releases?"):].split("&") if "=" in part)
            per_page, page = int(params.get("per_page", "30")), int(params.get("page", "1"))
            if self.list_error is not None:
                raise release.ReleaseError(self.list_error)
            records = list(self.releases.values()) + self.extra_records
            start = (page - 1) * per_page
            self.list_calls += 1
            return copy.deepcopy(records[start:start + per_page])
        if path.startswith("releases/tags/"):
            # Published-release lookup only: drafts are not disclosed here.
            record = self.releases.get(path.removeprefix("releases/tags/"))
            if record is None or record.get("draft"):
                raise release.ReleaseError("HTTP 404: Not Found (published by-tag lookup)")
            return copy.deepcopy(record)
        raise AssertionError((endpoint, method, fields))

    def run(self, *args, ok=(0,)):
        if len(args) >= 4 and args[0] == "gh" and args[1] == "release" and args[2] in ("create", "edit"):
            tag, record = args[3], self.releases.get(args[3])
            if args[2] == "create":
                if record is not None:
                    raise release.ReleaseError(f"gh release create: release {tag} already exists")
                self.release_commands.append("create")
                self.releases[tag] = {"tag_name": tag, "draft": "--draft" in args}
                return ""
            self.release_commands.append("edit")
            if "--draft=false" in args:
                # Newly-published releases default to GitHub Latest unless
                # the edit explicitly opts out.
                if record["draft"] and "--latest=false" not in args:
                    self.latest = tag
                record["draft"] = False
            elif "--latest=true" in args:
                self.latest = tag
            return ""
        return super().run(*args, ok=ok)


class ReleaseBehavior(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="m4b-reservation-")
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.remote = root / "remote.git"
        self.checkout = root / "checkout"
        self.cmd(root, "git", "init", "--bare", str(self.remote))
        self.cmd(root, "git", "init", "-b", "main", str(self.checkout))
        self.repo = LocalGitHub(self.checkout, self.remote)
        for key, value in (("user.name", "Release Test"), ("user.email", "test@example.invalid"), ("commit.gpgsign", "false"), ("tag.gpgsign", "false")):
            self.git("config", key, value)
        self.git("remote", "add", "origin", str(self.remote))
        self.commit("initial", "one")
        self.git("tag", "v1.0.0")
        self.git("push", "origin", "main", "--tags")
        self.git("switch", "-c", "develop")
        self.commit("feat: new behavior", "two")
        self.git("push", "origin", "develop")
        self.git("switch", "main")
        self.git("merge", "--no-ff", "develop", "-m", "Promote develop")
        self.git("push", "origin", "main")
        self.source = self.git("rev-parse", "HEAD")
        self.repo.fetch()

    def cmd(self, cwd, *args):
        result = subprocess.run(args, cwd=cwd, env=release.isolated_git_environment(), text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        return result.stdout.strip()

    def git(self, *args):
        return self.cmd(self.checkout, "git", *args)

    def commit(self, message, content):
        (self.checkout / "source.txt").write_text(content)
        self.git("add", "source.txt")
        self.git("commit", "-m", message)

    def event(self):
        repository = {"full_name": self.repo.repository()}
        pull = {"state": "open", "head": {"ref": "develop", "sha": self.git("rev-parse", "develop"), "repo": repository}, "base": {"ref": "main", "sha": self.source, "repo": repository}}
        self.repo.pulls = [copy.deepcopy(pull)]
        return {"action": "opened", "number": 1, "repository": repository, "pull_request": pull}

    def next_develop(self):
        self.git("switch", "develop")
        self.commit("fix: next behavior", "three")
        self.git("push", "origin", "develop")
        self.repo.fetch()
        return self.git("rev-parse", "HEAD")

    def test_reservation_reuses_exact_source_after_newer_release(self):
        tag = self.repo.reserve(self.source)
        self.assertEqual(tag, "v1.1.0")
        self.next_develop()
        self.git("switch", "main")
        self.git("merge", "--no-ff", "develop", "-m", "Next promotion")
        self.git("push", "origin", "main")
        newer = self.git("rev-parse", "HEAD")
        self.assertEqual(self.repo.reserve(newer), "v1.1.1")
        self.assertEqual(self.repo.reserve(self.source), tag)
        self.assertEqual(self.repo.remote_git("rev-parse", "refs/tags/v1.1.0"), self.source)

    def test_unfinished_reservation_is_not_reallocated(self):
        self.assertEqual(self.repo.reserve(self.source), "v1.1.0")
        self.assertEqual(self.repo.reserve(self.source, bump="major"), "v1.1.0")
        self.assertEqual(self.repo.select(self.source, "v1.1.0"), "v1.1.0")
        with self.assertRaises(release.ReleaseError):
            self.repo.reserve(self.source, "v2.0.0")

    def test_other_branch_tag_cannot_set_reachable_baseline(self):
        self.git("switch", "-c", "unrelated", "v1.0.0")
        self.commit("unrelated", "elsewhere")
        self.git("tag", "v99.0.0")
        self.git("push", "origin", "v99.0.0")
        self.repo.fetch()
        self.assertEqual(self.repo.select(self.source), "v1.1.0")

    def test_explicit_different_source_collision_never_moves_tag(self):
        with self.assertRaises(release.ReleaseError):
            self.repo.reserve(self.source, "v1.0.0")
        self.assertEqual(self.repo.remote_git("rev-parse", "refs/tags/v1.0.0"), self.git("rev-parse", "v1.0.0"))

    def test_ambiguous_exact_source_reservations_fail(self):
        for tag in ("v1.1.0", "v1.2.0"):
            self.git("tag", tag, self.source)
            self.git("push", "origin", tag)
        self.repo.fetch()
        with self.assertRaises(release.ReleaseError):
            self.repo.select(self.source)

    def test_release_refuses_tag_pointing_at_another_source(self):
        initial = self.git("rev-parse", "v1.0.0")
        self.assertNotEqual(initial, self.source)
        # Release metadata routes raise AssertionError in this fixture, so a
        # ReleaseError proves the frozen-source check failed closed first.
        with self.assertRaisesRegex(release.ReleaseError, "does not point at the frozen source"):
            self.repo.release("v1.0.0", self.source, publish=True)
        self.assertEqual(self.repo.remote_git("rev-parse", "refs/tags/v1.0.0"), initial)

    def test_release_draft_publish_cycle_is_idempotent(self):
        # v1.10.0 beats the published v1.9.0 numerically, not lexically.
        self.repo.extra_records.append(
            {"tag_name": "v1.9.0", "draft": False, "prerelease": False})
        self.repo.latest = "v1.9.0"
        self.assertEqual(
            self.repo.api("repos/example/m4b-merge/releases/latest")["tag_name"], "v1.9.0")
        self.assertEqual(self.repo.reserve(self.source, version="v1.10.0"), "v1.10.0")
        self.repo.fetch()
        self.repo.release("v1.10.0", self.source)
        self.assertEqual(self.repo.release_commands, ["create"])
        draft = self.repo.releases["v1.10.0"]
        self.assertTrue(draft["draft"])
        self.repo.release("v1.10.0", self.source)
        self.assertEqual(self.repo.release_commands, ["create"])
        self.assertIs(self.repo.releases["v1.10.0"], draft)
        self.repo.release("v1.10.0", self.source, publish=True)
        self.assertEqual(self.repo.release_commands, ["create", "edit"])
        self.assertFalse(self.repo.releases["v1.10.0"]["draft"])
        self.assertEqual(
            self.repo.api("repos/example/m4b-merge/releases/latest")["tag_name"], "v1.10.0")
        self.repo.release("v1.10.0", self.source, publish=True)
        self.assertEqual(self.repo.release_commands, ["create", "edit"])
        self.assertEqual(len(self.repo.releases), 1)
        self.assertEqual(self.repo.remote_git("rev-parse", "refs/tags/v1.10.0"), self.source)

    def test_publish_without_existing_draft_is_refused(self):
        self.git("tag", "v1.1.0", self.source)
        self.git("push", "origin", "v1.1.0")
        self.repo.fetch()
        with self.assertRaisesRegex(release.ReleaseError, "Cannot finalize a missing draft release"):
            self.repo.release("v1.1.0", self.source, publish=True)
        self.assertEqual(self.repo.releases, {})

    def publish_tag(self, tag):
        self.git("tag", tag, self.source)
        self.git("push", "origin", tag)
        self.repo.fetch()

    def test_publish_higher_than_published_promotes_latest(self):
        self.publish_tag("v1.3.0")
        self.repo.extra_records = [
            {"tag_name": "v2.0.0", "draft": False, "prerelease": False},
            {"tag_name": "v3.0.0-rc.1", "draft": False, "prerelease": True},
            {"tag_name": "v9.9.9", "draft": True, "prerelease": False},
        ]
        self.repo.latest = "v2.0.0"
        self.assertEqual(
            self.repo.api("repos/example/m4b-merge/releases/latest")["tag_name"], "v2.0.0")
        self.repo.release("v1.3.0", self.source)
        self.repo.release("v1.3.0", self.source, publish=True)
        self.assertEqual(self.repo.release_commands, ["create", "edit"])
        self.assertFalse(self.repo.releases["v1.3.0"]["draft"])
        self.assertEqual(
            self.repo.api("repos/example/m4b-merge/releases/latest")["tag_name"], "v2.0.0")
        self.repo.release("v1.3.0", self.source, publish=True)

    def test_published_highest_release_reconciles_stale_latest(self):
        self.publish_tag("v1.3.0")
        # The old helper published v1.3.0 with --latest=false: it is the
        # highest published stable, but GitHub Latest still points at the
        # older v1.1.0, so this version must advance it with one edit.
        self.repo.extra_records = [{"tag_name": "v1.1.0", "draft": False, "prerelease": False}]
        self.repo.latest = "v1.1.0"
        self.repo.releases["v1.3.0"] = {"tag_name": "v1.3.0", "draft": False, "prerelease": False}
        self.assertEqual(
            self.repo.api("repos/example/m4b-merge/releases/latest")["tag_name"], "v1.1.0")
        self.repo.release("v1.3.0", self.source, publish=True)
        self.assertEqual(self.repo.release_commands, ["edit"])
        self.assertEqual(len(self.repo.releases), 1)
        self.assertEqual(
            self.repo.api("repos/example/m4b-merge/releases/latest")["tag_name"], "v1.3.0")

    def test_published_highest_release_restores_missing_latest(self):
        self.publish_tag("v1.3.0")
        # The old helper published v1.3.0 with --latest=false and nothing else
        # advanced it: GET releases/latest 404s, so reconciliation must not
        # skip the missing-Latest case and must restore it with one edit.
        self.repo.extra_records = [{"tag_name": "v1.1.0", "draft": False, "prerelease": False}]
        self.repo.latest = None
        with self.assertRaisesRegex(release.ReleaseError, "HTTP 404"):
            self.repo.api("repos/example/m4b-merge/releases/latest")
        self.repo.releases["v1.3.0"] = {"tag_name": "v1.3.0", "draft": False, "prerelease": False}
        self.repo.release("v1.3.0", self.source, publish=True)
        self.assertEqual(self.repo.release_commands, ["edit"])
        self.assertEqual(self.repo.latest, "v1.3.0")
        self.repo.release("v1.3.0", self.source, publish=True)
        self.assertEqual(self.repo.release_commands, ["edit"])
        self.assertEqual(self.repo.latest, "v1.3.0")

    def test_published_lower_release_never_promotes_latest(self):
        self.publish_tag("v1.3.0")
        # A higher published stable v2.0.0 exists, so the lower v1.3.0 must
        # perform no edit and leave GitHub Latest unchanged.
        self.repo.extra_records = [{"tag_name": "v2.0.0", "draft": False, "prerelease": False}]
        self.repo.latest = "v2.0.0"
        self.repo.releases["v1.3.0"] = {"tag_name": "v1.3.0", "draft": False, "prerelease": False}
        self.assertEqual(
            self.repo.api("repos/example/m4b-merge/releases/latest")["tag_name"], "v2.0.0")
        self.repo.release("v1.3.0", self.source, publish=True)
        self.assertEqual(self.repo.release_commands, [])
        self.assertEqual(len(self.repo.releases), 1)
        self.assertEqual(
            self.repo.api("repos/example/m4b-merge/releases/latest")["tag_name"], "v2.0.0")

    def test_published_release_with_current_latest_performs_no_write(self):
        self.publish_tag("v1.3.0")
        # GitHub Latest already points at this published release: an ordinary
        # idempotent retry must be a complete no-op, not a redundant edit.
        self.repo.latest = "v1.3.0"
        self.repo.releases["v1.3.0"] = {"tag_name": "v1.3.0", "draft": False, "prerelease": False}
        self.assertEqual(
            self.repo.api("repos/example/m4b-merge/releases/latest")["tag_name"], "v1.3.0")
        self.repo.release("v1.3.0", self.source, publish=True)
        self.assertEqual(self.repo.release_commands, [])
        self.assertEqual(len(self.repo.releases), 1)
        self.assertEqual(
            self.repo.api("repos/example/m4b-merge/releases/latest")["tag_name"], "v1.3.0")

    def test_prerelease_and_draft_higher_records_never_block_latest(self):
        self.publish_tag("v2.1.0")
        # Only a prerelease and a draft sit above v2.1.0: neither is published
        # stable, so the publish passes --latest=true and Latest advances to it.
        self.repo.extra_records = [
            {"tag_name": "v3.0.0-rc.1", "draft": False, "prerelease": True},
            {"tag_name": "v9.9.9", "draft": True, "prerelease": False},
        ]
        self.repo.latest = "v1.9.0"
        self.repo.releases["v1.9.0"] = {"tag_name": "v1.9.0", "draft": False, "prerelease": False}
        self.assertEqual(
            self.repo.api("repos/example/m4b-merge/releases/latest")["tag_name"], "v1.9.0")
        self.repo.release("v2.1.0", self.source)
        self.repo.release("v2.1.0", self.source, publish=True)
        self.assertEqual(self.repo.release_commands, ["create", "edit"])
        self.assertFalse(self.repo.releases["v2.1.0"]["draft"])
        self.assertEqual(
            self.repo.api("repos/example/m4b-merge/releases/latest")["tag_name"], "v2.1.0")
        self.repo.release("v2.1.0", self.source, publish=True)
        self.assertEqual(
            self.repo.api("repos/example/m4b-merge/releases/latest")["tag_name"], "v2.1.0")
        self.assertEqual(self.repo.release_commands, ["create", "edit"])

    def test_release_listing_pagination_walks_all_pages(self):
        initial = self.git("rev-parse", "v1.0.0")
        self.repo.extra_records = [{"tag_name": f"v0.0.{i}", "draft": False, "prerelease": False}
                                   for i in range(105)]
        self.repo.latest = "v0.0.104"
        self.assertEqual(
            self.repo.api("repos/example/m4b-merge/releases/latest")["tag_name"], "v0.0.104")
        self.repo.release("v1.0.0", initial)
        self.assertEqual(self.repo.release_commands, ["create"])
        self.assertGreaterEqual(self.repo.list_calls, 2)
        self.repo.release("v1.0.0", initial, publish=True)
        self.assertEqual(self.repo.release_commands, ["create", "edit"])
        # v1.0.0 is the new highest published stable: the seeded page-2 Latest advances.
        self.assertEqual(
            self.repo.api("repos/example/m4b-merge/releases/latest")["tag_name"], "v1.0.0")

    def test_release_duplicate_tag_records_fail_closed(self):
        self.repo.extra_records = [
            {"tag_name": "v1.0.0", "draft": False, "prerelease": False},
            {"tag_name": "v1.0.0", "draft": True, "prerelease": False},
        ]
        with self.assertRaisesRegex(release.ReleaseError, "Multiple release records share tag v1.0.0"):
            self.repo.release("v1.0.0", self.git("rev-parse", "v1.0.0"))
        self.assertEqual(self.repo.release_commands, [])

    def test_release_listing_failure_propagates_without_release_mutation(self):
        initial = self.git("rev-parse", "v1.0.0")
        for message in ("HTTP 502: Bad Gateway (remote registry unavailable)", "HTTP 403: Rate limit exceeded, please try again later."):
            self.repo.list_error = message
            with self.assertRaisesRegex(release.ReleaseError, re.escape(message)):
                self.repo.release("v1.0.0", initial, publish=True)
        self.assertEqual(self.repo.releases, {})
        self.assertEqual(self.repo.release_commands, [])

    def test_unreserved_automatic_candidate_is_coalesced_but_manual_is_not(self):
        old = self.source
        self.next_develop()
        self.git("switch", "main")
        self.git("merge", "--no-ff", "develop", "-m", "Next promotion")
        self.git("push", "origin", "main")
        self.repo.reserve(self.git("rev-parse", "HEAD"), "v1.2.0")
        self.repo.fetch()
        self.assertIsNone(self.repo.select(old, automatic=True))
        self.assertEqual(self.repo.select(old), "v1.1.0")

    def test_successive_direct_promotions_accept_merge_only_main_history(self):
        head = self.next_develop()
        self.assertEqual(self.repo.verify(head, self.source), head)
        self.git("switch", "main")
        self.git("merge", "--no-ff", "develop", "-m", "Second promotion")
        self.git("push", "origin", "main")
        second = self.git("rev-parse", "HEAD")
        self.git("switch", "develop")
        self.commit("fix: third change", "four")
        self.git("push", "origin", "develop")
        self.repo.fetch()
        self.assertEqual(self.repo.verify(self.git("rev-parse", "HEAD"), second), self.git("rev-parse", "HEAD"))

    def test_main_only_source_change_is_rejected(self):
        self.commit("independent main fix", "main only")
        self.git("push", "origin", "main")
        main = self.git("rev-parse", "HEAD")
        head = self.next_develop()
        with self.assertRaises(release.ReleaseError):
            self.repo.verify(head, main)

    def test_guard_fails_wrong_head_and_reports_red_instead_of_skipping(self):
        self.next_develop()
        event = self.event()
        event["pull_request"]["head"]["ref"] = "feature/pretend-promotion"
        self.repo.pulls[0] = copy.deepcopy(event["pull_request"])
        with self.assertRaises(release.ReleaseError):
            self.repo.policy(event, report_status=True)
        self.assertEqual(self.repo.statuses[-1]["state"], "failure")
        self.assertEqual(self.repo.statuses[-1]["context"], "Promotion guard")

    def test_guard_reports_direct_develop_and_rejects_new_head(self):
        self.next_develop()
        event = self.event()
        self.repo.policy(event, report_status=True)
        self.assertEqual(self.repo.statuses[-1]["state"], "success")
        self.commit("fix: source advanced", "four")
        self.git("push", "origin", "develop")
        with self.assertRaises(release.ReleaseError):
            self.repo.policy(event, report_status=True)
        self.assertEqual(self.repo.statuses[-1]["state"], "failure")


if __name__ == "__main__":
    unittest.main()
