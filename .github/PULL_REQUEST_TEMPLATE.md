<!--
Thanks for contributing! Please keep PRs small and focused.
Title format: Conventional Commits, e.g. "fix(claude): treat 401 as expired".
-->

## Summary

<!-- What does this PR change, and why? -->

Closes #

## Type of change

- [ ] Bug fix
- [ ] New feature
- [ ] Refactor (no behavior change)
- [ ] Performance
- [ ] Documentation
- [ ] Build / tooling

## How was this tested?

<!-- Describe manual and automated testing. For UI changes, include screenshots. -->

- Windows version/build:
- Display scaling (DPI) and monitor setup:
- Taskbar alignment (center/left) and auto-hide:

## Checklist

- [ ] `cargo fmt --all -- --check` passes
- [ ] `cargo clippy --all-targets -- -D warnings` passes
- [ ] `cargo test` passes
- [ ] New parsing/logic code has unit tests (fixtures use fake tokens only)
- [ ] No credentials, tokens, or account IDs are logged, persisted, or sent anywhere other than the provider's own endpoint
- [ ] Credential files are still only read, never written (outside `src/limits/switcher/`)
- [ ] No telemetry or new network destinations added
- [ ] New dependencies (if any) are justified below and keep the binary small
- [ ] Docs (README, CONTRIBUTING, etc.) updated if behavior changed

## Notes for reviewers

<!-- Anything that needs special attention, trade-offs, follow-ups. -->
