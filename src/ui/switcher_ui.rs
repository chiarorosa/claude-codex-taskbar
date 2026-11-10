//! Win32 glue for the "Claude account" submenu (#57): builds it from [`super::accounts`],
//! asks the confirmations, and runs a switch off the UI thread (the locks can wait seconds).

use super::accounts::{
    AccountMenu, account_menu, last_seen_from, live_state, save_error_text, switch_error_text,
};
use super::menu::{escape, text as menu_text};
use crate::i18n::{Key, Lang, fmt, t};
use crate::limits::switcher::sessions::busy_now;
use crate::limits::switcher::store::{LastSeen, Store, claude_label, read_live};
use crate::limits::switcher::switch::{SwitchError, Switched, switch_to};
use crate::timeutil::now_unix;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, HMENU, IDYES, MB_DEFBUTTON2, MB_ICONERROR, MB_ICONQUESTION,
    MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_YESNO, MESSAGEBOX_STYLE, MF_CHECKED, MF_DISABLED,
    MF_GRAYED, MF_POPUP, MF_SEPARATOR, MF_STRING, MessageBoxW, PostMessageW, WM_APP,
};
use windows::core::{PCWSTR, w};

/// Posted by the switch thread; `lparam` owns a boxed `Result<Switched, SwitchError>`.
pub const WM_APP_SWITCHED: u32 = WM_APP + 4;

const CMD_SAVE: usize = 50;
const CMD_SWITCH_BASE: usize = 1_000;
const CMD_REMOVE_BASE: usize = 1_000_000;

static SWITCHING: AtomicBool = AtomicBool::new(false);

fn wide_z(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}

fn config_dir_override() -> bool {
    std::env::var_os("CLAUDE_CONFIG_DIR").is_some_and(|v| !v.is_empty())
}

/// The saved account that holds the live Claude login, if any.
pub fn live_slot() -> Option<u32> {
    let accounts = Store::new(Store::default_dir()).load().ok()?;
    let live = home().and_then(|h| read_live(&h).ok());
    live_state(&accounts, live.as_ref(), now_unix()).slot
}

/// The label to show before the Claude gauges right now (#74): the live saved account's own
/// label, else `default`.
pub fn live_claude_label(default: &str) -> String {
    let Ok(accounts) = Store::new(Store::default_dir()).load() else {
        return default.to_string();
    };
    let live = home().and_then(|h| read_live(&h).ok());
    let slot = live_state(&accounts, live.as_ref(), now_unix()).slot;
    claude_label(default, &accounts.slots, slot)
}

/// The submenu's model for right now, or `None` to leave it out of the menu.
pub fn current_menu(lang: Lang) -> Option<AccountMenu> {
    let store = Store::new(Store::default_dir());
    let accounts = store.load().ok()?;
    let now = now_unix();
    let live = home().and_then(|h| read_live(&h).ok());
    let state = live_state(&accounts, live.as_ref(), now);
    account_menu(
        &accounts,
        state,
        SWITCHING.load(Ordering::Relaxed),
        now,
        lang,
    )
}

/// Appends the "Claude account" submenu to `parent`.
///
/// # Safety
/// `parent` must be a live menu; the submenu is owned by it and destroyed with it.
pub unsafe fn append_submenu(parent: HMENU, model: &AccountMenu, lang: Lang) {
    // SAFETY: per the caller contract; every string outlives the AppendMenuW call it is in.
    unsafe {
        let Ok(sub) = CreatePopupMenu() else { return };
        let add = |menu: HMENU, flags, id: usize, text: &str| {
            let text = wide_z(text);
            let _ = AppendMenuW(menu, flags, id, PCWSTR(text.as_ptr()));
        };
        let disabled = MF_STRING | MF_GRAYED | MF_DISABLED;
        for e in &model.entries {
            let mut flags = MF_STRING;
            if e.active {
                flags |= MF_CHECKED;
            }
            if !e.enabled {
                flags |= MF_GRAYED | MF_DISABLED;
            }
            add(
                sub,
                flags,
                CMD_SWITCH_BASE + e.id as usize,
                &escape(&e.label),
            );
        }
        for note in [&model.unsaved_note, &model.hint].into_iter().flatten() {
            add(sub, disabled, 0, &escape(note));
        }
        let _ = AppendMenuW(sub, MF_SEPARATOR, 0, PCWSTR::null());
        let save_flags = if model.can_save { MF_STRING } else { disabled };
        add(
            sub,
            save_flags,
            CMD_SAVE,
            &menu_text(lang, Key::MenuSaveAccount),
        );
        if !model.removable.is_empty()
            && let Ok(remove) = CreatePopupMenu()
        {
            for (id, email) in &model.removable {
                add(
                    remove,
                    MF_STRING,
                    CMD_REMOVE_BASE + *id as usize,
                    &escape(email),
                );
            }
            add(
                sub,
                MF_POPUP,
                remove.0 as usize,
                &menu_text(lang, Key::MenuRemoveAccount),
            );
        }
        add(
            parent,
            MF_POPUP,
            sub.0 as usize,
            &menu_text(lang, Key::MenuClaudeAccount),
        );
    }
}

fn ask(text: &str, style: MESSAGEBOX_STYLE) -> bool {
    let text = wide_z(text);
    // SAFETY: the text outlives the call; no owner window, so the strip stays non-activating.
    unsafe {
        MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            w!("CCT"),
            style | MB_SETFOREGROUND,
        ) == IDYES
    }
}

fn tell(text: &str) {
    ask(text, MB_OK | MB_ICONERROR);
}

/// Handles a menu command if it belongs to the submenu. `last_seen` is the strip's current
/// Claude usage. Returns true when a switch was started or a save/remove changed the store.
pub fn handle_command(hwnd: HWND, cmd: usize, lang: Lang, last_seen: Option<LastSeen>) -> bool {
    match cmd {
        CMD_SAVE => {
            save_current(lang);
            true
        }
        c if c >= CMD_REMOVE_BASE => {
            remove((c - CMD_REMOVE_BASE) as u32, lang);
            true
        }
        c if c >= CMD_SWITCH_BASE => {
            start_switch(hwnd, (c - CMD_SWITCH_BASE) as u32, lang, last_seen);
            true
        }
        _ => false,
    }
}

fn save_current(lang: Lang) {
    let Some(home) = home() else { return };
    let store = Store::new(Store::default_dir());
    let result = match read_live(&home) {
        Ok(live) => store.save_current(&live, config_dir_override(), now_unix()),
        Err(e) => Err(e.into()),
    };
    if let Err(e) = result {
        tell(&save_error_text(lang, &e));
    }
}

fn remove(id: u32, lang: Lang) {
    let store = Store::new(Store::default_dir());
    let Ok(accounts) = store.load() else { return };
    let Some(slot) = accounts.slots.iter().find(|s| s.id == id) else {
        return;
    };
    let prompt = fmt(t(lang, Key::RemoveAccountPromptFmt), &slot.email);
    if ask(&prompt, MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2)
        && let Err(e) = store.remove_slot(id)
    {
        tell(&save_error_text(lang, &e.into()));
    }
}

fn start_switch(hwnd: HWND, target: u32, lang: Lang, last_seen: Option<LastSeen>) {
    let Some(home) = home() else { return };
    if SWITCHING.load(Ordering::Relaxed) {
        return;
    }
    // Confirmations happen here, before any lock is taken.
    if current_menu(lang).is_some_and(|m| m.unsaved_note.is_some())
        && !ask(
            t(lang, Key::SwitchUnsavedPrompt),
            MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2,
        )
    {
        return;
    }
    if !busy_now(&home).is_empty()
        && !ask(
            t(lang, Key::SwitchBusyPrompt),
            MB_YESNO | MB_ICONQUESTION | MB_DEFBUTTON2,
        )
    {
        return;
    }
    if SWITCHING.swap(true, Ordering::Relaxed) {
        return;
    }
    // Settings shows the account being left (its label field edits that account). Close it
    // like Esc does, so pending edits land on that account before the switch writes the
    // account index, and a reopened window shows the new account.
    super::settings::close_if_open();
    let strip = hwnd.0 as isize;
    let override_dir = config_dir_override();
    std::thread::spawn(move || {
        let store = Store::new(Store::default_dir());
        let result = switch_to(&store, &home, target, override_dir, last_seen, now_unix());
        let raw = Box::into_raw(Box::new(result));
        // SAFETY: posting to our own window. On failure the box was not handed over, so it is
        // reclaimed here; on success `on_switched` takes ownership.
        unsafe {
            if PostMessageW(
                Some(HWND(strip as _)),
                WM_APP_SWITCHED,
                WPARAM(0),
                LPARAM(raw as isize),
            )
            .is_err()
            {
                drop(Box::from_raw(raw));
                SWITCHING.store(false, Ordering::Relaxed);
            }
        }
    });
}

/// Handles [`WM_APP_SWITCHED`]. Returns true when the live account changed.
pub fn on_switched(lparam: LPARAM, lang: Lang) -> bool {
    SWITCHING.store(false, Ordering::Relaxed);
    if lparam.0 == 0 {
        return false;
    }
    // SAFETY: lparam came from Box::into_raw in `start_switch`, posted exactly once.
    let result = unsafe { Box::from_raw(lparam.0 as *mut Result<Switched, SwitchError>) };
    match *result {
        Ok(s) => !s.no_op,
        Err(e) => {
            tell(&switch_error_text(lang, &e));
            false
        }
    }
}

/// The usage snapshot to store on the slot being left.
pub fn snapshot(status: &crate::limits::ProviderStatus) -> Option<LastSeen> {
    last_seen_from(status)
}
