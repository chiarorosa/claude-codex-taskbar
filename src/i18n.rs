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
    LoginFoundStatus,
    SettingsPageTitle,
    SettingsGeneral,
    SettingsHardware,
    SettingsLimits,
    SwitchOn,
    SwitchOff,
    KeepAliveRow,
    KeepAliveSubClaude,
    KeepAliveSubCodex,
    HwIntervalRange,
    LimitsIntervalRange,
    SettingsLabel,
    SettingsLabelSub,
    SettingsLabelSubAccount,
    SettingsTitle,
    SettingsProviders,
    SettingsItems,
    SettingsHwInterval,
    SettingsLimitsInterval,
    SettingsWeekSuffix,
    NotifyNearLimit,
    NotifyNearLimitSub,
    NotifyTitle,
    NotifyFiveHour,
    KeepAlivePromptBody,
    KeepAliveRenewedFmt,
    KeepAliveTriedFmt,
    MenuClaudeAccount,
    MenuSaveAccount,
    MenuRemoveAccount,
    AccountNotSaved,
    AccountAddHint,
    AccountExpiresFmt,
    AccountNeedsLogin,
    AgoMinutesFmt,
    AgoHoursFmt,
    AgoDaysFmt,
    SwitchBusyPrompt,
    SwitchUnsavedPrompt,
    RemoveAccountPromptFmt,
    SwitchFailedFmt,
    SaveFailedFmt,
    ErrNotPaid,
    ErrTrial,
    ErrNotSubscription,
    ErrNoIdentity,
    ErrConfigDir,
    ErrExpired,
    ErrUnreadableConfig,
    ErrLocked,
    ErrNothingChangedFmt,
    ErrNoSuchSlot,
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
        Key::LoginFoundStatus,
        Key::SettingsPageTitle,
        Key::SettingsGeneral,
        Key::SettingsHardware,
        Key::SettingsLimits,
        Key::SwitchOn,
        Key::SwitchOff,
        Key::KeepAliveRow,
        Key::KeepAliveSubClaude,
        Key::KeepAliveSubCodex,
        Key::HwIntervalRange,
        Key::LimitsIntervalRange,
        Key::SettingsLabel,
        Key::SettingsLabelSub,
        Key::SettingsLabelSubAccount,
        Key::SettingsTitle,
        Key::SettingsProviders,
        Key::SettingsItems,
        Key::SettingsHwInterval,
        Key::SettingsLimitsInterval,
        Key::SettingsWeekSuffix,
        Key::NotifyNearLimit,
        Key::NotifyNearLimitSub,
        Key::NotifyTitle,
        Key::NotifyFiveHour,
        Key::KeepAlivePromptBody,
        Key::KeepAliveRenewedFmt,
        Key::KeepAliveTriedFmt,
        Key::MenuClaudeAccount,
        Key::MenuSaveAccount,
        Key::MenuRemoveAccount,
        Key::AccountNotSaved,
        Key::AccountAddHint,
        Key::AccountExpiresFmt,
        Key::AccountNeedsLogin,
        Key::AgoMinutesFmt,
        Key::AgoHoursFmt,
        Key::AgoDaysFmt,
        Key::SwitchBusyPrompt,
        Key::SwitchUnsavedPrompt,
        Key::RemoveAccountPromptFmt,
        Key::SwitchFailedFmt,
        Key::SaveFailedFmt,
        Key::ErrNotPaid,
        Key::ErrTrial,
        Key::ErrNotSubscription,
        Key::ErrNoIdentity,
        Key::ErrConfigDir,
        Key::ErrExpired,
        Key::ErrUnreadableConfig,
        Key::ErrLocked,
        Key::ErrNothingChangedFmt,
        Key::ErrNoSuchSlot,
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
        Key::LoginFoundStatus => ("Login found", "Login encontrado"),
        Key::SettingsPageTitle => ("Settings", "Configurações"),
        Key::SettingsGeneral => ("General", "Geral"),
        Key::SettingsHardware => ("Hardware", "Hardware"),
        Key::SettingsLimits => ("Limits", "Limites"),
        Key::SwitchOn => ("On", "Ativado"),
        Key::SwitchOff => ("Off", "Desativado"),
        Key::KeepAliveRow => ("Keep login active", "Manter login ativo"),
        Key::KeepAliveSubClaude => (
            "Experimental · runs `claude auth status` when the login expires",
            "Experimental · roda `claude auth status` quando o login expira",
        ),
        Key::KeepAliveSubCodex => (
            "Experimental · runs `codex login status` when the login expires",
            "Experimental · roda `codex login status` quando o login expira",
        ),
        Key::HwIntervalRange => ("1 to 5 seconds", "1 a 5 segundos"),
        Key::LimitsIntervalRange => ("60 to 3600 seconds", "60 a 3600 segundos"),
        Key::SettingsLabel => ("Label", "Rótulo"),
        Key::SettingsLabelSub => (
            "Shown before the gauges · up to 5 characters",
            "Aparece antes dos mostradores · até 5 caracteres",
        ),
        Key::SettingsLabelSubAccount => (
            "This account's label · up to 5 characters",
            "Rótulo desta conta · até 5 caracteres",
        ),
        Key::SettingsTitle => ("CCT — Settings", "CCT — Configurações"),
        Key::SettingsProviders => ("Subscriptions", "Assinaturas"),
        Key::SettingsItems => ("Items on the strip", "Itens na faixa"),
        Key::SettingsHwInterval => ("Hardware refresh", "Atualização do hardware"),
        Key::SettingsLimitsInterval => ("Limits refresh", "Atualização dos limites"),
        Key::SettingsWeekSuffix => ("weekly", "semanal"),
        Key::NotifyNearLimit => ("Notify near limit", "Avisar ao se aproximar do limite"),
        Key::NotifyNearLimitSub => (
            "Windows notification at {} of a limit",
            "Notificação do Windows ao chegar a {} de um limite",
        ),
        Key::NotifyTitle => ("CCT — Usage limits", "CCT — Limites de uso"),
        Key::NotifyFiveHour => ("5h", "5h"),
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
        Key::MenuClaudeAccount => ("Claude account", "Conta do Claude"),
        Key::MenuSaveAccount => ("Save current account", "Salvar conta atual"),
        Key::MenuRemoveAccount => ("Remove saved account", "Remover conta salva"),
        Key::AccountNotSaved => ("Current login not saved", "Login atual não salvo"),
        Key::AccountAddHint => (
            "To add another account: /login in Claude Code, then Save current account",
            "Para adicionar outra conta: /login no Claude Code e depois Salvar conta atual",
        ),
        Key::AccountExpiresFmt => ("⚠ login expires in {}d", "⚠ login expira em {}d"),
        Key::AccountNeedsLogin => ("⚠ needs /login", "⚠ precisa de /login"),
        Key::AgoMinutesFmt => ("{}m ago", "há {}min"),
        Key::AgoHoursFmt => ("{}h ago", "há {}h"),
        Key::AgoDaysFmt => ("{}d ago", "há {}d"),
        Key::SwitchBusyPrompt => (
            "A Claude Code task is running or waiting for your answer. It will continue on the new account from its next request, and its first message there may use extra usage while the cache rebuilds.\n\nSwitch anyway?",
            "Uma tarefa do Claude Code está em andamento ou esperando sua resposta. Ela continua na conta nova a partir da próxima requisição, e a primeira mensagem lá pode gastar mais uso enquanto o cache é reconstruído.\n\nTrocar mesmo assim?",
        ),
        Key::SwitchUnsavedPrompt => (
            "The current Claude login is not saved. A safety copy is kept, but it will not appear in the list.\n\nSwitch anyway? (Choose No, then Save current account, to keep it in the list.)",
            "O login atual do Claude não está salvo. Uma cópia de segurança é guardada, mas ela não aparece na lista.\n\nTrocar mesmo assim? (Escolha Não e depois Salvar conta atual para mantê-lo na lista.)",
        ),
        Key::RemoveAccountPromptFmt => (
            "Remove {} from the saved accounts? Its saved login is deleted from this PC. The live Claude Code login is not changed.",
            "Remover {} das contas salvas? O login salvo é apagado deste PC. O login atual do Claude Code não muda.",
        ),
        Key::SwitchFailedFmt => (
            "Could not switch the Claude account: {}",
            "Não foi possível trocar a conta do Claude: {}",
        ),
        Key::SaveFailedFmt => (
            "Could not save the Claude account: {}",
            "Não foi possível salvar a conta do Claude: {}",
        ),
        Key::ErrNotPaid => (
            "only paid Claude plans (Pro, Max, Team, Enterprise) can be saved.",
            "só planos pagos do Claude (Pro, Max, Team, Enterprise) podem ser salvos.",
        ),
        Key::ErrTrial => (
            "accounts on a Claude Code trial can't be saved.",
            "contas em período de teste do Claude Code não podem ser salvas.",
        ),
        Key::ErrNotSubscription => (
            "this login is not a Claude subscription (an API key?).",
            "este login não é uma assinatura do Claude (uma chave de API?).",
        ),
        Key::ErrNoIdentity => (
            "Claude Code has no account details yet; open it once and try again.",
            "o Claude Code ainda não tem os dados da conta; abra-o uma vez e tente de novo.",
        ),
        Key::ErrConfigDir => (
            "CLAUDE_CONFIG_DIR is set, so the login lives elsewhere; this is not supported.",
            "CLAUDE_CONFIG_DIR está definido, então o login fica em outro lugar; isso não é suportado.",
        ),
        Key::ErrExpired => (
            "that account's saved login has expired. Run /login as that account in Claude Code, then Save current account.",
            "o login salvo dessa conta expirou. Faça /login com essa conta no Claude Code e depois Salvar conta atual.",
        ),
        Key::ErrUnreadableConfig => (
            "~/.claude.json could not be read; open Claude Code once and try again. Nothing was changed.",
            "não foi possível ler o ~/.claude.json; abra o Claude Code uma vez e tente de novo. Nada foi alterado.",
        ),
        Key::ErrLocked => (
            "Claude Code is busy renewing its login; try again in a few seconds. Nothing was changed.",
            "o Claude Code está renovando o login; tente de novo em alguns segundos. Nada foi alterado.",
        ),
        Key::ErrNothingChangedFmt => ("{}. Nothing was changed.", "{}. Nada foi alterado."),
        Key::ErrNoSuchSlot => (
            "that account is no longer saved.",
            "essa conta não está mais salva.",
        ),
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
