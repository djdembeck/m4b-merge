<!-- omit in toc -->
# Contributing to m4b-merge

First off, thanks for taking the time to contribute! ❤️

All types of contributions are encouraged and valued. See the [Table of Contents](#table-of-contents) for different ways to help and details about how this project handles them. Please make sure to read the relevant section before making your contribution. It will make it a lot easier for us maintainers and smooth out the experience for all involved. The community looks forward to your contributions. 🎉

> And if you like the project, but just don't have time to contribute, that's fine. There are other easy ways to support the project and show your appreciation, which we would also be very happy about:
> - Star the project
> - Tweet about it
> - Refer this project in your project's readme
> - Mention the project at local meetups and tell your friends/colleagues

<!-- omit in toc -->
## Table of Contents

- [I Have a Question](#i-have-a-question)
- [I Want To Contribute](#i-want-to-contribute)
  - [Reporting Bugs](#reporting-bugs)
  - [Suggesting Enhancements](#suggesting-enhancements)
  - [Your First Code Contribution](#your-first-code-contribution)
  - [Improving The Documentation](#improving-the-documentation)
- [Releasing](#releasing)
- [Styleguides](#styleguides)
  - [Commit Messages](#commit-messages)
- [Join The Project Team](#join-the-project-team)



## I Have a Question

> If you want to ask a question, we assume that you have read the available [Documentation](https://github.com/djdembeck/m4b-merge).

Before you ask a question, it is best to search for existing [Issues](https://github.com/djdembeck/m4b-merge/issues) that might help you. In case you have found a suitable issue and still need clarification, you can write your question in this issue. It is also advisable to search the internet for answers first.

If you then still feel the need to ask a question and need clarification, we recommend the following:

- Open an [Issue](https://github.com/djdembeck/m4b-merge/issues/new).
- Provide as much context as you can about what you're running into.
- Provide project and platform versions (nodejs, npm, etc), depending on what seems relevant.

We will then take care of the issue as soon as possible.

<!--
You might want to create a separate issue tag for questions and include it in this description. People should then tag their issues accordingly.

Depending on how large the project is, you may want to outsource the questioning, e.g. to Stack Overflow or Gitter. You may add additional contact and information possibilities:
- IRC
- Slack
- Gitter
- Stack Overflow tag
- Blog
- FAQ
- Roadmap
- E-Mail List
- Forum
-->

## I Want To Contribute

> ### Legal Notice <!-- omit in toc -->
> When contributing to this project, you must agree that you have authored 100% of the content, that you have the necessary rights to the content and that the content you contribute may be provided under the project license.

### Reporting Bugs

<!-- omit in toc -->
#### Before Submitting a Bug Report

A good bug report shouldn't leave others needing to chase you up for more information. Therefore, we ask you to investigate carefully, collect information and describe the issue in detail in your report. Please complete the following steps in advance to help us fix any potential bug as fast as possible.

- Make sure that you are using the latest version.
- Determine if your bug is really a bug and not an error on your side e.g. using incompatible environment components/versions (Make sure that you have read the [documentation](https://github.com/djdembeck/m4b-merge). If you are looking for support, you might want to check [this section](#i-have-a-question)).
- To see if other users have experienced (and potentially already solved) the same issue you are having, check if there is not already a bug report existing for your bug or error in the [bug tracker](https://github.com/djdembeck/m4b-merge/issues?q=label%3Abug).
- Also make sure to search the internet (including Stack Overflow) to see if users outside of the GitHub community have discussed the issue.
- Collect information about the bug:
  - Stack trace (Traceback)
  - OS, Platform and Version (Windows, Linux, macOS, x86, ARM)
  - Version of the interpreter, compiler, SDK, runtime environment, package manager, depending on what seems relevant.
  - Possibly your input and the output
  - Can you reliably reproduce the issue? And can you also reproduce it with older versions?

<!-- omit in toc -->
#### How Do I Submit a Good Bug Report?

> You must never report security related issues, vulnerabilities or bugs to the issue tracker, or elsewhere in public. Instead sensitive bugs must be sent by email to <>.
<!-- You may add a PGP key to allow the messages to be sent encrypted as well. -->

We use GitHub issues to track bugs and errors. If you run into an issue with the project:

- Open an [Issue](https://github.com/djdembeck/m4b-merge/issues/new). (Since we can't be sure at this point whether it is a bug or not, we ask you not to talk about a bug yet and not to label the issue.)
- Explain the behavior you would expect and the actual behavior.
- Please provide as much context as possible and describe the *reproduction steps* that someone else can follow to recreate the issue on their own. This usually includes your code. For good bug reports you should isolate the problem and create a reduced test case.
- Provide the information you collected in the previous section.

Once it's filed:

- The project team will label the issue accordingly.
- A team member will try to reproduce the issue with your provided steps. If there are no reproduction steps or no obvious way to reproduce the issue, the team will ask you for those steps and mark the issue as `needs-repro`. Bugs with the `needs-repro` tag will not be addressed until they are reproduced.
- If the team is able to reproduce the issue, it will be marked `needs-fix`, as well as possibly other tags (such as `critical`), and the issue will be left to be [implemented by someone](#your-first-code-contribution).

<!-- You might want to create an issue template for bugs and errors that can be used as a guide and that defines the structure of the information to be included. If you do so, reference it here in the description. -->


### Suggesting Enhancements

This section guides you through submitting an enhancement suggestion for m4b-merge, **including completely new features and minor improvements to existing functionality**. Following these guidelines will help maintainers and the community to understand your suggestion and find related suggestions.

<!-- omit in toc -->
#### Before Submitting an Enhancement

- Make sure that you are using the latest version.
- Read the [documentation](https://github.com/djdembeck/m4b-merge) carefully and find out if the functionality is already covered, maybe by an individual configuration.
- Perform a [search](https://github.com/djdembeck/m4b-merge/issues) to see if the enhancement has already been suggested. If it has, add a comment to the existing issue instead of opening a new one.
- Find out whether your idea fits with the scope and aims of the project. It's up to you to make a strong case to convince the project's developers of the merits of this feature. Keep in mind that we want features that will be useful to the majority of our users and not just a small subset. If you're just targeting a minority of users, consider writing an add-on/plugin library.

<!-- omit in toc -->
#### How Do I Submit a Good Enhancement Suggestion?

Enhancement suggestions are tracked as [GitHub issues](https://github.com/djdembeck/m4b-merge/issues).

- Use a **clear and descriptive title** for the issue to identify the suggestion.
- Provide a **step-by-step description of the suggested enhancement** in as many details as possible.
- **Describe the current behavior** and **explain which behavior you expected to see instead** and why. At this point you can also tell which alternatives do not work for you.
- You may want to **include screenshots and animated GIFs** which help you demonstrate the steps or point out the part which the suggestion is related to. You can use [this tool](https://www.cockos.com/licecap/) to record GIFs on macOS and Windows, and [this tool](https://github.com/colinkeenan/silentcast) or [this tool](https://github.com/GNOME/byzanz) on Linux. <!-- this should only be included if the project has a GUI -->
- **Explain why this enhancement would be useful** to most m4b-merge users. You may also want to point out the other projects that solved it better and which could serve as inspiration.

<!-- You might want to create an issue template for enhancement suggestions that can be used as a guide and that defines the structure of the information to be included. If you do so, reference it here in the description. -->

### Your First Code Contribution
<!-- TODO
include Setup of env, IDE and typical getting started instructions?

-->

### Improving The Documentation
<!-- TODO
Updating, improving and correcting the documentation

-->

## Releasing

m4b-merge publishes from a **frozen snapshot of `develop`** that is manually
promoted to `main`. There is no CI-driven merge from `develop` and no
automatic version bump on push — versions are bumped explicitly on
`develop` by `release.py prepare`. The `Release` publisher only runs
against the newer version a promotion just promoted to `main`; it never
republishes, retags, or rolls back anything older, and it never syncs
`main` back to `develop`.

Releases use the operator script `scripts/release.py` (Python 3.11+, stdlib
only; it uses your local `gh`/`git` for the same remote it does in CI).

### 1. Prepare a version on `develop`

After your changes are merged to `develop` and CI is green:

```bash
python3 scripts/release.py prepare --dry-run --bump auto   # actions preview
python3 scripts/release.py prepare --bump auto
```

Use `--bump minor` / `--bump major` / `--version 1.2.3` instead of
`--bump auto` for an explicit version. The preview reads the remote state
but creates nothing. `prepare` then opens a pull request to `develop` that bumps the `Cargo.toml` /
`Cargo.lock` package version on a short-lived
`release-prep/vVERSION-<source>` branch. **Merge it** and wait for CI —
the promotion in the next step freezes the commit `develop` points at.

### 2. Freeze the promotion candidate

```bash
python3 scripts/release.py candidate --dry-run   # actions preview
python3 scripts/release.py candidate
```

Both print a JSON result whose `head` (the new candidate SHA) and `base`
(the current `main` SHA the candidate is merged against) are the
`<CANDIDATE_SHA>` / `<CURRENT_MAIN_SHA>` values used in the next step's
`verify` command.

The command builds `release/vVERSION-<source>-<main-base>` against current
`main` and merges the *frozen snapshot of `develop`* with `--no-ff`, so the
promoted tree is byte-identical to `develop` at that SHA. On conflict (e.g.
a `main` hotfix that `develop` has not picked up), it fails hard: forward-port
`main` into `develop` (merge it and resolve there) so the frozen tree already
contains those changes, then re-run. `--resolved-commit OID` only reuses an
existing merge whose first parent is current `main`, whose second parent is
the frozen `develop` commit, and whose tree already equals that `develop`
tree — it re-checks exactly two parents, an untagged version newer than every
release tag reachable from `main`, and version agreement with the frozen tree
— so `main`-only changes never survive promotion.

### 3. Merge the promotion PR (human review)

The promotion is gated by human review, the `Release policy` status check
when required, and `main`'s native strict-base freshness protection.

- One-time bootstrap, in time order:
  - **Before the first promotion** — verify the candidate manually from a
    full clone; the trusted `Release policy` status protects `main` only
    once the workflow and helper are merged there.
  - **Always** — keep `main`'s native "strict base freshness" branch
    protection enabled — never relax it so an out-of-date promotion can
    merge.
  - **After the first promotion** — make the `Release policy` check
    **required** on `main`.
- Before merging, verify the candidate's first parent, exact frozen `develop`
  tree, and fresh version against the current `main` SHA:

  ```bash
  python3 scripts/release.py verify --head <CANDIDATE_SHA> --base <CURRENT_MAIN_SHA>
  ```

- After the merge, do not run candidate verification against the final merge
  commit. Confirm publication eligibility against the final `main` SHA instead:

  ```bash
  python3 scripts/release.py check-publication --sha <FINAL_MAIN_SHA>
  ```

  The `Release policy` status ran on the promotion PR's head commit,
  before the merge; the `Release` workflow's `check-publication` gate
  re-runs the version and tag checks on the final `main` SHA.


### 4. Publish

Pushing the promotion merge to `main` starts the `Release` workflow, which
publishes that exact commit:

- Verifies it is publishable (SHA is on `main`, version is new, and no
  conflicting tag exists) via `check-publication`.
- Runs the **real test suite** (with `ffmpeg` installed on Linux) and
  builds the macOS and Windows binaries **from the exact SHA that will
  be tagged**, and the Docker images natively (amd64 on an x86 runner,
  arm64 on an ARM runner) — **no QEMU cross-build**. Per-arch images are
  pushed as immutable `sha-<sha>-<arch>` staging tags first.
- The Linux release assets are the **exact bytes extracted** from those
  verified per-arch staging images (the binary is stripped once in the
  image builder); they are proven on the native runners with a static
  musl smoke, two container smokes, and an **advisory** GNU-vs-musl
  parity benchmark whose per-case report is uploaded as evidence (same
  convention as the advisory `cargo-audit` check) — it never gates
  publication, which remains ruled by the natively passing checks above.
- Only after test + binaries + Docker + Linux asset verification all
  succeed does the helper create the `vX.Y.Z` tag (via
  `scripts/release.py tag --sha`) and the minimal GitHub Release; the
  Annalist GitHub App fills the release body via webhook.
- The multi-arch `vX.Y.Z` image is assembled **only after** the per-arch
  images from the same SHA exist. The `Release` workflow **never** writes
  `latest`; CI owns it and re-points it on `main` pushes once the test job
  and both native per-arch Docker builds succeed.

### Retry / immutability semantics

- A tag that already exists **and** points at the same SHA is accepted
  (same-SHA reuse, for retry after an interrupted run).
- A tag that exists at a **different** SHA fails `check-publication`
  hard — you cannot "republish" an older version or move a tag.
- Publish runs are resumable: re-running the `Release` workflow on the
  same commit reuses the existing tag/release and only adds missing outputs.
- Runs are serialized across commits with GitHub Actions `queue: max`:
  up to 100 runs may queue, and extra runs are cancelled once the queue
  is full. Manual dispatch is main-only.
- Staging images are reused only after their Linux architecture, OCI version
  and revision labels match the prepared version and tested SHA. Version tags
  must match the captured immutable image digests; multi-arch assembly uses
  those digests, never a later mutable-tag lookup. Mismatches and registry
  authentication/transport errors fail without overwriting existing images.
- Release assets (including `m4b-merge-windows.exe`) are never overwritten.
  Missing assets are uploaded on re-run; an existing asset is reused only
  via one of two paths: a labeled asset must have a provenance label bound
  to this publication (same SHA and tag) whose digest also matches, and an
  unlabeled legacy asset is reused only when its content digest matches the
  freshly built artifact. Malformed labels and conflicting
  bindings/digests fail before any upload (see
  `scripts/publish_release.py`).
- `latest` is never rolled back: only CI writes it, on main pushes.
  Re-running the workflow for an older main commit passes only when its
  `vX.Y.Z` tag already points at that same SHA, or when its untagged
  version is still newer than every release tag reachable from main;
  `check-publication` rejects everything else.

## Styleguides
### Commit Messages
This project uses [Conventional Commits](https://www.conventionalcommits.org/en/v1.0.0/), and all PRs/contributions must use them as well.

## Join The Project Team
<!-- TODO -->

<!-- omit in toc -->
## Attribution
This guide is based on the **contributing-gen**. [Make your own](https://github.com/bttger/contributing-gen)!