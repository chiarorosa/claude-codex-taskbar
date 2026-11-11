<#
.SYNOPSIS
  Builds CCT's MSIX package from a release build (#5).

.DESCRIPTION
  Stages claude-codex-taskbar.exe, the logos (drawn from res\app.ico) and AppxManifest.xml
  (version from Cargo.toml), then packs them with makeappx from the Windows SDK.

  For the Microsoft Store, pass the identity Partner Center shows after the name is reserved
  (Product identity: Package/Identity/Name, Publisher, PublisherDisplayName) and upload the
  unsigned package: the Store signs it.

  For a local test, add -Sign. It creates a self-signed code-signing certificate for
  -Publisher in Cert:\CurrentUser\My (once) and signs the package with signtool. Installing it
  then needs that certificate in Local Machine > Trusted People (an administrator step); see
  packaging\README.md.

.EXAMPLE
  cargo build --release
  pwsh packaging\build-msix.ps1 -Sign
#>
[CmdletBinding()]
param(
  [string]$Exe = "target\release\claude-codex-taskbar.exe",
  [string]$OutDir = "dist-msix",
  [string]$IdentityName = "CCT.LocalTest",
  [string]$Publisher = "CN=CCT Local Test",
  [string]$PublisherDisplayName = "CCT",
  [string]$DisplayName = "CCT",
  [switch]$Sign
)

$ErrorActionPreference = "Stop"
$repo = Split-Path -Parent $PSScriptRoot

function Find-SdkTool([string]$name) {
  $roots = @("${env:ProgramFiles(x86)}\Windows Kits\10\bin", "$env:ProgramFiles\Windows Kits\10\bin")
  $hit = Get-ChildItem -Path $roots -Filter $name -Recurse -ErrorAction SilentlyContinue |
    Where-Object { $_.Directory.Name -eq "x64" } |
    Sort-Object { [version]($_.Directory.Parent.Name -replace '[^0-9.]', '') } -Descending |
    Select-Object -First 1
  if (-not $hit) { throw "$name not found: install the Windows 10/11 SDK" }
  $hit.FullName
}

# Cargo's "0.4.3" (or "0.5.0-rc.1") becomes "0.4.3.0": MSIX wants four numbers, and the
# Store wants the last one to be 0.
function Get-PackageVersion {
  $toml = Get-Content -Raw (Join-Path $repo "Cargo.toml")
  if ($toml -notmatch '(?m)^version\s*=\s*"(\d+)\.(\d+)\.(\d+)') { throw "no version in Cargo.toml" }
  "$($Matches[1]).$($Matches[2]).$($Matches[3]).0"
}

# The 256 px PNG inside res\app.ico, as bytes.
function Get-IconPng {
  $bytes = [IO.File]::ReadAllBytes((Join-Path $repo "res\app.ico"))
  $count = [BitConverter]::ToUInt16($bytes, 4)
  for ($i = 0; $i -lt $count; $i++) {
    $entry = 6 + 16 * $i
    $width = $bytes[$entry]
    $size = [BitConverter]::ToUInt32($bytes, $entry + 8)
    $offset = [BitConverter]::ToUInt32($bytes, $entry + 12)
    if ($width -eq 0) { return $bytes[$offset..($offset + $size - 1)] }
  }
  throw "res\app.ico has no 256 px image"
}

function Save-Logo([byte[]]$png, [int]$size, [string]$path) {
  Add-Type -AssemblyName System.Drawing
  $stream = [IO.MemoryStream]::new($png)
  $source = [Drawing.Image]::FromStream($stream)
  $bitmap = [Drawing.Bitmap]::new($size, $size)
  $g = [Drawing.Graphics]::FromImage($bitmap)
  $g.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
  $g.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
  $g.DrawImage($source, 0, 0, $size, $size)
  $bitmap.Save($path, [Drawing.Imaging.ImageFormat]::Png)
  $g.Dispose(); $bitmap.Dispose(); $source.Dispose(); $stream.Dispose()
}

$exePath = Resolve-Path (Join-Path $repo $Exe) -ErrorAction SilentlyContinue
if (-not $exePath) { $exePath = Resolve-Path $Exe }
$version = Get-PackageVersion
$out = if ([IO.Path]::IsPathRooted($OutDir)) { $OutDir } else { Join-Path $repo $OutDir }
$stage = Join-Path $out "stage"
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force (Join-Path $stage "Assets") | Out-Null

Copy-Item $exePath (Join-Path $stage "claude-codex-taskbar.exe")
Copy-Item (Join-Path $repo "LICENSE") $stage
$png = Get-IconPng
Save-Logo $png 44 (Join-Path $stage "Assets\Square44x44Logo.png")
Save-Logo $png 150 (Join-Path $stage "Assets\Square150x150Logo.png")
Save-Logo $png 50 (Join-Path $stage "Assets\StoreLogo.png")

$xml = { param($v) [Security.SecurityElement]::Escape($v) }
$manifest = (Get-Content -Raw (Join-Path $PSScriptRoot "AppxManifest.xml")).
  Replace("{{IDENTITY_NAME}}", (& $xml $IdentityName)).
  Replace("{{PUBLISHER}}", (& $xml $Publisher)).
  Replace("{{PUBLISHER_DISPLAY_NAME}}", (& $xml $PublisherDisplayName)).
  Replace("{{DISPLAY_NAME}}", (& $xml $DisplayName)).
  Replace("{{VERSION}}", $version)
if ($manifest -match "\{\{") { throw "AppxManifest.xml has a placeholder this script does not fill" }
[IO.File]::WriteAllText((Join-Path $stage "AppxManifest.xml"), $manifest, [Text.UTF8Encoding]::new($false))

$package = Join-Path $out "claude-codex-taskbar-$version-x64.msix"
& (Find-SdkTool "makeappx.exe") pack /d $stage /p $package /o /h SHA256
if ($LASTEXITCODE -ne 0) { throw "makeappx failed ($LASTEXITCODE)" }

if ($Sign) {
  $cert = Get-ChildItem Cert:\CurrentUser\My -CodeSigningCert |
    Where-Object { $_.Subject -eq $Publisher -and $_.NotAfter -gt (Get-Date) } |
    Select-Object -First 1
  if (-not $cert) {
    $cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject $Publisher `
      -CertStoreLocation Cert:\CurrentUser\My -KeyUsage DigitalSignature `
      -TextExtension @("2.5.29.19={text}") -NotAfter (Get-Date).AddYears(1)
  }
  & (Find-SdkTool "signtool.exe") sign /fd SHA256 /sha1 $cert.Thumbprint $package
  if ($LASTEXITCODE -ne 0) { throw "signtool failed ($LASTEXITCODE)" }
  Export-Certificate -Cert $cert -FilePath (Join-Path $out "CCT-LocalTest.cer") | Out-Null
}

Write-Output "MSIX $version -> $package"
