# Strip position selector: Left / Right with fallback

Date: 2026-11-12 · Follow-up to #10 / PR #121 (stacked on `feat/strip-next-to-tray`).

## Intent

PR #121 added `"placement": "auto" | "left" | "right"` in `config.json`, with no UI, and
`auto` chosen by Windows version. The maintainer wants:

- a **Left / Right** selector in Settings → **General**, which moves the strip at once;
- the choice to be a **preference with fallback**, decided by **free space**, not by Windows
  version (Windows 11 can also have Start on the left);
- a notification whenever an **explicit** choice is not honored.

## Behavior (maintainer's decisions)

- Selector shows two options, **Left** and **Right**; no "Automatic" option. Default is Left.
- Left → try the left corner; if it has no room, go next to the tray.
  Right → try next to the tray; if it has no room, go to the left corner.
- If neither side has room: the strip hides.
- Whenever an explicit choice is not honored (the strip moved to the other side, or hid),
  notify.
- Changing the selector applies immediately and saves `config.json`.

## Calls made in design (maintainer can override)

1. **Config values stay `auto | left | right`.** `auto` (never touched the selector) behaves
   like Left → Right, and the selector shows Left as selected, but fallback to the right is
   **silent** (otherwise every Windows 10 user would get a notice on every launch without
   having chosen anything). Hiding is still notified for `auto`, as in PR #121. Picking
   either option in Settings writes `left` or `right` (explicit).
2. **"Has room" per side** = the layout for that side's free width keeps at least one item
   (`layout(...).width > 0`; hardware is dropped first, so that item is a usage limit when any
   is enabled). Left free width = today's `available_width` (0 for left-aligned / Windows 10
   taskbars). Right free width = `right_gap_width`.
3. **Notifications per transition, not per run.** One notice when the placement settles into
   a non-preferred state (fallback side, or hidden); no repeat while that state lasts; no
   notice when it returns to the preferred side. A new notice may follow when the state
   changes again (e.g. fallback → hidden). The PR #121 settle rules stay: a state must hold
   10 s (`SETTLE_SECS`), nothing settles in the 30 s grace after startup or Explorer restart
   (`GRACE_SECS`), notices wait while a fullscreen app is in front. This replaces PR #121's
   "one notice per run".
4. **Texts** (exact, three languages), title stays `NotifyPlacementTitle`:
   - Moved to the right: EN "CCT moved next to the system tray: there is no room in the left corner of the taskbar." · pt-BR "O CCT foi para o lado da bandeja do sistema: não há espaço no canto esquerdo da barra de tarefas." · ES "CCT se movió junto a la bandeja del sistema: no hay espacio en la esquina izquierda de la barra de tareas."
   - Moved to the left: EN "CCT moved to the left corner of the taskbar: there is no room next to the system tray." · pt-BR "O CCT foi para o canto esquerdo da barra de tarefas: não há espaço ao lado da bandeja do sistema." · ES "CCT se movió a la esquina izquierda de la barra de tareas: no hay espacio junto a la bandeja del sistema."
   - Hidden: the existing `NotifyPlacementBody` text is reworded to cover both sides: EN "CCT could not fit on the taskbar: there is no room in the left corner or next to the system tray. Close some windows or open Settings to choose fewer items." · pt-BR "O CCT não coube na barra de tarefas: não há espaço no canto esquerdo nem ao lado da bandeja do sistema. Feche algumas janelas ou abra as Configurações para escolher menos itens." · ES "CCT no cabe en la barra de tareas: no hay espacio en la esquina izquierda ni junto a la bandeja del sistema. Cierra algunas ventanas o abre Configuración para elegir menos elementos."
   - Selector row (General card): title EN "Strip position" · pt-BR "Posição da faixa" · ES "Posición de la franja"; options EN "Left" / "Right" · pt-BR "Esquerda" / "Direita" · ES "Izquierda" / "Derecha".
5. **Click on any placement notice opens Settings** (as in PR #121). Settings opens by itself
   only for a failure settled at startup, as in PR #121; for a fallback (strip visible) it
   does not open by itself.
6. **Control**: real Win32 radio buttons (`BS_AUTORADIOBUTTON`, one `WS_GROUP`) painted like
   the existing chips (DESIGN.md: real controls, our paint; keyboard arrows move between them;
   UIA sees a radio group). Placed as a row in the General card, right after "Start with
   Windows".
7. Vertical taskbars: unchanged from PR #121 (left behavior, no notice).

## Non-goals

- No "Automatic" UI option, no per-monitor choice, no compact strip.
- No change to the layout drop rules or the edge lookups from PR #121 (beyond reading the
  right edges also when Left is preferred but has no room).

## Anti-goals

- Covering taskbar icons; side ping-pong notices while windows open/close (settle rule).
- Notifying users who never chose a side when the strip just went to the right.

## Validation

- Unit tests: side resolution with fallback for every preference × room combination;
  transition notices (fallback, hidden, return, repeated state, auto silent fallback);
  config round-trip; selector state from config.
- Manual (maintainer, Windows 11): selector moves the strip instantly both ways; Right with
  many windows → moves left + one notice; Left on left-aligned icons → moves right + one notice;
  default config on left-aligned icons → moves right silently; keyboard arrows on the
  selector; light/dark.
- Gate: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`;
  README/CHANGELOG/site updated (EN + pt-BR), `python site/build.py` + `python site/test_build.py`.

## Left to the builder

- How the render loop gets both sides' widths cheaply (read right edges only when Right is
  preferred or Left has no room).
- Exact paint of the radio chips within DESIGN.md tokens.
