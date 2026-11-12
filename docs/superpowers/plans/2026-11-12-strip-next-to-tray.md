# Strip Next to the System Tray Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show the CCT strip flush against the system tray's left edge on Windows 10 (and on
demand on Windows 11), following the tray, and tell the user once when it cannot fit.

**Architecture:** A new `placement` config value resolves to a `Side` (left corner as today,
or right next to the tray). Right mode reads two edges from the primary taskbar (right edge
of the app buttons, left edge of the tray) through a cached lookup module, computes the gap
with pure functions, and feeds a small state machine that decides when a failure is
"settled" and when to send the single notification.

**Tech Stack:** Rust stable, `windows` crate (Win32, UI Automation), Direct2D renderer
(unchanged), serde config.

**Spec:** `docs/superpowers/specs/2026-11-12-strip-next-to-tray-design.md`

## Global Constraints

- Every change passes `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- Pure logic in testable functions; write the failing test first.
- All user-facing text through `i18n::t`, with English, pt-BR and Spanish strings.
- Documented APIs only (DESIGN.md); no new crates.
- No network, no telemetry; nothing here touches credentials.
- Never cover app buttons, tray icons or the clock: when in doubt, hide.
- Primary taskbar only; vertical taskbars and secondary monitors keep today's behavior.
- Left mode behavior must stay byte-for-byte what it is today (position, hints, silent hide).
- Conventional Commits; no AI attribution lines in commits or PR text.

## Review Focus

1. Explorer restart / `TaskbarCreated` while in right mode: cached UIA elements go stale;
   expect a silent hide, re-lookup, and the strip back without a notification. (Task 3 test
   `transient_failures_then_success_never_notify`; Task 4 drops cached elements on taskbar
   change.)
2. A new tray icon appears: the tray's left edge moves left by one icon; the strip must not
   overlap it beyond one refresh. (Task 4 invalidates the edge cache on the same events as
   `invalidate_icons`; Task 2 test `strip_left_is_flush_with_tray`.)
3. Taskbar fills up then empties: strip hides, one notification, then comes back when there
   is room, and never notifies again in that run. (Task 3 test
   `no_room_notifies_once_then_recovers_silently`.)
4. Auto-hidden taskbar slid off screen or a fullscreen app: hide exactly as today and do not
   count it as a placement failure. (Task 5 step: covered states skip `PlacementWatch`.)
5. High DPI (150%): margins in DIP converted with the taskbar DPI on both edges. (Task 2 test
   `right_gap_scales_margin_with_dpi`.)

---

### Task 1: `placement` config value and side resolution

**Files:**
- Modify: `src/config.rs` (field, default, tests)
- Modify: `src/ui/taskbar.rs` (`Side`, `resolve_side`, tests)

**Interfaces:**
- Produces: `config::Placement` (`Auto` default, `Left`, `Right`; serialized lowercase
  `"auto" | "left" | "right"`); `Config::placement: Placement`.
- Produces: `taskbar::Side { Left, Right }` and
  `pub fn resolve_side(placement: Placement, windows_10: bool, left_aligned: bool) -> Side`.

- [ ] **Step 1: Write failing tests** in `src/config.rs` tests:
  - `placement_defaults_to_auto_and_old_configs_get_it`: parsing `{}` gives `Placement::Auto`.
  - `placement_roundtrips_lowercase`: `"right"` parses to `Right`; saving writes `"placement": "right"`.
  - `unknown_placement_falls_back_to_auto_without_losing_the_rest`: `{"placement":"top","claude_enabled":true}` gives `Auto` **and** `claude_enabled == true` (a bad value must not reset the whole file).
  In `src/ui/taskbar.rs` tests, `resolve_side_table`:
  - `(Auto, windows_10=true, _)` → `Right`; `(Auto, false, left_aligned=true)` → `Right`;
    `(Auto, false, false)` → `Left`; `(Left, true, true)` → `Left`; `(Right, false, false)` → `Right`.
- [ ] **Step 2: Run** `cargo test placement resolve_side` → FAIL (types missing).
- [ ] **Step 3: Implement.** `Placement` derives `Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default`, `#[serde(rename_all = "lowercase")]`, with `#[serde(other)]` on `Auto` so unknown strings map to `Auto` (if `serde(other)` cannot be used with the derive here, use a `deserialize_with` that reads a `String` and maps unknown values to `Auto`). `resolve_side` is a three-line `match`.
- [ ] **Step 4: Run** `cargo test` → PASS.
- [ ] **Step 5: Commit** `feat(config): placement setting (auto, left, right) (#10)`.

### Task 2: Right-side geometry

**Files:**
- Modify: `src/ui/taskbar.rs` (`TaskbarInfo` fields, pure functions, tests)

**Interfaces:**
- Consumes: `Side` (Task 1).
- Produces: `TaskbarInfo { .., apps_right: Option<i32>, tray_left: Option<i32> }` (screen px; both `None` until Task 4 fills them).
- Produces: `pub fn right_gap_width(info: &TaskbarInfo, margin: i32) -> i32` and
  `pub fn right_strip_left(tray_left: i32, margin: i32, strip_width: i32) -> i32`.

- [ ] **Step 1: Write failing tests** (extend the existing `info()` test helper with the two new fields):
  - `right_gap_is_space_between_apps_and_tray`: `apps_right=Some(900), tray_left=Some(1600)`, margin 8 → `684`.
  - `right_gap_is_zero_when_an_edge_is_missing`: either edge `None` → `0`.
  - `right_gap_is_zero_when_crowded`: `apps_right=Some(1595), tray_left=Some(1600)` → `0` (never negative).
  - `right_gap_scales_margin_with_dpi`: the caller passes `to_px(MARGIN_DIP, 144)`; with `apps_right=Some(900), tray_left=Some(1600)` the result is `700 - 2 * to_px(MARGIN_DIP, 144)`.
  - `strip_left_is_flush_with_tray`: `right_strip_left(1600, 8, 300) == 1292`.
- [ ] **Step 2: Run** `cargo test right_` → FAIL.
- [ ] **Step 3: Implement** both functions; add the two fields to `TaskbarInfo` and set them to `None` in `query` for now.
- [ ] **Step 4: Run** `cargo test` → PASS.
- [ ] **Step 5: Commit** `feat(taskbar): right-side gap geometry (#10)`.

### Task 3: Placement watch (settled failure + one notification)

**Files:**
- Create: `src/ui/placement.rs` (pure state machine + tests); register in `src/ui/mod.rs`.

**Interfaces:**
- Produces:
  ```rust
  pub enum Outcome { Placed, NoRoom, LookupFailed }
  pub struct PlacementWatch { /* consecutive failures, settled flag, notified flag */ }
  pub struct Verdict { pub settled_failure: bool, pub notify: bool }
  impl PlacementWatch { pub fn observe(&mut self, outcome: Outcome) -> Verdict; pub fn settled_failure(&self) -> bool }
  pub const LOOKUP_FAILS_TO_SETTLE: u32 = 3;
  ```
  Rules: `NoRoom` (both edges read, gap too small) settles immediately. `LookupFailed`
  settles only after `LOOKUP_FAILS_TO_SETTLE` consecutive observations. `Placed` clears
  the failure count and the settled flag. `notify` is true at most once per `PlacementWatch`
  lifetime (one CCT run), on the first transition into a settled failure.

- [ ] **Step 1: Write failing tests:**
  - `no_room_settles_and_notifies_immediately`.
  - `lookup_failures_settle_after_three_in_a_row`: two `LookupFailed` → not settled, no notify; third → settled + notify.
  - `transient_failures_then_success_never_notify`: `LookupFailed, LookupFailed, Placed, LookupFailed, LookupFailed` → `notify` never true.
  - `no_room_notifies_once_then_recovers_silently`: `NoRoom` (notify) → `Placed` (not settled) → `NoRoom` (settled, **no** notify).
  - `staying_settled_does_not_repeat_notify`: `NoRoom, NoRoom, NoRoom` → notify only on the first.
- [ ] **Step 2: Run** `cargo test placement::` → FAIL.
- [ ] **Step 3: Implement** `PlacementWatch` (derive `Default, Debug`).
- [ ] **Step 4: Run** `cargo test` → PASS.
- [ ] **Step 5: Commit** `feat(ui): placement watch for settled failures (#10)`.

### Task 4: Edge lookups (Windows 10 and Windows 11)

**Files:**
- Create: `src/ui/taskbar/edges.rs` (lookup + cache); add `mod edges;` in `src/ui/taskbar.rs`.
- Modify: `src/ui/taskbar.rs` `query` signature.

**Interfaces:**
- Consumes: `TaskbarInfo.apps_right / tray_left` (Task 2).
- Produces: `pub fn query(with_icons: bool, placement: Placement) -> Option<TaskbarInfo>`;
  it sets the new field `TaskbarInfo.side = resolve_side(placement, windows_10, left_aligned)`
  (both inputs are cheap: build number and registry) and reads `apps_right` / `tray_left`
  only when `side == Side::Right`, otherwise leaves them `None`.
- Produces: `pub fn invalidate_edges()` (drops cached elements; called wherever
  `invalidate_icons` is called today).

- [ ] **Step 1: Write the failing pure test** in `edges.rs`: `apps_right_is_max_right_of_buttons`
  — `fn apps_right_of(rects: &[Rect], left_group_right: Option<i32>) -> Option<i32>` returns the
  largest `right` of non-empty rects, falling back to `left_group_right` when there are none, and
  `None` when both are missing. Also `zero_width_rects_are_ignored`.
- [ ] **Step 2: Run** `cargo test edges::` → FAIL.
- [ ] **Step 3: Implement the lookups.**
  - `tray_left`: `FindWindowExW(Shell_TrayWnd, None, "TrayNotifyWnd", None)` + `GetWindowRect`, left edge. On Windows 11 verify on this machine that this rect covers the whole visible right group (chevron, Copilot if present, tray icons, clock); if it does not, read the left edge of the tray's UI Automation element instead and note which element in a comment.
  - `apps_right`: UI Automation, reusing the `StartButton` caching pattern (#86: cached client and elements, warm reads, full find every `FULL_FIND_EVERY`, drop client on failure). Collect the bounding rects of the app buttons and pass them to `apps_right_of`. Windows 10: buttons under `MSTaskListWClass` (inside `ReBarWindow32` → `MSTaskSwWClass`); any other band between apps and tray (deskband, toolbar, News and interests) counts as occupied, i.e. take the max right of all `ReBarWindow32` children left of the tray. Windows 11: the taskbar's app-button list elements. Comment the expected IDs/classes for each OS.
  - Any failure returns `None` for that edge (never a guess).
  - Add `#[ignore]` live test `live_edges_are_ordered`: on the real taskbar, both edges are `Some` and `apps_right < tray_left` within the taskbar rect.
- [ ] **Step 4: Run** `cargo test` → PASS; then `cargo test -- --ignored live_edges` on this machine → PASS.
- [ ] **Step 5: Commit** `feat(taskbar): read app and tray edges for right placement (#10)`.

### Task 5: Render wiring, notification and startup

**Files:**
- Modify: `src/ui/app.rs` (`render`, `taskbar_hints`, new `PlacementWatch` field, pending-notice accessor)
- Modify: `src/ui/strip.rs` (`render` passes `with_edges`; startup Settings decision; call `taskbar::invalidate_edges` next to `invalidate_icons`)
- Modify: `src/ui/strip/notifications.rs` (send the placement notice)
- Modify: `src/i18n.rs` (`Key::NotifyPlacementTitle`, `Key::NotifyPlacementBody`)

**Interfaces:**
- Consumes: `resolve_side`, `right_gap_width`, `right_strip_left`, `PlacementWatch`, `query(_, with_edges)`.
- Produces: `App::take_placement_notice(&mut self) -> bool`, `App::placement_failed(&self) -> bool` (settled).

- [ ] **Step 1: Write failing tests** (pure helpers in `app.rs` or `placement.rs`):
  - `outcome_of(side, info, layout_width)` → `None` for `Side::Left` (left mode never feeds the watch); `Some(LookupFailed)` when an edge is `None`; `Some(NoRoom)` when edges are present and `layout_width == 0`; `Some(Placed)` otherwise.
  - i18n: the existing every-key-has-every-language test covers the two new keys once they are in `Key::ALL` (or the equivalent list).
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement.**
  - `strip::render`: pass `app.cfg.placement` to `taskbar::query`; the side comes back in `info.side`.
  - `App::render`: in right mode use `right_gap_width` as `available` and `(right_strip_left(tray_left, margin, l.width), info.rect.top)` as position; `FrameKey` must include the position (or `tray_left`) so a moved tray repaints. Covered states (auto-hide slid away, fullscreen) hide as today and **do not** call `observe`. Otherwise feed `outcome_of(...)` to `PlacementWatch`; store `notify` as a pending flag.
  - `strip/notifications.rs`: `drain_placement(hwnd)` sends one balloon with `NotifyPlacementTitle` / `NotifyPlacementBody` through `send_balloon`, with a `current` check that does **not** require `cfg.notify_near_limit` (keep the fullscreen and life checks). Call it after `render` in the same places `drain` runs.
  - Startup (`strip.rs` ~L532): replace `if left_aligned { settings::open }` with: left mode → unchanged; right mode → open Settings the first time `placement_failed()` becomes true (alongside the notification), not after the first render.
  - Strings (exact):
    - `NotifyPlacementTitle`: `"CCT — Taskbar"`, `"CCT — Barra de tarefas"`, `"CCT — Barra de tareas"`.
    - `NotifyPlacementBody`: `"CCT could not fit on the taskbar: there is not enough free space next to the system tray. Close some windows or open Settings to choose fewer items."`, `"O CCT não coube na barra de tarefas: não há espaço livre suficiente ao lado da bandeja do sistema. Feche algumas janelas ou abra as Configurações para escolher menos itens."`, `"CCT no cabe en la barra de tareas: no hay suficiente espacio libre junto a la bandeja del sistema. Cierra algunas ventanas o abre Configuración para elegir menos elementos."`
- [ ] **Step 4: Run** full gate (`cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`) → PASS; `cargo build --release`.
- [ ] **Step 5: Commit** `feat(ui): place the strip next to the tray on Windows 10 (#10)`.

### Task 6: Hints

**Files:**
- Modify: `src/ui/view.rs` (`TaskbarHints`, `setup_hints`, tests), `src/ui/app.rs` (`taskbar_hints`)

**Interfaces:**
- Consumes: `TaskbarInfo.side` (Task 4).
- Produces: `TaskbarHints { .., side: Side }` (default `Left`), filled from `info.side` in `App::taskbar_hints`.

- [ ] **Step 1: Update/write tests:**
  - `right_side_has_no_setup_hints`: `side: Right` with `left_aligned: true, windows_10: true, widgets_button_on: true` → empty.
  - Existing left-side tests keep their expectations, except Windows 10 cases now carry `side: Right` (via `taskbar_hints`) and expect empty.
- [ ] **Step 2: Run** → FAIL.
- [ ] **Step 3: Implement:** `setup_hints` returns empty for `Side::Right`; left side unchanged.
- [ ] **Step 4: Run** full gate → PASS.
- [ ] **Step 5: Commit** `feat(ui): no taskbar hints when the strip sits next to the tray (#10)`.

### Task 7: Manual validation by the maintainer, then cleanup and docs

**Files:**
- Modify: `src/i18n.rs`, `src/ui/view.rs` (remove `Key::HintWindows10` and its tests' references)
- Modify: `README.md` (FAQ "Does it work on macOS, Linux or Windows 10?", roadmap line for #10, Tips), `CHANGELOG.md` (Unreleased), `site/pages/index.html` (FAQ at ~L137, requirements at ~L97), `site/pages/claude-code-usage-windows.html` (~L40)

- [ ] **Step 1: Hand the release build to the maintainer** with `"placement": "right"` instructions (`%APPDATA%\claude-codex-taskbar\config.json`, restart CCT) and the checklist from the spec's *Validation* section: flush with tray; moves when a tray icon appears/disappears; many windows → hides + one notification + Settings opens; closing windows → back; DPI 100/150%; light/dark. **Stop here until the maintainer confirms.**
- [ ] **Step 2: Remove `HintWindows10`** (key, three strings, `Key::ALL` entry, test references). Run full gate → PASS.
- [ ] **Step 3: Docs.** README FAQ: "On Windows 10 the strip sits next to the system tray." Roadmap: drop the #10 line. CHANGELOG Unreleased → Added: "Windows 10: the strip now shows next to the system tray, and follows it (#10). Set `"placement": "right"` in `config.json` to use the same position on Windows 11." Site: same facts as README (`python site/build.py` + `python -I site/test_build.py` pass).
- [ ] **Step 4: Commit** `docs: Windows 10 shows the strip next to the tray (#10)`.
