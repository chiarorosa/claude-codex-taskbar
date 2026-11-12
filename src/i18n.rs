//! User-facing strings in English, Brazilian Portuguese and Spanish.
//!
//! Each key holds one string per language, in `Lang::ALL` order. To add a language, add a
//! `Lang` variant (to the end of `Lang::ALL` too), map its Windows language ID in
//! `lang_from_langid`, and add its string to every key: the compiler lists the ones missing.

/// The discriminant is the index into each key's translations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En = 0,
    PtBr = 1,
    Es = 2,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::En, Lang::PtBr, Lang::Es];
}

/// Any Portuguese UI language maps to pt-BR and any Spanish one to Spanish (by the primary
/// language ID); everything else falls back to English.
pub fn lang_from_langid(langid: u16) -> Lang {
    const LANG_PORTUGUESE: u16 = 0x16;
    const LANG_SPANISH: u16 = 0x0a;
    match langid & 0x3ff {
        LANG_PORTUGUESE => Lang::PtBr,
        LANG_SPANISH => Lang::Es,
        _ => Lang::En,
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
    RateLimited,
    RateLimitedNextFmt,
    Loading,
    ErrorFmt,
    VersionFmt,
    UpdateCheck,
    UpdateChecking,
    UpdateUpToDate,
    UpdateAvailableFmt,
    UpdateOpen,
    UpdateFailedFmt,
    HintCenterTaskbar,
    HintWidgetsButton,
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
    ShowRemaining,
    Remaining,
    NotifyNearLimitSub,
    NotifyTitle,
    NotifyPlacementTitle,
    NotifyPlacementBody,
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
        Key::RateLimited,
        Key::RateLimitedNextFmt,
        Key::Loading,
        Key::ErrorFmt,
        Key::VersionFmt,
        Key::UpdateCheck,
        Key::UpdateChecking,
        Key::UpdateUpToDate,
        Key::UpdateAvailableFmt,
        Key::UpdateOpen,
        Key::UpdateFailedFmt,
        Key::HintCenterTaskbar,
        Key::HintWidgetsButton,
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
        Key::ShowRemaining,
        Key::Remaining,
        Key::NotifyNearLimitSub,
        Key::NotifyTitle,
        Key::NotifyPlacementTitle,
        Key::NotifyPlacementBody,
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
    strings(key)[lang as usize]
}

/// Every translation of `key`, in `Lang::ALL` order. The array length comes from
/// `Lang::ALL`, so adding a language without translating every key does not compile.
fn strings(key: Key) -> [&'static str; Lang::ALL.len()] {
    match key {
        Key::MenuSettings => ["Settings", "Configurações", "Configuración"],
        Key::MenuRefresh => ["Refresh now", "Atualizar agora", "Actualizar ahora"],
        Key::MenuCopyUsage => ["Copy usage", "Copiar uso", "Copiar uso"],
        Key::MenuAutostart => [
            "Start with Windows",
            "Iniciar com o Windows",
            "Iniciar con Windows",
        ],
        Key::MenuExit => ["Exit", "Sair", "Salir"],
        Key::LabelWeek => ["wk", "sem", "sem"],
        Key::LabelOpusWeek => ["Opus wk", "Opus sem", "Opus sem"],
        Key::LabelSonnetWeek => ["Sonnet wk", "Sonnet sem", "Sonnet sem"],
        Key::ExtraUsageEnabled => [
            "Extra usage enabled",
            "Uso extra habilitado",
            "Uso adicional habilitado",
        ],
        Key::ClaudeExpired => [
            "Claude login expired — open Claude Code to renew",
            "Login do Claude expirado — abra o Claude Code para renovar",
            "Sesión de Claude caducada — abre Claude Code para renovarla",
        ],
        Key::ClaudeNoLogin => [
            "Claude Code CLI login not found — desktop app logins aren't supported; run `claude` once to fix it",
            "Login do Claude Code CLI não encontrado — logins do app desktop não são suportados; rode `claude` uma vez para resolver",
            "No se encontró el inicio de sesión de la CLI de Claude Code — los inicios de sesión de la app de escritorio no son compatibles; ejecuta `claude` una vez para solucionarlo",
        ],
        Key::CodexExpired => [
            "Codex login expired — run `codex login`",
            "Login do Codex expirado — rode `codex login`",
            "Sesión de Codex caducada — ejecuta `codex login`",
        ],
        Key::CodexNoLogin => [
            "Log in by running `codex login`",
            "Faça login rodando `codex login`",
            "Inicia sesión ejecutando `codex login`",
        ],
        Key::NoSubscription => [
            "No subscription limits (API key login)",
            "Sem limites de assinatura (login por API key)",
            "Sin límites de suscripción (inicio de sesión con clave de API)",
        ],
        Key::StaleFmt => [
            "Update failed; last data at {}",
            "Falha ao atualizar; último dado às {}",
            "Error al actualizar; último dato a las {}",
        ],
        Key::StaleReasonFmt => [
            "Update failed ({}); last data at {}",
            "Falha ao atualizar ({}); último dado às {}",
            "Error al actualizar ({}); último dato a las {}",
        ],
        Key::SessionDataFmt => [
            "Data from {} (last session)",
            "Dado de {} (última sessão)",
            "Dato de {} (última sesión)",
        ],
        Key::ResetsFmt => ["resets at {}", "reseta às {}", "se reinicia a las {}"],
        Key::ResetsInMinutesFmt => ["in {}m", "em {}min", "en {}min"],
        Key::ResetsInHoursOnlyFmt => ["in {}h", "em {}h", "en {}h"],
        Key::ResetsInHoursFmt => ["in {}h {}m", "em {}h {}min", "en {}h {}min"],
        Key::ResetsInDaysOnlyFmt => ["in {}d", "em {}d", "en {}d"],
        Key::ResetsInDaysFmt => ["in {}d {}h", "em {}d {}h", "en {}d {}h"],
        Key::UpdatedFmt => ["updated at {}", "atualizado às {}", "actualizado a las {}"],
        Key::NoRecentData => ["No recent data", "Sem dado recente", "Sin datos recientes"],
        Key::RateLimited => [
            "rate limited",
            "limite de requisições",
            "límite de solicitudes",
        ],
        Key::RateLimitedNextFmt => [
            "rate limited, next try {}",
            "limite de requisições, próxima tentativa às {}",
            "límite de solicitudes, próximo intento a las {}",
        ],
        Key::Loading => ["Loading…", "Carregando…", "Cargando…"],
        Key::ErrorFmt => ["Error: {}", "Erro: {}", "Error: {}"],
        Key::VersionFmt => ["Version {}", "Versão {}", "Versión {}"],
        Key::UpdateCheck => [
            "Check for updates",
            "Verificar atualizações",
            "Buscar actualizaciones",
        ],
        Key::UpdateChecking => ["Checking…", "Verificando…", "Comprobando…"],
        Key::UpdateUpToDate => [
            "You have the latest version",
            "Você está na versão mais recente",
            "Tienes la versión más reciente",
        ],
        Key::UpdateAvailableFmt => [
            "Version {} is available",
            "A versão {} está disponível",
            "La versión {} está disponible",
        ],
        Key::UpdateOpen => [
            "Open the release page",
            "Abrir a página da versão",
            "Abrir la página de la versión",
        ],
        Key::UpdateFailedFmt => [
            "Could not check for updates ({})",
            "Não foi possível verificar atualizações ({})",
            "No se pudo comprobar si hay actualizaciones ({})",
        ],
        Key::HintCenterTaskbar => [
            "Center the taskbar icons for the strip to fit (Settings → Personalization → Taskbar → Taskbar behaviors)",
            "Centralize os ícones da barra para a faixa caber (Configurações → Personalização → Barra de tarefas → Comportamentos)",
            "Centra los iconos de la barra de tareas para que la franja quepa (Configuración → Personalización → Barra de tareas → Comportamientos de la barra de tareas)",
        ],
        Key::HintWidgetsButton => [
            "Turn off the Windows Widgets button to free this corner (Settings → Personalization → Taskbar → Widgets)",
            "Desative o botão de Widgets do Windows para liberar este canto (Configurações → Personalização → Barra de tarefas → Widgets)",
            "Desactiva el botón de Widgets de Windows para liberar esta esquina (Configuración → Personalización → Barra de tareas → Widgets)",
        ],
        Key::HiddenItems => [
            "Not shown (no room):",
            "Não exibidos (sem espaço):",
            "No mostrados (sin espacio):",
        ],
        Key::LoginFoundStatus => ["Login found", "Login encontrado", "Sesión encontrada"],
        Key::SettingsPageTitle => ["Settings", "Configurações", "Configuración"],
        Key::SettingsGeneral => ["General", "Geral", "General"],
        Key::SettingsHardware => ["Hardware", "Hardware", "Hardware"],
        Key::SettingsLimits => ["Limits", "Limites", "Límites"],
        Key::SwitchOn => ["On", "Ativado", "Activado"],
        Key::SwitchOff => ["Off", "Desativado", "Desactivado"],
        Key::KeepAliveRow => [
            "Keep login active",
            "Manter login ativo",
            "Mantener la sesión activa",
        ],
        Key::KeepAliveSubClaude => [
            "Experimental · runs `claude auth status` when the login expires",
            "Experimental · roda `claude auth status` quando o login expira",
            "Experimental · ejecuta `claude auth status` cuando la sesión caduca",
        ],
        Key::KeepAliveSubCodex => [
            "Experimental · runs `codex login status` when the login expires",
            "Experimental · roda `codex login status` quando o login expira",
            "Experimental · ejecuta `codex login status` cuando la sesión caduca",
        ],
        Key::HwIntervalRange => ["1 to 5 seconds", "1 a 5 segundos", "De 1 a 5 segundos"],
        Key::LimitsIntervalRange => [
            "60 to 3600 seconds",
            "60 a 3600 segundos",
            "De 60 a 3600 segundos",
        ],
        Key::SettingsLabel => ["Label", "Rótulo", "Etiqueta"],
        Key::SettingsLabelSub => [
            "Shown before the gauges · up to 5 characters",
            "Aparece antes dos mostradores · até 5 caracteres",
            "Se muestra antes de los indicadores · hasta 5 caracteres",
        ],
        Key::SettingsLabelSubAccount => [
            "This account's label · up to 5 characters",
            "Rótulo desta conta · até 5 caracteres",
            "La etiqueta de esta cuenta · hasta 5 caracteres",
        ],
        Key::SettingsTitle => [
            "CCT — Settings",
            "CCT — Configurações",
            "CCT — Configuración",
        ],
        Key::SettingsProviders => ["Subscriptions", "Assinaturas", "Suscripciones"],
        Key::SettingsItems => [
            "Items on the strip",
            "Itens na faixa",
            "Elementos en la franja",
        ],
        Key::SettingsHwInterval => [
            "Hardware refresh",
            "Atualização do hardware",
            "Actualización del hardware",
        ],
        Key::SettingsLimitsInterval => [
            "Limits refresh",
            "Atualização dos limites",
            "Actualización de los límites",
        ],
        Key::SettingsWeekSuffix => ["weekly", "semanal", "semanal"],
        Key::ShowRemaining => [
            "Show remaining instead of used",
            "Mostrar restante em vez de usado",
            "Mostrar lo que queda en lugar de lo usado",
        ],
        Key::Remaining => ["left", "restante", "restante"],
        Key::NotifyNearLimit => [
            "Notify near limit",
            "Avisar ao se aproximar do limite",
            "Avisar cerca del límite",
        ],
        Key::NotifyNearLimitSub => [
            "Windows notification at {} of a limit",
            "Notificação do Windows ao chegar a {} de um limite",
            "Notificación de Windows al llegar al {} de un límite",
        ],
        Key::NotifyTitle => [
            "CCT — Usage limits",
            "CCT — Limites de uso",
            "CCT — Límites de uso",
        ],
        Key::NotifyPlacementTitle => [
            "CCT — Taskbar",
            "CCT — Barra de tarefas",
            "CCT — Barra de tareas",
        ],
        Key::NotifyPlacementBody => [
            "CCT could not fit on the taskbar: there is not enough free space next to the system tray. Close some windows or open Settings to choose fewer items.",
            "O CCT não coube na barra de tarefas: não há espaço livre suficiente ao lado da bandeja do sistema. Feche algumas janelas ou abra as Configurações para escolher menos itens.",
            "CCT no cabe en la barra de tareas: no hay suficiente espacio libre junto a la bandeja del sistema. Cierra algunas ventanas o abre Configuración para elegir menos elementos.",
        ],
        Key::NotifyFiveHour => ["5h", "5h", "5h"],
        Key::KeepAlivePromptBody => [
            "Logins for {} can expire when the CLI is not used for a while (Claude Code: about 8 hours), and the strip then shows —.\n\nKeep them active? When a login expires, the app runs the CLI's own status command in the background (`claude auth status` / `codex login status`: no prompt is sent, no quota is used) so the CLI renews its own login.\n\nExperimental. You can change this later in Settings.",
            "O login de {} pode expirar quando a CLI fica um tempo sem uso (Claude Code: cerca de 8 horas), e a faixa passa a mostrar —.\n\nManter ativo? Quando expirar, o app roda o comando de status da própria CLI em segundo plano (`claude auth status` / `codex login status`: nenhuma mensagem é enviada, não gasta cota) para a própria CLI renovar o login.\n\nExperimental. Você pode mudar isso depois nas Configurações.",
            "La sesión de {} puede caducar cuando la CLI pasa un tiempo sin usarse (Claude Code: unas 8 horas), y entonces la franja muestra —.\n\n¿Mantenerla activa? Cuando caduque, la app ejecuta en segundo plano el propio comando de estado de la CLI (`claude auth status` / `codex login status`: no se envía ningún mensaje ni se gasta cuota) para que la CLI renueve su propia sesión.\n\nExperimental. Puedes cambiarlo después en Configuración.",
        ],
        Key::KeepAliveRenewedFmt => [
            "Login renewed by the CLI at {}",
            "Login renovado pela CLI às {}",
            "Sesión renovada por la CLI a las {}",
        ],
        Key::KeepAliveTriedFmt => [
            "CLI status check ran at {}; login still expired",
            "Verificação da CLI rodou às {}; login continua expirado",
            "La comprobación de la CLI se ejecutó a las {}; la sesión sigue caducada",
        ],
        Key::MenuClaudeAccount => ["Claude account", "Conta do Claude", "Cuenta de Claude"],
        Key::MenuSaveAccount => [
            "Save current account",
            "Salvar conta atual",
            "Guardar la cuenta actual",
        ],
        Key::MenuRemoveAccount => [
            "Remove saved account",
            "Remover conta salva",
            "Quitar una cuenta guardada",
        ],
        Key::AccountNotSaved => [
            "Current login not saved",
            "Login atual não salvo",
            "La sesión actual no está guardada",
        ],
        Key::AccountAddHint => [
            "To add another account: /login in Claude Code, then Save current account",
            "Para adicionar outra conta: /login no Claude Code e depois Salvar conta atual",
            "Para añadir otra cuenta: /login en Claude Code y después Guardar la cuenta actual",
        ],
        Key::AccountExpiresFmt => [
            "⚠ login expires in {}d",
            "⚠ login expira em {}d",
            "⚠ la sesión caduca en {}d",
        ],
        Key::AccountNeedsLogin => ["⚠ needs /login", "⚠ precisa de /login", "⚠ necesita /login"],
        Key::AgoMinutesFmt => ["{}m ago", "há {}min", "hace {}min"],
        Key::AgoHoursFmt => ["{}h ago", "há {}h", "hace {}h"],
        Key::AgoDaysFmt => ["{}d ago", "há {}d", "hace {}d"],
        Key::SwitchBusyPrompt => [
            "A Claude Code task is running or waiting for your answer. It will continue on the new account from its next request, and its first message there may use extra usage while the cache rebuilds.\n\nSwitch anyway?",
            "Uma tarefa do Claude Code está em andamento ou esperando sua resposta. Ela continua na conta nova a partir da próxima requisição, e a primeira mensagem lá pode gastar mais uso enquanto o cache é reconstruído.\n\nTrocar mesmo assim?",
            "Hay una tarea de Claude Code en curso o esperando tu respuesta. Seguirá en la cuenta nueva desde su próxima solicitud, y su primer mensaje allí puede gastar más uso mientras se reconstruye la caché.\n\n¿Cambiar de todos modos?",
        ],
        Key::SwitchUnsavedPrompt => [
            "The current Claude login is not saved. A safety copy is kept, but it will not appear in the list.\n\nSwitch anyway? (Choose No, then Save current account, to keep it in the list.)",
            "O login atual do Claude não está salvo. Uma cópia de segurança é guardada, mas ela não aparece na lista.\n\nTrocar mesmo assim? (Escolha Não e depois Salvar conta atual para mantê-lo na lista.)",
            "La sesión actual de Claude no está guardada. Se conserva una copia de seguridad, pero no aparecerá en la lista.\n\n¿Cambiar de todos modos? (Elige No y después Guardar la cuenta actual para mantenerla en la lista.)",
        ],
        Key::RemoveAccountPromptFmt => [
            "Remove {} from the saved accounts? Its saved login is deleted from this PC. The live Claude Code login is not changed.",
            "Remover {} das contas salvas? O login salvo é apagado deste PC. O login atual do Claude Code não muda.",
            "¿Quitar {} de las cuentas guardadas? Su sesión guardada se borra de este PC. La sesión actual de Claude Code no cambia.",
        ],
        Key::SwitchFailedFmt => [
            "Could not switch the Claude account: {}",
            "Não foi possível trocar a conta do Claude: {}",
            "No se pudo cambiar la cuenta de Claude: {}",
        ],
        Key::SaveFailedFmt => [
            "Could not save the Claude account: {}",
            "Não foi possível salvar a conta do Claude: {}",
            "No se pudo guardar la cuenta de Claude: {}",
        ],
        Key::ErrNotPaid => [
            "only paid Claude plans (Pro, Max, Team, Enterprise) can be saved.",
            "só planos pagos do Claude (Pro, Max, Team, Enterprise) podem ser salvos.",
            "solo se pueden guardar los planes de pago de Claude (Pro, Max, Team, Enterprise).",
        ],
        Key::ErrTrial => [
            "accounts on a Claude Code trial can't be saved.",
            "contas em período de teste do Claude Code não podem ser salvas.",
            "las cuentas en periodo de prueba de Claude Code no se pueden guardar.",
        ],
        Key::ErrNotSubscription => [
            "this login is not a Claude subscription (an API key?).",
            "este login não é uma assinatura do Claude (uma chave de API?).",
            "esta sesión no es una suscripción de Claude (¿una clave de API?).",
        ],
        Key::ErrNoIdentity => [
            "Claude Code has no account details yet; open it once and try again.",
            "o Claude Code ainda não tem os dados da conta; abra-o uma vez e tente de novo.",
            "Claude Code todavía no tiene los datos de la cuenta; ábrelo una vez y vuelve a intentarlo.",
        ],
        Key::ErrConfigDir => [
            "CLAUDE_CONFIG_DIR is set, so the login lives elsewhere; this is not supported.",
            "CLAUDE_CONFIG_DIR está definido, então o login fica em outro lugar; isso não é suportado.",
            "CLAUDE_CONFIG_DIR está definido, así que la sesión está en otro lugar; esto no es compatible.",
        ],
        Key::ErrExpired => [
            "that account's saved login has expired. Run /login as that account in Claude Code, then Save current account.",
            "o login salvo dessa conta expirou. Faça /login com essa conta no Claude Code e depois Salvar conta atual.",
            "la sesión guardada de esa cuenta caducó. Haz /login con esa cuenta en Claude Code y después Guardar la cuenta actual.",
        ],
        Key::ErrUnreadableConfig => [
            "~/.claude.json could not be read; open Claude Code once and try again. Nothing was changed.",
            "não foi possível ler o ~/.claude.json; abra o Claude Code uma vez e tente de novo. Nada foi alterado.",
            "no se pudo leer ~/.claude.json; abre Claude Code una vez y vuelve a intentarlo. No se cambió nada.",
        ],
        Key::ErrLocked => [
            "Claude Code is busy renewing its login; try again in a few seconds. Nothing was changed.",
            "o Claude Code está renovando o login; tente de novo em alguns segundos. Nada foi alterado.",
            "Claude Code está renovando su sesión; vuelve a intentarlo en unos segundos. No se cambió nada.",
        ],
        Key::ErrNothingChangedFmt => [
            "{}. Nothing was changed.",
            "{}. Nada foi alterado.",
            "{}. No se cambió nada.",
        ],
        Key::ErrNoSuchSlot => [
            "that account is no longer saved.",
            "essa conta não está mais salva.",
            "esa cuenta ya no está guardada.",
        ],
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
    fn every_key_has_every_language() {
        for key in Key::ALL {
            for lang in Lang::ALL {
                assert!(!t(lang, *key).is_empty(), "{key:?} {lang:?}");
            }
        }
    }

    #[test]
    fn every_translation_keeps_the_english_placeholders() {
        for key in Key::ALL {
            let want = t(Lang::En, *key).matches("{}").count();
            for lang in Lang::ALL {
                assert_eq!(
                    t(lang, *key).matches("{}").count(),
                    want,
                    "{key:?} {lang:?}"
                );
            }
        }
    }

    #[test]
    fn lang_all_is_in_index_order() {
        for (i, lang) in Lang::ALL.into_iter().enumerate() {
            assert_eq!(lang as usize, i);
        }
    }

    #[test]
    fn key_all_lists_every_key_once() {
        let mut seen = std::collections::HashSet::new();
        for key in Key::ALL {
            assert!(seen.insert(format!("{key:?}")), "{key:?} twice");
        }
    }

    #[test]
    fn langid_maps_spanish_variants_to_spanish() {
        for id in [0x0c0a, 0x040a, 0x080a, 0x2c0a, 0x540a] {
            assert_eq!(lang_from_langid(id), Lang::Es, "{id:#06x}"); // es-ES, es-ES_tradnl, es-MX, es-AR, es-US
        }
        assert_eq!(lang_from_langid(0x0416), Lang::PtBr);
        assert_eq!(lang_from_langid(0x040c), Lang::En); // fr-FR falls back to English
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
