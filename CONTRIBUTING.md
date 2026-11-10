# Contributing

Thanks for your interest in improving CCT (claude-codex-taskbar)! Everyone is welcome, whether you are fixing a typo, reporting a bug, reviewing the design, or writing code.

By participating, you agree to follow our [Code of Conduct](CODE_OF_CONDUCT.md).

## Project status

The project is in **early development (v0.2)**: the app works, and the most valuable contributions right now are bug reports from different setups (monitors, DPI, taskbar settings), feedback on the experimental *Keep login active* option, and well-described ideas. Please discuss larger changes in an issue first.

## Ways to contribute

- **Report a bug**: open a [bug report](../../issues/new?template=bug_report.yml). Include your Windows build, taskbar alignment, display scaling (DPI), and monitor setup; taskbar widgets are very sensitive to these.
- **Propose a feature**: see [Proposing features](#proposing-features).
- **Improve docs**: small doc fixes can go straight to a pull request.
- **Improve the website**: the [project site](https://chiarorosa.github.io/claude-codex-taskbar/) lives in `site/` (plain HTML pages in `site/pages/`, one CSS file and a Python build with no dependencies). Run `python site/build.py --offline` to build it into `_site/` and `python -m unittest discover -s site` to test it. It is published by `.github/workflows/pages.yml` and rebuilt after every release.
- **Write code**: pick an issue labeled `good first issue` or `help wanted`, comment that you are working on it, and open a PR.
- **Security issues**: do **not** open a public issue. Follow [SECURITY.md](SECURITY.md).

## Development setup

The app targets **Windows 11 and Windows 10** and is written in Rust using raw Win32 (the [`windows`](https://crates.io/crates/windows) crate) with Direct2D/DirectWrite.

1. Install the **Rust stable** toolchain via [rustup](https://rustup.rs/).
2. Use the **MSVC** target:

   ```powershell
   rustup default stable-x86_64-pc-windows-msvc
   rustup target add x86_64-pc-windows-msvc
   rustup component add rustfmt clippy
   ```

3. Install the **Visual Studio Build Tools** (or Visual Studio) with the *Desktop development with C++* workload, which provides the MSVC linker and the Windows SDK.
4. Build and run:

   ```powershell
   cargo build
   cargo run
   ```

No other runtime (.NET, WebView2, VC++ redistributable) is needed or allowed as a dependency: the release binary must be a single self-contained `.exe`.

## Before you open a pull request

Every PR is expected to pass these locally (CI will enforce them once it exists):

```powershell
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Guidelines:

- **Keep it light.** This app's main promise is a tiny binary and near-zero idle CPU. Justify any new dependency in the PR description, and prefer the standard library or the `windows` crate.
- **Test what can be tested.** Parsers for credentials files and endpoint responses must have unit tests with fixtures. Fixtures must never contain real tokens; use obviously fake values.
- **Never log credentials.** Do not print, log, or include tokens, account IDs, or raw credential file contents in errors, panics, debug output, or tests. See [SECURITY.md](SECURITY.md) for the full rules.
- **Credentials stay read-only.** Never write to `~/.claude` or `~/.codex`, and never refresh tokens. The only exception is the Claude account switcher in `src/limits/switcher/` (see AGENTS.md and #53).
- **No telemetry.** Network access is limited to the providers' own usage endpoints.
- **Show errors honestly.** Never display `0%` or stale data as if it were current when a fetch fails.

## Commits

We use [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(optional scope): <short summary in the imperative mood>
```

Common types: `feat`, `fix`, `docs`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`. Examples:

```
feat(disk): show one item per physical disk
fix(claude): treat 401 as expired instead of 0%
docs: clarify Start with Windows behavior
```

Write commit messages in English.

## Pull requests

- **Small, focused PRs** are much easier to review. One logical change per PR; split refactors from behavior changes.
- Link the related issue (`Closes #123`).
- Describe what changed, why, and how you tested it. For visual changes, include a screenshot and your display scaling.
- Fill in the [PR template](.github/PULL_REQUEST_TEMPLATE.md) checklist.
- Be patient and kind in reviews; maintainers are volunteers.

## Proposing features

1. Search existing issues first to avoid duplicates.
2. Open a [feature request](../../issues/new?template=feature_request.yml) describing the **problem** you want solved, not only the solution.
3. Consider the project's constraints: lightweight, single `.exe`, Windows 10/11 only, no telemetry, read-only credentials.
4. For larger changes, wait for a maintainer to agree on the approach before writing code. This avoids wasted effort on PRs that cannot be merged.

## Language

All repository content (code, comments, docs, issues, commits, PRs) should be in English so that anyone can participate. The original design document is an exception and is kept in Portuguese.

## License

By contributing, you agree that your contributions will be licensed under the [MIT License](LICENSE).
