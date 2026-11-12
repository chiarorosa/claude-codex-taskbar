# Strip Position Selector Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Left / Right selector in Settings → General; the choice is a preference with
fallback to the other side by free space, hiding when neither fits, with a notice whenever an
explicit choice is not honored.

**Architecture:** Pure side resolution (`preference order` × `which sides fit`) replaces the
version-based `resolve_side`. The placement watch is reworked from "one settled failure per
run" to "one notice per settled non-preferred state". The Settings window gets a radio-chip
row writing `cfg.placement`.

**Tech Stack:** Rust stable, `windows` crate (Win32 controls, UI Automation), Direct2D.

**Spec:** `docs/superpowers/specs/2026-11-12-placement-selector-design.md`

## Global Constraints

- Gate: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- Pure logic in testable functions; failing test first.
- All user-facing text through `i18n::t` (English, pt-BR, Spanish), exact strings from the spec.
- DESIGN.md: real Win32 controls with our paint, documented APIs, no new crates.
- Never cover taskbar icons; never show `0%` for errors.
- Config values stay `auto | left | right`; `auto` = Left → Right with **silent** fallback.
- Conventional Commits; no attribution trailers.

## Review Focus

1. Side ping-pong while windows open/close near the limit: settle rule must keep it to at most one notice per settled state. (Task 2 test `fallback_then_back_then_fallback_notifies_twice_only_when_settled`.)
2. A user who never touched the selector on Windows 10: strip goes right with **no** notice. (Task 2 test `auto_fallback_is_silent`.)
3. Changing the selector while the strip is on the fallback side or hidden: the new preference applies on the next render, and the watch starts over so no stale notice fires. (Task 3 step: `PlacementWatch::reset` on placement change.)
4. Explorer restart while on the fallback side: grace period, no spurious notice. (Existing `restart()` semantics kept; Task 2 test `restart_opens_new_grace`.)
5. Keyboard: arrow keys move between Left/Right radios and apply immediately. (Task 4 manual check + model test `selector_writes_explicit_placement`.)

---

### Task 1: Side resolution by preference and room

**Files:** Modify `src/ui/taskbar.rs` (replace `resolve_side`, tests).

**Interfaces:**
- Produces: `pub fn side_order(placement: Placement) -> [Side; 2]` (`Auto|Left` → `[Left, Right]`, `Right` → `[Right, Left]`).
- Produces: `pub enum Placed { Preferred(Side), Fallback(Side), Hidden }` and
  `pub fn place(order: [Side; 2], fits: impl Fn(Side) -> bool) -> Placed`.
- Produces: `pub fn is_explicit(placement: Placement) -> bool` (false only for `Auto`).

- [ ] **Step 1: Failing tests** `place_table`: for each order and every fits combination: `[Left,Right]` + (L✓) → `Preferred(Left)`; (L✗,R✓) → `Fallback(Right)`; (L✗,R✗) → `Hidden`; `[Right,Left]` + (R✓) → `Preferred(Right)`; (R✗,L✓) → `Fallback(Left)`; both ✗ → `Hidden`. `side_order_table`, `auto_is_not_explicit`.
- [ ] **Step 2:** run → FAIL.
- [ ] **Step 3:** implement; remove `resolve_side` and its test; keep `TaskbarInfo.side` (now set by the caller in Task 3, see there).
- [ ] **Step 4:** gate → PASS. **Step 5:** commit `feat(taskbar): resolve the strip side by preference and free space`.

### Task 2: Placement watch per settled state

**Files:** Modify `src/ui/placement.rs`.

**Interfaces:**
- Consumes: `Placed`, `Side` (Task 1).
- Produces:
  ```rust
  pub enum NoticeKind { MovedRight, MovedLeft, Hidden }
  pub struct Notice { pub kind: NoticeKind, pub open_settings: bool }
  impl PlacementWatch {
      pub fn observe(&mut self, placed: Placed, explicit: bool, now_secs: u64);
      pub fn pending_notice(&self) -> Option<Notice>;
      pub fn notice_sent(&mut self);
      pub fn restart(&mut self, now_secs: u64);   // Explorer restart: new grace, keep sent history
      pub fn reset(&mut self, now_secs: u64);     // placement changed in Settings: forget state, new grace
      pub fn hidden_settled(&self) -> bool;        // replaces settled_failure()
  }
  ```
  Rules: a non-preferred state (`Fallback(_)` or `Hidden`) must hold `SETTLE_SECS` (outside the `GRACE_SECS` grace) to settle; settling a state different from the last notified one raises one pending notice (`Fallback(Right)` → `MovedRight`, `Fallback(Left)` → `MovedLeft`, `Hidden` → `Hidden`); `Preferred(_)` clears the current state, the pending notice and the "last notified" memory (so a later fallback notifies again). With `explicit == false`, `Fallback(_)` never notifies (`Hidden` still does). `open_settings` is true only for a `Hidden` notice settled before the strip was ever shown this run. `pending_notice` returns the notice only while its state is still the settled one. `outcome_of` is replaced by the caller passing `Placed` directly (vertical taskbars: caller passes nothing, see Task 3).

- [ ] **Step 1: Failing tests** (rewrite the module's tests to the new API, keeping the grace/settle/restart ones): `fallback_settles_after_10s_and_notifies_moved_right`, `auto_fallback_is_silent` (explicit=false, Fallback(Right) 60 s → no notice; Hidden → notice), `return_to_preferred_is_silent_and_clears`, `fallback_then_back_then_fallback_notifies_twice_only_when_settled`, `fallback_to_hidden_notifies_again`, `staying_in_state_does_not_repeat`, `hidden_at_startup_opens_settings_runtime_does_not`, `restart_opens_new_grace`, `reset_forgets_state`.
- [ ] **Step 2:** run → FAIL. **Step 3:** implement. **Step 4:** gate → PASS.
- [ ] **Step 5:** commit `feat(ui): one placement notice per settled state, explicit choices only`.

### Task 3: Render wiring and notices

**Files:** Modify `src/ui/app.rs`, `src/ui/strip.rs`, `src/ui/strip/notifications.rs`, `src/ui/taskbar.rs` (`query`), `src/i18n.rs`.

**Interfaces:**
- Consumes: Tasks 1–2.
- Produces: `taskbar::query(with_icons: bool, with_apps: bool, with_edges: bool) -> Option<TaskbarInfo>` (edges read only when `with_edges` and the bar is horizontal); `App::pending_placement_notice() -> Option<Notice>`; i18n keys `NotifyMovedRight`, `NotifyMovedLeft` (+ reworded `NotifyPlacementBody`), exact texts in the spec.

- [ ] **Step 1: Failing tests** (pure helper in `src/ui/placement.rs` or `app.rs`): `fn wants_edges(placement: Placement, left_fitted_last: Option<bool>, left_aligned: bool) -> bool` — true when order starts with Right, or Left did not fit at the last render, or (no render yet and `left_aligned`). Table test `wants_edges_table`.
- [ ] **Step 2:** run → FAIL.
- [ ] **Step 3: Implement.**
  - `strip::render` passes `with_edges = wants_edges(cfg.placement, app.left_fitted_last, …)` (`left_aligned` from the last `TaskbarInfo`, false when none).
  - `App::render`: compute the left layout (`available_width`) and, when edges are present on a horizontal bar, the right layout (`right_gap_width`); `fits(side) = layout width > 0` (right without edges = false); `place(side_order(cfg.placement), fits)`; draw the chosen side (`Preferred`/`Fallback`) at its position (left: `rect.left + margin`; right: `right_strip_left`), hide on `Hidden`. Store `left_fitted_last`. Vertical bar: left logic only, no `observe`. Covered (fullscreen/auto-hide) and all-items-off: hide and no `observe`, as before. Otherwise `observe(placed, is_explicit(cfg.placement), now)`. `info.side` = drawn side (for hints/FrameKey).
  - `App::apply_config`: when `placement` changed, `placement.reset(now)`, invalidate frame and edges.
  - Notices: `strip/notifications.rs` sends the body for the notice kind (`NotifyMovedRight` / `NotifyMovedLeft` / `NotifyPlacementBody`) with `NotifyPlacementTitle`; Settings auto-opens only when `open_settings`; click opens Settings (unchanged). Keep fullscreen/closing/shell gating.
  - Remove the startup `left_side_blocked()` Settings opening (Left now falls back); if the old hints for left-aligned taskbars become unreachable, leave `setup_hints` unchanged (Task 4 handles hints).
- [ ] **Step 4:** gate → PASS; `cargo build --release`; `cargo test -- --ignored live_edges`.
- [ ] **Step 5:** commit `feat(ui): fall back to the other side and notify when an explicit choice is not honored`.

### Task 4: Settings selector

**Files:** Modify `src/ui/settings.rs`, `src/ui/settings/layout.rs`, `src/ui/settings/model.rs`, the Settings painter in `src/ui/settings/`, `src/i18n.rs`, `src/ui/view.rs` (hints).

**Interfaces:**
- Produces: `Row::Choice { title: String, options: Vec<(usize, String, bool)> }` (id, label, selected) in the General card right after "Start with Windows"; model fn `pub fn with_placement(cfg: &Config, side: Side) -> Config` writing `Placement::Left` / `Placement::Right`; i18n keys `SettingsPlacement`, `PlacementLeft`, `PlacementRight` (exact texts in the spec). Selected option: `Right` iff `cfg.placement == Right`, else `Left`.

- [ ] **Step 1: Failing tests:** layout test `choice_row_sits_right_like_chips` (two option rects right-aligned in the row, chip tokens 11×4 padding, 6 gap); model tests `selector_writes_explicit_placement` (Auto + pick Left → `Left`; pick Right → `Right`; other fields unchanged), `auto_shows_left_selected`.
- [ ] **Step 2:** run → FAIL.
- [ ] **Step 3: Implement.** Real `BUTTON` controls `BS_AUTORADIOBUTTON` (first with `WS_GROUP | WS_TABSTOP`), painted like chips via the existing custom-draw path (accent fill when checked, focus ring per DESIGN.md). `BN_CLICKED` → `with_placement` → existing apply path (config saved, strip re-rendered at once). `setup_hints`: drop `HintCenterTaskbar` (Left now falls back to the right, so centering is no longer required); keep `HintWidgetsButton` only when the strip is drawn on the left.
- [ ] **Step 4:** gate → PASS; `cargo build --release`.
- [ ] **Step 5:** commit `feat(settings): Left / Right strip position selector`.

### Task 5: Docs

**Files:** `README.md`, `CHANGELOG.md` (Unreleased), `site/pages/index.html`, `site/pages/pt-br/index.html`, `site/pages/claude-code-usage-windows.html`, `site/pages/pt-br/uso-claude-code-windows.html`.

- [ ] **Step 1:** Replace the `"placement"` config instructions with: Settings → General → *Strip position* (Left / Right). Left is the default; if the chosen side has no room the strip moves to the other one, and hides only if neither fits. A notification tells you when your choice could not be honored. Mention `HintCenterTaskbar` removal implicitly (no need to center icons). EN + pt-BR site match README.
- [ ] **Step 2:** `python site/build.py` + `python site/test_build.py` → OK.
- [ ] **Step 3:** commit `docs: Strip position selector in Settings`.
