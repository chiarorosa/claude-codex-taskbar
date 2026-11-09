# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

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

[0.3.0]: https://github.com/chiarorosa/claude-codex-taskbar/compare/v0.2.1...v0.3.0
[0.2.1]: https://github.com/chiarorosa/claude-codex-taskbar/compare/v0.2.0...v0.2.1
[0.2.0]: https://github.com/chiarorosa/claude-codex-taskbar/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/chiarorosa/claude-codex-taskbar/releases/tag/v0.1.0
