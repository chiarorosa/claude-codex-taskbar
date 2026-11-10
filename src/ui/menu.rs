//! Right-click menu as data (#58): groups, access keys and the default item, so `show_menu`
//! only turns entries into `AppendMenuW` calls.

use crate::i18n::{Key, Lang, t};

/// Command ids of the strip's own menu items.
pub const CMD_SETTINGS: usize = 1;
pub const CMD_REFRESH: usize = 2;
pub const CMD_AUTOSTART: usize = 3;
pub const CMD_EXIT: usize = 4;
pub const CMD_COPY: usize = 5;

/// The bold item: the same action as a double-click on the strip (#48).
pub const DEFAULT_CMD: usize = CMD_REFRESH;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Command {
        id: usize,
        text: String,
        checked: bool,
    },
    /// Where the Claude account submenu goes.
    Accounts,
    Separator,
}

/// The strip's menu, top to bottom.
pub fn entries(lang: Lang, autostart: bool, accounts: bool) -> Vec<Entry> {
    let cmd = |id: usize, key: Key, checked: bool| Entry::Command {
        id,
        text: text(lang, key),
        checked,
    };
    let mut out = vec![
        cmd(CMD_REFRESH, Key::MenuRefresh, false),
        cmd(CMD_COPY, Key::MenuCopyUsage, false),
        Entry::Separator,
    ];
    if accounts {
        out.extend([Entry::Accounts, Entry::Separator]);
    }
    out.extend([
        cmd(CMD_SETTINGS, Key::MenuSettings, false),
        cmd(CMD_AUTOSTART, Key::MenuAutostart, autostart),
        Entry::Separator,
        cmd(CMD_EXIT, Key::MenuExit, false),
    ]);
    out
}

/// The access key of each menu command: the English one, kept in every language so the
/// same letter always does the same thing.
fn access_letter(key: Key) -> Option<char> {
    Some(match key {
        Key::MenuRefresh => 'r',
        Key::MenuCopyUsage => 'c',
        Key::MenuClaudeAccount => 'a',
        Key::MenuSettings => 's',
        Key::MenuAutostart => 'w',
        Key::MenuExit => 'x',
        Key::MenuSaveAccount => 's',
        Key::MenuRemoveAccount => 'r',
        _ => return None,
    })
}

/// A menu item's text: the label with its access key (`&`), `…` on items that open a window,
/// and the letter in the shortcut column after a tab, so mouse users see it too (Windows hides
/// the underline in a menu opened with the mouse).
///
/// The `&` goes before the first word that starts with the letter, else before its first
/// occurrence; when the translation has no such letter, it marks the letter in the column.
pub fn text(lang: Lang, key: Key) -> String {
    let mut label = escape(t(lang, key));
    let ellipsis = if key == Key::MenuSettings { "…" } else { "" };
    let Some(letter) = access_letter(key) else {
        return format!("{label}{ellipsis}");
    };
    let upper = letter.to_ascii_uppercase();
    let is = |c: char| c.to_lowercase().eq(letter.to_lowercase());
    let chars: Vec<(usize, char)> = label.char_indices().collect();
    let word_start = chars
        .iter()
        .enumerate()
        .find(|(n, (_, c))| is(*c) && (*n == 0 || !chars[n - 1].1.is_alphanumeric()))
        .map(|(_, (i, _))| *i);
    let at = word_start.or_else(|| chars.iter().find(|(_, c)| is(*c)).map(|(i, _)| *i));
    match at {
        Some(i) => {
            label.insert(i, '&');
            format!("{label}{ellipsis}	{upper}")
        }
        None => format!("{label}{ellipsis}	&{upper}"),
    }
}

/// Escapes `&` in text that comes from data (emails, labels) so it never becomes an access key.
pub fn escape(text: &str) -> String {
    text.replace('&', "&&")
}

#[cfg(test)]
mod tests {
    use super::*;

    const LANGS: [Lang; 2] = [Lang::En, Lang::PtBr];

    /// The access key of a menu text: the character after a single `&`.
    fn access_key(text: &str) -> Option<char> {
        let mut chars = text.chars().peekable();
        let mut found = None;
        while let Some(c) = chars.next() {
            if c == '&' {
                match chars.next() {
                    Some('&') => {}
                    Some(k) => {
                        assert!(found.is_none(), "two access keys in {text:?}");
                        found = Some(k.to_lowercase().next().unwrap());
                    }
                    None => panic!("dangling & in {text:?}"),
                }
            }
        }
        found
    }

    fn assert_unique_keys(texts: &[String]) {
        let mut keys: Vec<char> = texts
            .iter()
            .map(|s| access_key(s).unwrap_or_else(|| panic!("no access key in {s:?}")))
            .collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), before, "duplicate access keys in {texts:?}");
    }

    fn shape(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|e| match e {
                Entry::Command { id, checked, .. } => {
                    format!("{id}{}", if *checked { "✓" } else { "" })
                }
                Entry::Accounts => "accounts".into(),
                Entry::Separator => "-".into(),
            })
            .collect()
    }

    #[test]
    fn menu_is_grouped_with_accounts_in_their_own_group() {
        let expected = ["2", "5", "-", "accounts", "-", "1", "3✓", "-", "4"];
        assert_eq!(shape(&entries(Lang::En, true, true)), expected);
    }

    #[test]
    fn without_accounts_there_is_no_empty_group() {
        assert_eq!(
            shape(&entries(Lang::PtBr, false, false)),
            ["2", "5", "-", "1", "3", "-", "4"]
        );
    }

    #[test]
    fn default_item_is_refresh_and_is_in_the_menu() {
        assert_eq!(DEFAULT_CMD, CMD_REFRESH);
        assert!(
            entries(Lang::En, false, false)
                .iter()
                .any(|e| matches!(e, Entry::Command { id, .. } if *id == DEFAULT_CMD))
        );
    }

    #[test]
    fn main_menu_access_keys_are_unique_in_both_languages() {
        for lang in LANGS {
            let mut texts: Vec<String> = entries(lang, false, true)
                .into_iter()
                .filter_map(|e| match e {
                    Entry::Command { text, .. } => Some(text),
                    _ => None,
                })
                .collect();
            texts.push(text(lang, Key::MenuClaudeAccount));
            assert_unique_keys(&texts);
        }
    }

    #[test]
    fn account_submenu_access_keys_are_unique() {
        for lang in LANGS {
            assert_unique_keys(&[
                text(lang, Key::MenuSaveAccount),
                text(lang, Key::MenuRemoveAccount),
            ]);
        }
    }

    const MENU_KEYS: [Key; 8] = [
        Key::MenuRefresh,
        Key::MenuCopyUsage,
        Key::MenuClaudeAccount,
        Key::MenuSettings,
        Key::MenuAutostart,
        Key::MenuExit,
        Key::MenuSaveAccount,
        Key::MenuRemoveAccount,
    ];

    #[test]
    fn access_keys_are_the_english_ones_in_every_language() {
        for key in MENU_KEYS {
            let en = access_key(&text(Lang::En, key));
            assert!(en.is_some(), "{key:?}");
            assert_eq!(access_key(&text(Lang::PtBr, key)), en, "{key:?}");
        }
    }

    #[test]
    fn texts_keep_the_translation_and_settings_gets_an_ellipsis() {
        for lang in LANGS {
            for key in MENU_KEYS {
                let plain = text(lang, key).replace('&', "");
                assert!(plain.starts_with(t(lang, key)), "{plain:?}");
            }
            assert!(text(lang, Key::MenuSettings).contains("…	"));
        }
        assert_eq!(text(Lang::En, Key::MenuClaudeAccount), "Claude &account	A");
        assert_eq!(text(Lang::En, Key::MenuExit), "E&xit	X");
        assert_eq!(text(Lang::PtBr, Key::MenuRefresh), "Atualiza&r agora	R");
        assert_eq!(text(Lang::PtBr, Key::MenuSettings), "Configuraçõe&s…	S");
    }

    #[test]
    fn a_letter_missing_from_the_translation_lives_in_the_shortcut_column() {
        assert_eq!(text(Lang::PtBr, Key::MenuExit), "Sair	&X");
    }

    #[test]
    fn every_item_shows_its_letter_in_the_shortcut_column() {
        for lang in LANGS {
            for key in MENU_KEYS {
                let s = text(lang, key);
                let (_, column) = s.split_once('\t').unwrap_or_else(|| panic!("{s:?}"));
                let letter = access_key(&s).unwrap().to_ascii_uppercase().to_string();
                assert_eq!(column.replace('&', ""), letter, "{s:?}");
            }
        }
    }

    #[test]
    fn data_text_never_gets_an_access_key() {
        assert_eq!(escape("R&D <a&b@x.com>"), "R&&D <a&&b@x.com>");
        assert_eq!(access_key(&escape("a&b")), None);
        assert_eq!(escape("plain"), "plain");
    }
}
