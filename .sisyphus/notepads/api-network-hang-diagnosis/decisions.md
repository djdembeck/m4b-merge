# API Network Hang Diagnosis - Decisions

## Decision 1: Diagnostic Approach
**Chosen**: Add detailed tracing at every step of the request flow
**Rationale**: Need to pinpoint exactly where the hang occurs (DNS, TCP, or HTTP)
**Outcome**: Successfully identified SSL/TLS as the failure point

## Decision 2: Log Level Selection
**Chosen**: Converted diagnostic `[API-DIAG]` logs to `debug!` level after fix
**Rationale**: Keep useful debugging capability without cluttering normal info logs
**Outcome**: Clean production code with debug-level tracing available when needed

## Decision 3: Fix Implementation
**Chosen**: Add `ca-certificates` package to Docker runtime stage
**Rationale**: Minimal change that directly addresses the root cause
**Rejected Alternatives**:
- Disabling SSL verification (security risk)
- Using HTTP instead of HTTPS (security risk)
- Installing full OpenSSL (unnecessary bloat)

## Decision 4: Code Cleanup
**Chosen**: Removed verbose diagnostic logging, kept minimal `debug!` trace
**Rationale**: Production code should be clean but maintainable
**Outcome**: Single `debug!` log for request URL, no impact on normal operation
