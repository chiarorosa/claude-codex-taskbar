//! User-facing strings in English and Brazilian Portuguese.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    PtBr,
}

/// Any Portuguese UI language maps to pt-BR; everything else falls back to English.
pub fn lang_from_langid(langid: u16) -> Lang {
    const LANG_PORTUGUESE: u16 = 0x16;
    if langid & 0x3ff == LANG_PORTUGUESE {
        Lang::PtBr
    } else {
        Lang::En
    }
}

pub fn detect_lang() -> Lang {
    // SAFETY: no arguments, no preconditions.
    let langid = unsafe { windows::Win32::Globalization::GetUserDefaultUILanguage() };
    lang_from_langid(langid)
}

/// Replaces the first `{}` in a template string.
pub fn fmt(template: &str, value: &str) -> String {
    template.replacen("{}", value, 1)
}

/// Fills both `{}` of a template in order; a `{}` inside `first` stays literal.
pub fn fmt2(template: &str, first: &str, second: &str) -> String {
    match template.split_once("{}") {
        Some((head, rest)) => format!("{head}{first}{}", fmt(rest, second)),
        None => template.into(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    MenuSettings,
    MenuRefresh,
    MenuCopyUsage,
    MenuAutostart,
    MenuExit,
    LabelWeek,
    LabelOpusWeek,
    LabelSonnetWeek,
    ExtraUsageEnabled,
    ClaudeExpired,
    ClaudeNoLogin,
    CodexExpired,
    CodexNoLogin,
    NoSubscription,
    StaleFmt,
    StaleReasonFmt,
    SessionDataFmt,
    ResetsFmt,
    ResetsInMinutesFmt,
    ResetsInHoursOnlyFmt,
    ResetsInHoursFmt,
    ResetsInDaysOnlyFmt,
    ResetsInDaysFmt,
    UpdatedFmt,
    NoRecentData,
    Loading,
    ErrorFmt,
    HintCenterTaskbar,
    HintWidgetsButton,
    HintWindows10,
    HiddenItems,
    LoginFound,
    SettingsTitle,
    SettingsProviders,
    SettingsItems,
    SettingsHwInterval,
    SettingsLimitsInterval,
    SettingsWeekSuffix,
    NotifyNearLimit,
    NotifyTitle,
    NotifyFiveHour,
    KeepAliveSetting,
    KeepAlivePromptBody,
    KeepAliveRenewedFmt,
    KeepAliveTriedFmt,
    Ok,
    Cancel,
}

impl Key {
    pub const ALL: &'static [Key] = &[
        Key::MenuSettings,
        Key::MenuRefresh,
        Key::MenuCopyUsage,
        Key::MenuAutostart,
        Key::MenuExit,
        Key::LabelWeek,
        Key::LabelOpusWeek,
        Key::LabelSonnetWeek,
        Key::ExtraUsageEnabled,
        Key::ClaudeExpired,
        Key::ClaudeNoLogin,
        Key::CodexExpired,
        Key::CodexNoLogin,
        Key::NoSubscription,
        Key::StaleFmt,
        Key::StaleReasonFmt,
        Key::SessionDataFmt,
        Key::ResetsFmt,
        Key::ResetsInMinutesFmt,
        Key::ResetsInHoursOnlyFmt,
        Key::ResetsInHoursFmt,
        Key::ResetsInDaysOnlyFmt,
        Key::ResetsInDaysFmt,
        Key::UpdatedFmt,
        Key::NoRecentData,
        Key::Loading,
        Key::ErrorFmt,
        Key::HintCenterTaskbar,
        Key::HintWidgetsButton,
        Key::HintWindows10,
        Key::HiddenItems,
        Key::LoginFound,
        Key::SettingsTitle,
        Key::SettingsProviders,
        Key::SettingsItems,
        Key::SettingsHwInterval,
        Key::SettingsLimitsInterval,
        Key::SettingsWeekSuffix,
        Key::NotifyNearLimit,
        Key::NotifyTitle,
        Key::NotifyFiveHour,
        Key::KeepAliveSetting,
        Key::KeepAlivePromptBody,
        Key::KeepAliveRenewedFmt,
        Key::KeepAliveTriedFmt,
        Key::Ok,
        Key::Cancel,
    ];
}

pub fn t(lang: Lang, key: Key) -> &'static str {
    let (en, pt) = match key {
        Key::MenuSettings => ("Settings", "Configurações"),
        Key::MenuRefresh => ("Refresh now", "Atualizar agora"),
        Key::MenuCopyUsage => ("Copy usage", "Copiar uso"),
        Key::MenuAutostart => ("Start with Windows", "Iniciar com o Windows"),
        Key::MenuExit => ("Exit", "Sair"),
        Key::LabelWeek => ("wk", "sem"),
        Key::LabelOpusWeek => ("Opus wk", "Opus sem"),
        Key::LabelSonnetWeek => ("Sonnet wk", "Sonnet sem"),
        Key::ExtraUsageEnabled => ("Extra usage enabled", "Uso extra habilitado"),
        Key::ClaudeExpired => (
            "Claude login expired — open Claude Code to renew",
            "Login do Claude expirado — abra o Claude Code para renovar",
        ),
        Key::ClaudeNoLogin => ("Log in by running `claude`", "Faça login rodando `claude`"),
        Key::CodexExpired => (
            "Codex login expired — run `codex login`",
            "Login do Codex expirado — rode `codex login`",
        ),
        Key::CodexNoLogin => (
            "Log in by running `codex login`",
            "Faça login rodando `codex login`",
        ),
        Key::NoSubscription => (
            "No subscription limits (API key login)",
            "Sem limites de assinatura (login por API key)",
        ),
        Key::StaleFmt => (
            "Update failed; last data at {}",
            "Falha ao atualizar; último dado às {}",
        ),
        Key::StaleReasonFmt => (
            "Update failed ({}); last data at {}",
            "Falha ao atualizar ({}); último dado às {}",
        ),
        Key::SessionDataFmt => ("Data from {} (last session)", "Dado de {} (última sessão)"),
        Key::ResetsFmt => ("resets at {}", "reseta às {}"),
        Key::ResetsInMinutesFmt => ("in {}m", "em {}min"),
        Key::ResetsInHoursOnlyFmt => ("in {}h", "em {}h"),
        Key::ResetsInHoursFmt => ("in {}h {}m", "em {}h {}min"),
        Key::ResetsInDaysOnlyFmt => ("in {}d", "em {}d"),
        Key::ResetsInDaysFmt => ("in {}d {}h", "em {}d {}h"),
        Key::UpdatedFmt => ("updated at {}", "atualizado às {}"),
        Key::NoRecentData => ("No recent data", "Sem dado recente"),
        Key::Loading => ("Loading…", "Carregando…"),
        Key::ErrorFmt => ("Error: {}", "Erro: {}"),
        Key::HintCenterTaskbar => (
            "Center the taskbar icons for the strip to fit (Settings → Personalization → Taskbar → Taskbar behaviors)",
            "Centralize os ícones da barra para a faixa caber (Configurações → Personalização → Barra de tarefas → Comportamentos)",
        ),
        Key::HintWidgetsButton => (
            "Turn off the Windows Widgets button to free this corner (Settings → Personalization → Taskbar → Widgets)",
            "Desative o botão de Widgets do Windows para liberar este canto (Configurações → Personalização → Barra de tarefas → Widgets)",
        ),
        Key::HintWindows10 => (
            "On Windows 10 the taskbar icons start at the left edge, so there is no free corner for the strip yet. This may change in a future version.",
            "No Windows 10, os ícones da barra de tarefas começam na borda esquerda, então ainda não há um canto livre para a faixa. Isso pode mudar em uma versão futura.",
        ),
        Key::HiddenItems => ("Not shown (no room):", "Não exibidos (sem espaço):"),
        Key::LoginFound => ("login found", "login encontrado"),
        Key::SettingsTitle => ("CCT — Settings", "CCT — Configurações"),
        Key::SettingsProviders => ("Subscriptions", "Assinaturas"),
        Key::SettingsItems => ("Items on the strip", "Itens na faixa"),
        Key::SettingsHwInterval => (
            "Hardware refresh (1–5 s)",
            "Atualização do hardware (1–5 s)",
        ),
        Key::SettingsLimitsInterval => (
            "Limits refresh (60–3600 s)",
            "Atualização dos limites (60–3600 s)",
        ),
        Key::SettingsWeekSuffix => ("weekly", "semanal"),
        Key::NotifyNearLimit => ("Notify near limit", "Avisar ao se aproximar do limite"),
        Key::NotifyTitle => ("CCT — Usage limits", "CCT — Limites de uso"),
        Key::NotifyFiveHour => ("5h", "5h"),
        Key::KeepAliveSetting => (
            "Keep {} login active (experimental)",
            "Manter login do {} ativo (experimental)",
        ),
        Key::KeepAlivePromptBody => (
            "Logins for {} can expire when the CLI is not used for a while (Claude Code: about 8 hours), and the strip then shows —.\n\nKeep them active? When a login expires, the app runs the CLI's own status command in the background (`claude auth status` / `codex login status`: no prompt is sent, no quota is used) so the CLI renews its own login.\n\nExperimental. You can change this later in Settings.",
            "O login de {} pode expirar quando a CLI fica um tempo sem uso (Claude Code: cerca de 8 horas), e a faixa passa a mostrar —.\n\nManter ativo? Quando expirar, o app roda o comando de status da própria CLI em segundo plano (`claude auth status` / `codex login status`: nenhuma mensagem é enviada, não gasta cota) para a própria CLI renovar o login.\n\nExperimental. Você pode mudar isso depois nas Configurações.",
        ),
        Key::KeepAliveRenewedFmt => (
            "Login renewed by the CLI at {}",
            "Login renovado pela CLI às {}",
        ),
        Key::KeepAliveTriedFmt => (
            "CLI status check ran at {}; login still expired",
            "Verificação da CLI rodou às {}; login continua expirado",
        ),
        Key::Ok => ("OK", "OK"),
        Key::Cancel => ("Cancel", "Cancelar"),
    };
    match lang {
        Lang::En => en,
        Lang::PtBr => pt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fmt2_fills_both_placeholders_in_order() {
        assert_eq!(fmt2("a ({}); b {}", "x", "y"), "a (x); b y");
    }

    #[test]
    fn fmt2_keeps_braces_inside_the_first_value() {
        assert_eq!(fmt2("({}) at {}", "odd {}", "09:41"), "(odd {}) at 09:41");
    }

    #[test]
    fn every_key_has_both_languages() {
        for key in Key::ALL {
            assert!(!t(Lang::En, *key).is_empty(), "{key:?} en");
            assert!(!t(Lang::PtBr, *key).is_empty(), "{key:?} pt");
        }
    }

    #[test]
    fn langid_maps_portuguese_to_pt_br() {
        assert_eq!(lang_from_langid(0x0416), Lang::PtBr); // pt-BR
        assert_eq!(lang_from_langid(0x0816), Lang::PtBr); // pt-PT
        assert_eq!(lang_from_langid(0x0409), Lang::En);
        assert_eq!(lang_from_langid(0x0407), Lang::En); // de-DE falls back to English
    }

    #[test]
    fn fmt_substitutes_placeholder() {
        assert_eq!(fmt(t(Lang::En, Key::ResetsFmt), "15:00"), "resets at 15:00");
    }
}
