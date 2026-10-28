# AGENTS.md

Development guidelines, processes, and standards for m4b-merge.

---

## Philosophy

> "First, do no harm." — Stability is paramount.

Every change must improve the codebase. Never introduce regressions, technical debt, or fragility. When in doubt, ask.

---

## Code Quality Standards

### Imperatives

1. **No new warnings** — All compiler/linter warnings must be resolved
2. **100% test coverage** — Critical paths require unit tests
3. **Typing everywhere** — No `any`, no implicit types
4. **No commented-out code** — Delete it or use feature flags
5. **No debug statements** — Remove `println!`, `console.log`, etc.

### Code Review Checklist

- [ ] **Stability** — Does this introduce race conditions, deadlocks, or panics?
- [ ] **Correctness** — Does this actually solve the stated problem?
- [ ] **Performance** — Is this O(n) when it could be O(1)? Are allocations wasteful?
- [ ] **Security** — Are inputs validated? Is sensitive data exposed?
- [ ] **Maintainability** — Will future developers understand this?
- [ ] **Edge cases** — What happens with empty inputs? Maximum values? Errors?

---

## Test-Driven Development (TDD)

### The Red-Green-Refactor Cycle

1. **Red** — Write a failing test. It must fail for the right reason.
2. **Green** — Write the minimal code to make it pass.
3. **Refactor** — Improve code without changing behavior.

### Test Requirements

Target 100% coverage on critical paths; 80%+ for I/O. Coverage is a review
target, not a CI gate — no coverage job runs in CI, so these targets are
enforced in code review.

---

## Git Workflow

### Branch Naming

| Type | Pattern | Example |
|------|---------|---------|
| Feature | `feature/[short-description]` | `feature/add-audio-normalization` |
| Bugfix | `fix/[issue-number]` | `fix/42-memory-leak-in-tagger` |
| Chore | `chore/[scope]/[description]` | `chore/deps/update-ffmpeg` |
| Hotfix | `hotfix/[critical-fix]` | `hotfix/security-patch` |

### Conventional Commits

```
<type>(<scope>): <description>

[optional body]

[optional footer]
```

#### Allowed Types

Use `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `chore`, `ci`, or `revert`. See [Conventional Commits](https://www.conventionalcommits.org/) for full spec.

### Pull Request Requirements

1. **Title** — Must follow conventional commit format
2. **Description** — What, why, and how (before/after if visual)
3. **Tests** — All tests passing, coverage maintained
4. **Linting** — No warnings, no errors
5. **Reviews** — At least 1 approval required
6. **CI Green** — All required checks must pass. The advisory `audit-advisory`
   job (`continue-on-error`) reports its own failure without gating the run.

#### PR Templates

Pull request templates belong in `.github/PULL_REQUEST_TEMPLATE.md`.

---

## CI/CD Pipeline

### Required Checks

1. **Lint & Format** — `cargo fmt --check`, `cargo clippy --all-features -- -D warnings`,
   and the release helper behavior tests
   (`python3 -m unittest discover -s scripts -p 'test_release.py'`) and
   publisher reconciliation tests
   (`python3 -m unittest discover -s scripts -p 'test_publish_release.py'`)
2. **Compile** — `cargo check --locked --all-features`
3. **Test** — `cargo test --all-features --locked`
4. **Docker** — amd64 build on PRs; arm64 builds and the multi-arch
   manifest run on main pushes only and are not PR-required checks

`cargo audit --deny warnings` runs in CI as a **visible advisory** job
(`audit-advisory`), deliberately outside the required chain — a new
advisory must not force a breaking release. Findings are reviewed manually
and never block dependency updates or merges — they are fixed in follow-up
PRs.

### Release Gate

`develop` reaches `main` only through a human-reviewed
`release/v*-*-*` promotion PR (see the `Releasing` section of
`CONTRIBUTING.md`). `main` keeps its native strict base-freshness
protection; there is no automatic sync back to `develop`. Promotion PRs
send a trusted `Release policy` status from the `Release Policy`
workflow, which verifies the frozen two-parent merge shape before the
check can be required.

---

## Dependencies

### Adding Dependencies

1. **Verify necessity** — Is there an existing solution?
2. **Check maintenance** — Last commit date? Open issues?
3. **Audit for vulnerabilities** — Run `cargo audit` (advisory — not a required check, does not block merges)
4. **Pin to version** — Use `version = "2.1.0"`, not `*`
5. **Document rationale** — Why this library?

### Dependency Review Checklist

- [ ] Is it actively maintained?
- [ ] Does it have security vulnerabilities?
- [ ] What's its dependency footprint?
- [ ] Are licenses compatible (MIT/Apache 2)?
- [ ] Can we use a lighter alternative?

---

## Error Handling

### Philosophy

Errors should be:

1. **Explicit** — No silent failures
2. **Recoverable** — When possible, retry or fallback
3. **Diagnosed** — Error messages must help debugging
4. **Typed** — Use `Result<T, E>` not `Option<T>` where meaningful

### Error Message Guidelines

Include context and actionable remediation in error messages.

---

## Performance

### Benchmarks

All hot paths must have benchmarks:

```rust
use criterion::{Criterion, criterion_group, criterion_main};

fn bench_merge_large_files(c: &mut Criterion) {
    c.benchmark_function("merge_large_files", |b| b.iter(|| merge_large_files()));
}

criterion_group!(benches, bench_merge_large_files);
criterion_main!(benches);
```

### Performance Budgets

| Operation | Max Time | Max Memory |
|-----------|----------|------------|
| Merge 1hr audiobook | 5min | 500MB |
| Metadata fetch | 2s per request | 10MB |
| Binary size | 15MB (compressed) | N/A |

---

## Security

### Secrets Management

- **Never commit secrets** — Use `.env` files, CI secrets
- **Audit commits** — `git log --all -p | grep -i password`
- **Rotate credentials** — If exposed, rotate immediately

### Dependency Security

Run regularly:
```bash
cargo audit
```

If vulnerabilities found:
1. Update if patch available
2. Fork and patch if critical
3. Replace library if unmaintained

---

## Documentation

### Code Documentation (Rustdoc)

Document all public items per [rustdoc conventions](https://doc.rust-lang.org/rustdoc/).
---

## Code Review Principles

### For Authors

1. **Self-review first** — Review your own PR before requesting review
2. **Small PRs** — <400 lines ideal, <1000 lines maximum
3. **Explain changes** — Context helps reviewers
4. **Respond to feedback** — Don't dismiss reviews

### For Reviewers

1. **Be thorough** — Don't rush reviews
2. **Be constructive** — Suggest improvements, don't just criticize
3. **Be timely** — Review within 24 hours
4. **Be specific** — Link to style guides, documentation

---

## Anti-Patterns (Never Do These)

| Pattern | Why |
|---------|-----|
| `unsafe { ... }` without comment | Security risk |
| `unwrap()` on user input | Panic on invalid input |
| `println!()` in production | Log instead |
| Global mutable state | Hard to test/debug |
| Magic numbers | Unclear intent |
| Deep nesting (>3 levels) | Hard to read |
---