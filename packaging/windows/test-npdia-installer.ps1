# SPDX-FileCopyrightText: MailCraft contributors
# SPDX-License-Identifier: MIT OR Apache-2.0

[CmdletBinding()]
param(
  [Parameter(Mandatory)] [string] $Installer,
  [Parameter(Mandatory)] [string] $Version,
  [string] $PreviousInstaller,
  [switch] $TestImap
)

$ErrorActionPreference = 'Stop'
$Installer = (Resolve-Path -LiteralPath $Installer).Path
if ($PreviousInstaller) { $PreviousInstaller = (Resolve-Path -LiteralPath $PreviousInstaller).Path }
$Results = Split-Path -Parent $Installer
$Registry = 'HKCU:\Software\NPDIA\MailCraft_NPDIA_Custom'
$TestProfileDirectory = Join-Path $env:APPDATA 'MailCraft_NPDIA_Custom'
if ((Test-Path -LiteralPath $Registry) -or (Test-Path -LiteralPath $TestProfileDirectory)) {
  throw 'Run installer tests in a disposable Windows account with no MailCraft_NPDIA_Custom installation or profile.'
}

function Invoke-Installer([string] $File, [string[]] $Arguments) {
  $StartInfo = [Diagnostics.ProcessStartInfo]::new($File)
  $StartInfo.UseShellExecute = $false
  foreach ($Argument in $Arguments) { $StartInfo.ArgumentList.Add($Argument) }
  $Process = [Diagnostics.Process]::Start($StartInfo)
  try {
    if (-not $Process.WaitForExit(180000)) {
      $Process.Kill($true)
      throw "Timed out while running $File."
    }
    return $Process.ExitCode
  } finally {
    $Process.Dispose()
  }
}

$Root = Join-Path ([IO.Path]::GetTempPath()) ([Guid]::NewGuid().ToString('N'))
$InstallDirectory = Join-Path $Root 'Custom install path'
$OtherDirectory = Join-Path $Root 'Existing upstream'
$App = Join-Path $InstallDirectory 'mailcraft.exe'
$Cli = Join-Path $InstallDirectory 'mailcraft-cli.exe'
$Uninstaller = Join-Path $InstallDirectory 'unins000.exe'
$Shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'MailCraft_NPDIA_Custom\MailCraft_NPDIA_Custom.lnk'
$Silent = @('/SP-', '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART')
if (Test-Path -LiteralPath $Shortcut) { throw 'The test account has an existing MailCraft_NPDIA_Custom shortcut.' }

function Invoke-Setup([string] $File, [string[]] $Options, [string] $Name, [switch] $ExpectFailure) {
  $Log = Join-Path $Results "$Name.log"
  $ExitCode = Invoke-Installer $File ($Silent + @("/LOG=$Log") + $Options)
  if (($ExitCode -eq 0) -eq [bool]$ExpectFailure) { throw "$Name returned exit code $ExitCode. See $Log." }
}

function Assert-Installed {
  $ActualApp = (Get-ItemProperty -LiteralPath $Registry -Name 'Executable').Executable
  if ($ActualApp -ine $App -or -not (Test-Path -LiteralPath $App) -or -not (Test-Path -LiteralPath $Shortcut)) {
    throw 'The installer did not preserve its chosen path, executable registration or Start Menu shortcut.'
  }
  if ((Invoke-Installer $App @('--version')) -ne 0) { throw 'The installed desktop executable could not run.' }
}

$OtherApp = Join-Path $OtherDirectory 'mailcraft.exe'
$ProfileMarker = Join-Path $TestProfileDirectory 'mailbox.mailcraft'
$ProfileCreated = $false
New-Item -ItemType Directory -Path $OtherDirectory | Out-Null
try {
  New-Item -ItemType Directory -Path $TestProfileDirectory | Out-Null
  $ProfileCreated = $true
  Set-Content -LiteralPath $OtherApp -Value 'Synthetic upstream installation'
  Set-Content -LiteralPath $ProfileMarker -Value 'Synthetic mailbox profile'
  $OtherHash = (Get-FileHash -LiteralPath $OtherApp).Hash
  $ProfileHash = (Get-FileHash -LiteralPath $ProfileMarker).Hash

  Invoke-Setup $Installer @("/DIR=$OtherDirectory", '/LANG=english') 'rejected-upstream-folder' -ExpectFailure
  if ((Get-FileHash -LiteralPath $OtherApp).Hash -ne $OtherHash) { throw 'The upstream fixture was modified.' }

  $FirstInstaller = if ($PreviousInstaller) { $PreviousInstaller } else { $Installer }
  Invoke-Setup $FirstInstaller @("/DIR=$InstallDirectory", '/LANG=korean') 'fresh-install-ko'
  Assert-Installed
  if ($PreviousInstaller) {
    $PreviousCliVersion = & $Cli --version
    if ($LASTEXITCODE -ne 0 -or $PreviousCliVersion -cnotmatch '^mailcraft-cli \S+$') { throw 'The previous CLI could not report its version.' }
  }

  Invoke-Setup $Installer @('/LANG=english') 'upgrade-en'
  Assert-Installed
  $CliVersion = & $Cli --version
  if ($LASTEXITCODE -ne 0 -or $CliVersion -cne "mailcraft-cli $Version") { throw "Unexpected CLI result: $CliVersion" }
  if ((Get-FileHash -LiteralPath $ProfileMarker).Hash -ne $ProfileHash) { throw 'The upgrade modified the synthetic profile.' }

  if ($TestImap) {
    python (Join-Path $PSScriptRoot 'test-npdia-imap.py') --cli $Cli --output (Join-Path $Results 'installer-imap.json')
    if ($LASTEXITCODE -ne 0) { throw 'The installed CLI failed the IMAP sign-in and first sync regression test.' }
  }

  if ($PreviousInstaller) {
    Invoke-Setup $PreviousInstaller @('/LANG=english') 'rollback-en'
    Assert-Installed
    $RolledBackVersion = & $Cli --version
    if ($LASTEXITCODE -ne 0 -or $RolledBackVersion -cne $PreviousCliVersion) { throw "Unexpected rollback CLI version: $RolledBackVersion" }
    if ((Get-FileHash -LiteralPath $ProfileMarker).Hash -ne $ProfileHash) { throw 'The rollback modified the synthetic profile.' }
  }

  Invoke-Setup $Uninstaller @() 'uninstall'
  if ((Test-Path -LiteralPath $Registry) -or (Test-Path -LiteralPath $App) -or (Test-Path -LiteralPath $Shortcut)) {
    throw 'Uninstall did not remove the app, registration and shortcut.'
  }
  if ((Get-FileHash -LiteralPath $ProfileMarker).Hash -ne $ProfileHash) { throw 'Uninstall modified the synthetic profile.' }
  [ordered]@{
    windows = [Environment]::OSVersion.VersionString
    fresh_install_korean = 'passed'
    upgrade_english = if ($PreviousInstaller) { 'passed from supplied earlier installer' } else { 'passed as same-version reinstall' }
    rollback = if ($PreviousInstaller) { 'passed' } else { 'not run' }
    reject_existing_mailcraft_directory = 'passed'
    chosen_install_path = 'passed'
    cli_version = $Version
    uninstall = 'passed'
    synthetic_profile_retained = 'passed'
  } | ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $Results 'installer-smoke.json')
} finally {
  if (Test-Path -LiteralPath $Uninstaller) {
    [void] (Invoke-Installer $Uninstaller $Silent)
  }
  Remove-Item -Recurse -Force -LiteralPath $Root
  if ($ProfileCreated) { Remove-Item -Recurse -Force -LiteralPath $TestProfileDirectory }
}
