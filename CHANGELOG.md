# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- **Show remaining instead of used (opt-in).** A Settings switch that shows what is left of each
  limit (`38%` instead of `62%`, `38% left` in the tooltip). Bar colors still follow usage, and
  errors still show `—` (#31, #80, thanks @DiulioAires).

### Changed
- **Right-click menu.** Items are grouped (Refresh now and Copy usage; Claude account;
  Settings… and Start with Windows; Exit), Refresh now is the bold default like the
  double-click, and every item has a one-letter shortcut shown on its right. The letters are
  the same in every language (#58).

### Fixed
- **The strip no longer covers its own right-click menu** a few seconds after it opens over
  the strip.
- **Keep-alive no longer kills what the CLI leaves running on success.** The process tree is
  still killed when the CLI times out (#75, #81, thanks @FramY2).

## [0.4.0] - 2026-11-09

### Added
- **Claude account switcher.** Save your paid Claude Code logins (Pro, Max, Team, Enterprise)
  and switch between them from the right-click menu (**Claude account**), without `/logout` and
  `/login` (#53, #54, #55, #56, #57).
  - Claude Code picks up the new login on its next request; no restart is needed.
  - Always manual: the app never switches on its own and never rotates accounts.
  - Asks first when a Claude Code task is running or waiting for your answer (for example on
    a permission prompt), because the rest of that task would run on the other account.
  - Each entry shows the plan and the usage last seen on that account, and warns when a saved
    login is about to expire (`⚠ login expires in Nd`) or has expired (`⚠ needs /login`).
  - Saved logins are encrypted with Windows DPAPI for your user. A switch holds Claude Code's
    own lock files, backs up the account being left, writes through a temp file, and restores
    everything if any step fails.
  - Applies to the `claude` CLI only: the Claude desktop app keeps its own login. Not supported
    when `CLAUDE_CONFIG_DIR` is set.
- **Account labels on the strip.** A short label (up to 5 characters, default `P`) before the
  Claude and the Codex gauges, as a badge in each service's color, tells you which account the numbers
  belong to, e.g. `P` for personal and `W` for work. Each saved Claude account has its own
  label, so a switch changes it too. Edit it in Settings; leave it empty to hide it (#74).
- **Quota alerts (opt-in).** A Windows notification when fresh Claude or Codex usage crosses
  80% or 95% of a window (thresholds in `config.json`), showing the highest threshold crossed
  per window. Off by default: turn it on in Settings (**Notify near limit**). It never replays
  old crossings, waits for a real reset before warning again, and stays quiet in full screen,
  presentations and quiet hours (#15, #73, #76, thanks @gnevesx).

### Fixed
- Codex and *Keep login active*: when the `codex app-server` fallback or a login status
  command times out, the whole process tree is now killed, so a `.cmd` shim can no longer
  leave `node` or the CLI running in the background (#62, #75, thanks @FramY2).
- Windows 10: Settings and the tooltip now explain why the strip stays hidden (there is no
  free taskbar corner there) instead of showing nothing (#64, #71, thanks @Daiyuanranfeng;
  confirmed by @pedrohassen).

### Changed
- **Settings window redesigned** in the style of the Windows 10/11 Settings app: cards,
  on/off switches, item chips and clear login status lines. It follows the Windows light or
  dark app theme and your accent color, live, uses system colors in High Contrast, adapts to
  each monitor's scaling, and scrolls when the screen is short. Changes apply immediately (no
  OK/Cancel; Esc closes). Switches and chips are real checkboxes underneath, so the keyboard
  and Narrator keep working. Design rules are in the new `DESIGN.md` (#58).
- Changing visible items or the hardware interval no longer triggers a limits fetch.
- Codex's color is now blue (`#3b82f6`) instead of green, on the strip and in Settings.
- Privacy: the account switcher is the one exception to read-only credentials. Only when you
  pick an account to switch to does the app write `~/.claude/.credentials.json` and the
  `oauthAccount` key of `~/.claude.json`; *Save current account* only reads them. The Codex login
  is never written, and tokens are never refreshed in-app. README, SECURITY.md and AGENTS.md
  are updated.

## [0.3.0] - 2026-11-09

### Added
- Codex: when the usage endpoint fails, live limits are read from the installed Codex CLI
  (`codex app-server`, hidden, 5 s timeout) before falling back to the session logs (#25, #60,
  thanks @FramY2).
- Tooltip shows the time left until each limit resets (#30, thanks @gnevesx).
- Tooltip says why the last update failed when the data shown is stale (#49).
- Limits refresh right after a known reset time passes (#32).
- *Copy usage* in the right-click menu (#33).
- Double-click the strip to refresh now (#34).

### Fixed
- Windows 10: the strip no longer covers the Start button and the search box (#43, reported by
  @pedrohassen).
- Windows 10: no more setup hints for taskbar settings that only exist on Windows 11 (centered
  icons, Widgets button) (#44, #61, thanks @fiower-grass; reported by @pedrohassen).
- GPU shows `—` instead of `0%` when its counter has no valid data (#1, #9, thanks @ricklaroque).
- Hidden disks stay hidden when Windows re-enumerates the disks (#4, #41, thanks @DiulioAires).
- The strip hides when painting fails instead of keeping a stale frame (#2, #42, thanks
  @pedrohassen).
- *Keep login active* (experimental): `claude auth status` now runs under a hidden pseudo
  console, so Claude Code treats it as interactive and can renew an expired login, as it does
  in a console window (#52, #59, thanks @jasestu for the report and the analysis). Both CLIs
  are now killed after 60 s. Renewal of an already-expired token is still being verified; help
  wanted in #65.

### Changed
- Privacy: besides *Keep login active*, CCT now also runs a program when the Codex usage
  endpoint fails: the local `codex app-server`, hidden, with its output used only for the
  limits. The README's Privacy section is updated.

### Documentation
- Code signing policy for the SignPath Foundation application (`CODE_SIGNING.md`), and notes
  on the pending winget package.

## [0.2.1] - 2026-11-04

### Added
- Claude tooltip shows per-model weekly limits (Opus, Sonnet) and extra usage when the account
  has them (#24, thanks @gnevesx).

### Fixed
- Codex: the weekly limit no longer shows as the 5-hour one; windows are classified by their
  reported duration (#22, thanks @gnevesx).
- HTTP requests refuse plain HTTP and never follow redirects, so headers cannot leak (#7).
- *Keep login active* runs the CLI by absolute path from `System32` instead of `cmd /C` (#6).

### Changed
- Executable is ~135 KB smaller (about 800 KB): the Universal CRT comes from Windows and the
  build is optimized for size.

### Security
- Releases carry signed build provenance attestations; see *Verifying a release* in
  `SECURITY.md` (#19).
- CI and release builds use Actions pinned by commit SHA and `--locked` dependencies (#8), and
  fail if the executable exceeds 1 MB (#18).

## [0.2.0] - 2026-10-30

### Changed
- Renamed to **CCT (claude-codex-taskbar)**: the executable is now `claude-codex-taskbar.exe`.
- Windows 10 is supported (validated), in addition to Windows 11.

### Added
- Automatic upgrade from v0.1.x: closes the old `widget-usage-windows.exe`, copies its settings
  and moves *Start with Windows* to the new executable.
- Experimental, opt-in *Keep login active* for Codex (`codex login status`), alongside Claude Code.

## [0.1.0] - 2026-10-30

### Added
- First release: taskbar strip with CPU, RAM, GPU and per-disk activity, Claude Code and Codex
  5-hour/weekly limits, tooltip with reset times, settings window, start with Windows,
  English and Brazilian Portuguese UI.

[Unreleased]: https://github.com/chiarorosa/claude-codex-taskbar/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/chiarorosa/claude-codex-taskbar/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/chiarorosa/claude-codex-taskbar/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/chiarorosa/claude-codex-taskbar/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/chiarorosa/claude-codex-taskbar/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/chiarorosa/claude-codex-taskbar/releases/tag/v0.1.0
