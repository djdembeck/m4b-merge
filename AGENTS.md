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

### Test Naming Convention

```rust
#[test]
fn given_valid_input_when_processed_then_produces_expected_output() {
    // GIVEN — Set up test state
    // WHEN — Trigger the behavior
    // THEN — Assert expected outcomes
}
```

### Test Requirements

| Code Type | Coverage Required | Notes |
|-----------|------------------|-------|
| Pure functions | 100% | Easy to test, no mocks needed |
| State machines | 100% | All transitions covered |
| Error handling | 100% | All error variants tested |
| I/O operations | 80%+ | Mock external dependencies |
| Public API surfaces | 100% | Contract tests for all entry points |

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

| Type | Description | Version Bump |
|------|-------------|--------------|
| `feat` | New feature | Minor |
| `fix` | Bug fix | Patch |
| `perf` | Performance improvement | Patch |
| `refactor` | Code restructuring | None |
| `test` | Adding/modifying tests | None |
| `docs` | Documentation changes | None |
| `chore` | Maintenance tasks | None |
| `ci` | CI/CD changes | None |
| `revert` | Reverting a commit | Depends |

#### Examples

```
feat(audio): add silence detection for chapter markers

The silence detection algorithm now uses RMS with a 20ms window,
improving detection accuracy by 15% compared to the previous
amplitude-based approach.

Closes #142
```

```
fix(metadata): correct ASIN parsing for multi-disc books

When multiple ASINs were present in a single directory, only the
first was being used. Now each file is processed independently.

Fixes #287
```

```
perf(ffmpeg): reduce memory allocations during transcoding

Pre-allocate buffer for FFmpeg frame processing, eliminating
15MB/s of heap allocations during large file transcoding.

Related: #198
```

### Pull Request Requirements

1. **Title** — Must follow conventional commit format
2. **Description** — What, why, and how (before/after if visual)
3. **Tests** — All tests passing, coverage maintained
4. **Linting** — No warnings, no errors
5. **Reviews** — At least 1 approval required
6. **CI Green** — All checks must pass

#### PR Templates

```markdown
## Summary
<!-- What's being changed and why -->

## Testing
<!-- How was this tested? New tests added? -->

## Checklist
- [ ] Tests pass
- [ ] Linting clean
- [ ] Documentation updated
- [ ] Breaking changes documented
```

---

## CI/CD Pipeline

### Required Checks

1. **Compile** — `cargo check` / `make check`
2. **Test** — `cargo test --all-features`
3. **Coverage** — ≥90% coverage, no new uncovered lines
4. **Lint** — `cargo clippy --all-features -- -D warnings`
5. **Format** — `cargo fmt --check`
6. **Security** — `cargo audit --deny warnings`

### Failing CI Actions

| Check | Action |
|-------|--------|
| Compilation error | Block merge |
| Test failure | Block merge |
| Coverage drop | Block merge |
| Clippy warning | Block merge |
| Format mismatch | Block merge |
| Audit finding | Block merge + security review |

---

## Dependencies

### Adding Dependencies

1. **Verify necessity** — Is there an existing solution?
2. **Check maintenance** — Last commit date? Open issues?
3. **Audit for vulnerabilities** — Run `cargo audit`
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

```rust
// BAD: Unhelpful
Err("Failed")

// GOOD: Contextual + actionable
Err(format!(
    "Failed to parse ASIN from filename '{}': expected format [B0A-Z0-9]{{10}}",
    filename
))

// BETTER: With context + remediation
Err(format!(
    "Failed to parse ASIN from '{}': expected format [B0-9A-Z]{{10}} (e.g., B0123456789). \
     Rename file or provide ASIN via metadata.",
    filename
))
```

---

## Performance

### Benchmarks

All hot paths must have benchmarks:

```rust
#[bench]
fn bench_merge_large_files(b: &mut Bencher) {
    b.iter(|| merge_large_files());
}
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

```rust
/// Detects silence thresholds in audio streams.
///
/// Uses root-mean-square (RMS) analysis with configurable window size
/// to identify potential chapter boundaries. Windows smaller than 20ms
/// may produce unreliable results.
///
/// # Arguments
///
/// * `stream` - The audio stream to analyze
/// * `threshold_db` - Silence threshold in decibels (default: -60dB)
///
/// # Returns
///
/// A sorted list of silence positions in milliseconds.
///
/// # Errors
///
/// Returns `SilenceDetectionError` if stream is corrupted or has
/// unsupported sample rate.
///
/// # Examples
///
/// ```
/// let silence = detect_silence(&stream, -60.0)?;
/// assert!(silence.is_empty());
/// ```
pub fn detect_silence(stream: &AudioStream, threshold_db: f64) -> Result<Vec<u64>, Error>
```

### Required Documentation

| Element | Required | Location |
|---------|----------|----------|
| Public functions | Yes | Above function |
| Structs | Yes | Above struct |
| Error types | Yes | Above enum + variants |
| Modules | Yes | In `mod.rs` |
| README | Yes | Project root |

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
| Copy-paste code | Duplication bugs |

---

## References

- [Conventional Commits](https://www.conventionalcommits.org/)
- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- [Google Engineering Practices](https://google.github.io/eng-practices/)
- [Test-Driven Development in Rust](https://rafohner.medium.com/test-driven-development-in-rust-2o9o9s9e2c0a)

---

## Change Log

| Version | Date | Description |
|---------|------|-------------|
| 1.0.0 | 2025-02-09 | Initial documentation |

---

*Last updated: 2025-02-09*