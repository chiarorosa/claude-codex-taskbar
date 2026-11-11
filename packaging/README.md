# MSIX package (Microsoft Store)

CCT can also ship as an MSIX package for the Microsoft Store (#5). The Store signs it and keeps
it updated, so its users get no SmartScreen warning. The portable `.exe` and zip on GitHub
Releases stay as they are.

- `AppxManifest.xml`: a full-trust desktop app (`runFullTrust`) with one startup task.
- `build-msix.ps1`: packs `target\release\claude-codex-taskbar.exe` with the logos (drawn
  from `res\app.ico`) and the manifest (version from `Cargo.toml`), using `makeappx` from the
  Windows SDK.

## What changes inside the package

- **Start with Windows** uses the package's startup task (`CCTStartup`) instead of the Run
  key, which packaged apps ignore. CCT finds out at runtime (`src/packaged.rs`); the menu item
  and the Settings switch work the same. If you turn it off in Task Manager or *Settings →
  Apps → Startup*, only you can turn it back on there.
- **Settings** stay in `%APPDATA%\claude-codex-taskbar\`. As Microsoft documents for packaged
  apps (Windows 10 1903 and later), files that already exist there, such as those of a
  portable install, are used and updated in place. Files the package creates are kept per
  package and removed when it is uninstalled.
- The Claude Code and Codex logins live outside `AppData`, so the package reads them, and the
  account switcher writes the Claude one, exactly as the portable `.exe` does.

Don't run the portable `.exe` and the package at the same time, and turn on Start with Windows
in only one of them.

## Local test

On Windows 10/11 with the Windows SDK:

```powershell
cargo build --release
pwsh packaging\build-msix.ps1 -Sign
```

`-Sign` creates a self-signed certificate for `CN=CCT Local Test` in your user's certificate
store (once) and signs `dist-msix\claude-codex-taskbar-<version>-x64.msix`. To install it,
trust that certificate once from an administrator PowerShell, then add the package:

```powershell
Import-Certificate -FilePath dist-msix\CCT-LocalTest.cer -CertStoreLocation Cert:\LocalMachine\TrustedPeople
Add-AppxPackage dist-msix\claude-codex-taskbar-<version>-x64.msix
```

Remove it with `Get-AppxPackage CCT.LocalTest | Remove-AppxPackage`, and delete the test
certificate from *Trusted People* and from `Cert:\CurrentUser\My` when you are done.

CI does the same on every pull request on a throwaway runner: pack, sign, install, launch and
remove.

## Store submission (maintainer)

1. In Partner Center, reserve the app name. Under *Product identity*, copy
   `Package/Identity/Name`, `Package/Identity/Publisher` and `Package/Properties/PublisherDisplayName`.
2. Save them as repository variables `MSIX_IDENTITY_NAME`, `MSIX_PUBLISHER` and
   `MSIX_PUBLISHER_DISPLAY_NAME` (*Settings → Secrets and variables → Actions → Variables*).
   They are not secrets.
3. Push a release tag. The Release workflow builds the unsigned package with that identity and
   keeps it as the `msix-<tag>` workflow artifact (90 days), next to the usual release files.
   Until the variables are set, the artifact carries the local test identity and the Store
   rejects it.
4. Upload the `.msix` to the submission. Listing: the privacy policy URL is
   <https://chiarorosa.github.io/claude-codex-taskbar/privacy/>, and the description must say
   that CCT is not affiliated with Anthropic or OpenAI. Check Microsoft's current Partner
   Center requirements and fees first.
