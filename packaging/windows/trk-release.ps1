<#
.SYNOPSIS
  Build the TRK PDF Flow installer (MSI) and, with -Publish, put it on GitHub as a release.

.DESCRIPTION
  1. Builds TRK-PDF-Flow.exe for Windows x64 (static C runtime: no VC++ redistributable needed).
  2. Stages it with the licences and the OCR models (assets\models).
  3. Builds dist\trk\TRK-PDF-Flow-<version>-x64.msi with WiX v5.
  4. With -Publish: tags v<version> and creates the GitHub release with the MSI attached
     (gh CLI, logged in). The app's update check reads the latest release from
     github.com/YamYam-ee/trk-pdf-flow and offers its page; the new MSI replaces the installed one.

  For an update: raise [workspace.package] version in Cargo.toml (and the internal crates'
  version = "..." in [workspace.dependencies]), commit, then run this with -Publish.

.EXAMPLE
  pwsh packaging/windows/trk-release.ps1
  pwsh packaging/windows/trk-release.ps1 -Publish -Notes "What's new"
#>
param(
  [switch] $Publish,
  [string] $Notes = '',
  [switch] $SkipBuild
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$Repo = 'YamYam-ee/trk-pdf-flow'

function Invoke-Native([string] $What, [scriptblock] $Block) {
  Write-Output "==> $What"
  & $Block
  if ($LASTEXITCODE -ne 0) { throw "$What failed with exit code $LASTEXITCODE" }
}

$Version = $null
$inPkg = $false
foreach ($line in Get-Content (Join-Path $Root 'Cargo.toml')) {
  if ($line -match '^\s*\[') { $inPkg = ($line.Trim() -eq '[workspace.package]'); continue }
  if ($inPkg -and $line -match '^\s*version\s*=\s*"([^"]+)"') { $Version = $Matches[1]; break }
}
if (-not $Version) { throw 'could not read [workspace.package] version from Cargo.toml' }
$MsiVersion = ($Version -split '-')[0]
$Target = 'x86_64-pc-windows-msvc'
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }
$Dist = Join-Path $Root 'dist\trk'
New-Item -ItemType Directory -Force -Path $Dist | Out-Null
Write-Output "TRK PDF Flow $Version"

if (-not $SkipBuild) {
  [Environment]::SetEnvironmentVariable('CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS', '-C target-feature=+crt-static')
  $env:PDFCRAFT_REQUIRE_WINRES = '1'
  $env:PDFCRAFT_BUILD_SHA = (git -C $Root rev-parse HEAD)
  $env:PDFCRAFT_BUILD_DATE = (Get-Date).ToUniversalTime().ToString('yyyy-MM-dd')
  Invoke-Native "cargo build ($Target)" { cargo build --release --locked -p pdfcraft --target $Target }
}
$Bin = Join-Path $TargetDir "$Target\release"

$Stage = Join-Path $TargetDir 'trk-package'
Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Stage | Out-Null
Copy-Item (Join-Path $Bin 'TRK-PDF-Flow.exe') $Stage
foreach ($f in 'LICENSE-MIT', 'LICENSE-APACHE', 'NOTICE') { Copy-Item (Join-Path $Root $f) $Stage }
$Models = Join-Path $Stage 'models'
Copy-Item -Recurse (Join-Path $Root 'assets\models') $Models
if (-not (Test-Path (Join-Path $Models 'text-recognition.rten'))) { throw "no OCR models in assets\models (run: cargo xtask models assets\models)" }

$Msi = Join-Path $Dist "TRK-PDF-Flow-$Version-x64.msi"
Remove-Item -Force $Msi -ErrorAction SilentlyContinue
Invoke-Native 'wix build' {
  wix build (Join-Path $PSScriptRoot 'trk-pdf-flow.wxs') (Join-Path $PSScriptRoot 'installer-ui.wxs') -arch x64 `
    -d "Version=$MsiVersion" -d "BinDir=$Stage" -d "ModelsDir=$Models" -d "IconPath=$(Join-Path $Root 'assets\app-icon\pdfcraft.ico')" `
    -o $Msi
}
Remove-Item -Force -ErrorAction SilentlyContinue ([IO.Path]::ChangeExtension($Msi, '.wixpdb'))
Get-Item $Msi | Format-Table Name, Length

if ($Publish) {
  $Tag = "v$Version"
  if (-not $Notes) { $Notes = "TRK PDF Flow $Version. Download TRK-PDF-Flow-$Version-x64.msi and run it; it replaces an installed earlier version." }
  Invoke-Native 'git push' { git -C $Root push origin HEAD }
  Invoke-Native "gh release create $Tag" { gh release create $Tag $Msi --repo $Repo --target (git -C $Root rev-parse HEAD) --title "TRK PDF Flow $Version" --notes $Notes }
}
