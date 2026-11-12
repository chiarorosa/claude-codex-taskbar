# Strip next to the system tray (#10)

Date: 2026-11-12 · Issue: [#10](https://github.com/chiarorosa/claude-codex-taskbar/issues/10)
(taken over from @ricklaroque), related: #26, #43, #64/#71.

## Intent

On Windows 10 the strip never shows: Start and the search box take the taskbar's left
corner, so `available_width` returns `0` for every left-aligned taskbar and CCT runs with no
visible UI. A Windows 10 user gets nothing out of CCT today. The fix is a second placement:
the strip sits **inside the right-hand group, flush against the left edge of the system tray**
(tray icons, network, volume, clock), in the free space between the last app button and the
tray:

```
[Start][Search][app][app][app]      ...free...      [CCT strip][tray icons][clock][▕]
```

## Goals

1. Windows 10: the strip shows next to the tray by default.
2. The strip follows the tray: when tray icons appear or disappear (the tray's left edge
   moves), the strip moves with it. It also respects the app buttons, which grow rightwards
   toward the tray as windows open.
3. When it cannot be placed (not enough room for the full strip, or the edge lookups fail),
   it stays hidden and a Windows notification says CCT could not be placed on the taskbar for
   lack of space.
4. The placement can be forced on Windows 11 so the maintainer can validate the position on
   his own machine (he is on Windows 11 and will validate alone; no external tester for now).

## Non-goals

- No change to the strip's look (items, sizes, colors). A compact version is a future,
  separate issue.
- No Settings UI for the placement in this PR. A **Left / Right** selector for any Windows
  version is the planned follow-up PR, after this placement is validated.
- No secondary-monitor taskbars, no vertical taskbars: keep today's behavior for them.

## Anti-goals

- Covering anything: app buttons, tray icons or the clock. When in doubt, hide.
- Repeated notifications while the user opens and closes windows near the limit.

## Decisions

**User's decisions**
- Position: inside the right group, flush with the tray's left edge, following it.
- Failure: hide and show a Windows notification explaining there was no room.
- Look unchanged for now; compact version later in its own issue.
- The Windows 10 "no free corner" note (`Key::HintWindows10`) is removed only once the new
  placement is validated. In this PR it goes away for users who get the right placement;
  see *Hints* below.
- Left / Right option for any Windows comes after validation, in a follow-up PR.
- Validation on Windows 11 by forcing the placement; the maintainer validates alone.
- **A Windows 11 validation is enough** for this PR to ship `auto = right` on Windows 10,
  remove `HintWindows10` and update README / CHANGELOG / site. The Windows 10 lookups
  (`ReBarWindow32` → `MSTaskSwWClass` / `MSTaskListWClass` buttons, `TrayNotifyWnd`; any
  deskband or News and interests between apps and tray counts as occupied space) are written
  defensively and get an `#[ignore]`d live test; if they fail on a real Windows 10, the strip
  stays hidden and the settled-failure notification explains it.

**Builder-facing calls made in design (maintainer can override)**
- The placement is a real config value from the start, not a debug flag:
  `"placement"` in `config.json`, values `"auto"` (default), `"left"`, `"right"`.
  `auto` = `right` on Windows 10, `left` on Windows 11 (today's behavior, unchanged).
  To test on Windows 11: set `"placement": "right"` and restart CCT. The follow-up PR only
  adds the Settings selector on top of this value.
- Windows 11 with left-aligned icons (`TaskbarAl = 0`) under `auto` also uses `right`: it
  has the same problem as Windows 10 and this is the original subject of #10.
- The notification is for **right placement only** (explicit or resolved from `auto`).
  Left placement keeps today's silent hide, so current Windows 11 users see no change and
  `"placement": "left"` on a left-aligned taskbar does not notify on every run.
- The notification fires only on a **settled** failure, never on a transient one: either
  both edges were read and the gap is too small (confirmed no room), or the edge lookups
  failed several times in a row after startup (Explorer restarting or the taskbar still
  filling in at login must not trigger it). One generic text covers both ("could not place
  CCT on the taskbar…"). The startup decision to open Settings waits for the same settled
  state instead of the first render.
- At most **one** "no room" notification per CCT run. It does not depend on the
  near-limit alerts toggle (`notify_near_limit`), since it explains why the app is
  invisible; it uses the existing balloon path, which respects Windows quiet time.
- No room means: the layout cannot fit at least what it fits today for the left corner
  (`layout(...).width == 0` for the gap width). The existing ellipsis/drop rules of
  `layout` apply unchanged inside the gap.

## How it works

### Geometry (pure, unit-tested)

Two new edges, in screen pixels, from the taskbar query:

- `apps_right`: right edge of the last app button (the rightmost button of the running apps
  list, including pinned ones). If there are no buttons, the right edge of the left group
  (Start / search / Task View).
- `tray_left`: left edge of the notification area (the whole right group).

Free width in right mode: `tray_left - apps_right - 2 * margin`, clamped at `0`. The strip's
left position is `tray_left - margin - strip_width` (right-aligned against the tray). Any
missing edge (`None`) gives width `0`: hide. These live next to `available_width` in
`src/ui/taskbar.rs` as pure functions with tests in the style of the existing
`available_*` tests (DPI, missing edges, crowded taskbar, exact fit).

In left mode nothing changes: `available_width` and the `info.rect.left + margin` position
stay as they are.

### Edge lookups

Use documented APIs only (DESIGN.md / AGENTS.md), as the Start-button lookup does today:

- UI Automation for the app buttons' right edge; the Win32 tray child window
  (`TrayNotifyWnd` under `Shell_TrayWnd`) or its UI Automation element for `tray_left`.
- Windows 10 and Windows 11 build the taskbar differently; the lookup must handle both, and
  the builder should verify the element names/IDs on Windows 11 (available here) and
  document which ones are expected on Windows 10.
- Reuse the caching approach of `IconCache` / `StartButton` (#86): cached elements,
  periodic refresh, invalidate on taskbar change, back off on failure. Moving with the tray
  needs the edges re-read often enough that a new tray icon does not overlap the strip for
  long; the existing render cadence plus invalidation on shell changes is the starting
  point.
- Lookup failure: keep hidden (never guess a position in right mode, unlike the left-mode
  fallback estimate).

### Hints and startup

- Today startup opens Settings when `left_aligned` is true, to explain the hidden strip.
  With right placement active that is no longer the case: open Settings only if the strip
  still cannot be placed (and then the notification is also sent).
- `setup_hints` (`src/ui/view.rs`): in right mode the "center your icons" and "turn off
  Widgets" hints do not apply; `HintWindows10` is not shown when right placement works.
  The string itself is removed from `src/i18n.rs` once the maintainer has validated
  (can be in this PR if validation happens before merge).
- New i18n string for the notification, in English, pt-BR and Spanish, e.g.
  EN: "CCT could not fit on the taskbar: there is not enough free space next to the system
  tray. Close some windows or open Settings to adjust what the strip shows."

### Docs

README (FAQ "Does it work on … Windows 10?", roadmap, Tips), CHANGELOG, and the site
(`site/`) must say the strip shows next to the tray on Windows 10, once validated.

## Validation

- Unit tests: geometry functions, `auto` resolution per build/alignment, config parse of
  `placement` (unknown value → `auto`), one-notification-per-run gate.
- Manual, by the maintainer on Windows 11 with `"placement": "right"`: strip flush with the
  tray, moves when a tray icon appears/disappears, hides + one notification when many
  windows fill the taskbar, comes back when they close, DPI 100/150%, light/dark.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.

## Left to the builder

- Exact UI Automation conditions and caching cadence for the two edges.
- Whether right mode on Windows 11 places the strip left of the whole tray (including the
  Copilot/overflow chevron) — it must, to cover nothing.
- Wording polish of the new strings.
