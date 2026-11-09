# Security Policy

This app reads the login credentials of the Claude Code and Codex CLIs on your machine, so we take security seriously.

## Supported versions

The project is in early development. Only the **latest release** receives security fixes.

## Verifying a release

Code signing of the executable is planned through SignPath Foundation; see [CODE_SIGNING.md](CODE_SIGNING.md).

Every release from v0.2.1 on carries a signed build provenance attestation (Sigstore), created by
the release workflow for `claude-codex-taskbar.exe` and the `.zip`. Check that a download was built
from this repository with the [GitHub CLI](https://cli.github.com/):

```powershell
gh attestation verify claude-codex-taskbar.exe --repo chiarorosa/claude-codex-taskbar
```

`SHA256SUMS.txt` is still attached to each release for a quick integrity check.

## Reporting a vulnerability

**Please do not report security vulnerabilities through public GitHub issues, discussions, or pull requests.**

Report them privately through **GitHub Security Advisories**:

1. Go to the repository's **Security** tab.
2. Click **Report a vulnerability**.
3. Describe the issue, its impact, and steps to reproduce.

Please **never include real tokens or the contents of your credentials files** in a report. Redact them (for example, `sk-...REDACTED`).

What to expect:

- An acknowledgement within about **7 days**.
- A status update as the investigation progresses.
- Credit in the advisory and release notes, if you wish, once a fix is published.

Please give us reasonable time to release a fix before disclosing the issue publicly.

## Credential-handling rules

These rules are part of the project's design. Any violation is treated as a security bug, and contributions must follow them:

1. **Read-only access.** The app only reads `%USERPROFILE%\.claude\.credentials.json` and `%USERPROFILE%\.codex\auth.json` (or `%CODEX_HOME%\auth.json`). It never writes, moves, locks for writing, or deletes these files. **One exception, the Claude account switcher (#53):** when the user explicitly clicks *Switch*, the app writes `%USERPROFILE%\.claude\.credentials.json` and the `oauthAccount` key of `%USERPROFILE%\.claude.json`, holding Claude Code's own lock directories, through a temp file and rename, after backing up the outgoing login, and restores both files if any step fails. The Codex login is never written.
2. **No token refresh.** The app never refreshes or rotates tokens. An expired token is shown as an "expired" state, and the user is asked to re-open the official CLI.
3. **Single destination.** A provider's credentials are only ever sent to that provider's own endpoint over HTTPS (Anthropic credentials to Anthropic, OpenAI credentials to OpenAI). Never to any other host, proxy, or service.
4. **Never logged or persisted.** Tokens, account IDs, and raw credential file contents must never appear in logs, debug output, error messages, panic messages, crash dumps produced by the app, settings files, or the clipboard. Tokens are not cached on disk, except the Claude logins the user saves for the account switcher: those live in `%APPDATA%\claude-codex-taskbar\accounts\`, each encrypted with DPAPI for the current Windows user, next to an index that holds no tokens.
5. **Minimal lifetime in memory.** Read credentials only when needed for a request or an explicit switch, and do not keep extra copies.
6. **No telemetry.** The app sends no analytics or usage data anywhere. Network access is limited to the usage endpoints of the providers the user has enabled.
7. **Fake fixtures only.** Tests and fixtures must use obviously fake tokens. Never commit real credentials.
8. **No elevation.** The app runs without administrator privileges.

## Scope

In scope: anything in this repository that could leak, misuse, or corrupt credentials; send data to unexpected hosts; or allow code execution or privilege escalation.

Out of scope: vulnerabilities in the official Claude Code or Codex CLIs, in Anthropic or OpenAI services, or in Windows itself. Please report those to their respective vendors.
