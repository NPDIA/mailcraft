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
if ($PreviousInstaller -and (Get-FileHash -LiteralPath $PreviousInstaller).Hash -eq (Get-FileHash -LiteralPath $Installer).Hash) {
  throw 'The upgrade fixture requires two distinct installer binaries.'
}
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
$FixtureState = Join-Path $Root 'upgrade-profile.json'
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

function Test-InstallationRemains {
  (Test-Path -LiteralPath $Registry) -or (Test-Path -LiteralPath $App) -or (Test-Path -LiteralPath $Shortcut)
}

function Invoke-ProfileFixture([string] $Stage) {
  python (Join-Path $PSScriptRoot 'test-npdia-upgrade-profile.py') $Stage --profile $TestProfileDirectory --state $FixtureState --cli $Cli --windows-credentials
  if ($LASTEXITCODE -ne 0) { throw "The structured upgrade profile check failed during '$Stage'." }
}

function Assert-DesktopProfile {
  python (Join-Path $PSScriptRoot 'test-npdia-upgrade-desktop.py') --app $App --profile $TestProfileDirectory --state $FixtureState
  if ($LASTEXITCODE -ne 0) { throw 'The installed desktop did not reopen the synthetic profile.' }
}

$OtherApp = Join-Path $OtherDirectory 'mailcraft.exe'
$ProfileMarker = Join-Path $TestProfileDirectory 'mailbox.mailcraft'
$ProfileCreated = $false
$TestFailed = $false
New-Item -ItemType Directory -Path $OtherDirectory | Out-Null
try {
  New-Item -ItemType Directory -Path $TestProfileDirectory | Out-Null
  $ProfileCreated = $true
  Set-Content -LiteralPath $OtherApp -Value 'Synthetic upstream installation'
  $OtherHash = (Get-FileHash -LiteralPath $OtherApp).Hash
  if (-not $PreviousInstaller) {
    Set-Content -LiteralPath $ProfileMarker -Value 'Synthetic mailbox profile'
    $ProfileHash = (Get-FileHash -LiteralPath $ProfileMarker).Hash
  }

  Invoke-Setup $Installer @("/DIR=$OtherDirectory", '/LANG=english') 'rejected-upstream-folder' -ExpectFailure
  if ((Get-FileHash -LiteralPath $OtherApp).Hash -ne $OtherHash) { throw 'The upstream fixture was modified.' }

  $FirstInstaller = if ($PreviousInstaller) { $PreviousInstaller } else { $Installer }
  Invoke-Setup $FirstInstaller @("/DIR=$InstallDirectory", '/LANG=korean') 'fresh-install-ko'
  Assert-Installed
  if ($PreviousInstaller) {
    $PreviousCliVersion = & $Cli --version
    if ($LASTEXITCODE -ne 0 -or $PreviousCliVersion -cnotmatch '^mailcraft-cli \S+$') { throw 'The previous CLI could not report its version.' }
    $PreviousDesktopHash = (Get-FileHash -LiteralPath $App).Hash
    Invoke-ProfileFixture 'seed'
  }

  Invoke-Setup $Installer @('/LANG=english') 'upgrade-en'
  Assert-Installed
  $CliVersion = & $Cli --version
  if ($LASTEXITCODE -ne 0 -or $CliVersion -cne "mailcraft-cli $Version") { throw "Unexpected CLI result: $CliVersion" }
  if ($PreviousInstaller) {
    if ((Get-FileHash -LiteralPath $App).Hash -eq $PreviousDesktopHash) { throw 'The upgrade kept the previous desktop executable.' }
    Invoke-ProfileFixture 'open'
    Assert-DesktopProfile
  } elseif ((Get-FileHash -LiteralPath $ProfileMarker).Hash -ne $ProfileHash) {
    throw 'The upgrade modified the synthetic profile.'
  }

  if ($TestImap) {
    python (Join-Path $PSScriptRoot 'test-npdia-imap.py') --cli $Cli --output (Join-Path $Results 'installer-imap.json')
    if ($LASTEXITCODE -ne 0) { throw 'The installed CLI failed the IMAP sign-in and first sync regression test.' }
  }

  if ($PreviousInstaller) {
    Invoke-Setup $PreviousInstaller @('/LANG=english') 'rollback-en'
    Assert-Installed
    $RolledBackVersion = & $Cli --version
    if ($LASTEXITCODE -ne 0 -or $RolledBackVersion -cne $PreviousCliVersion) { throw "Unexpected rollback CLI version: $RolledBackVersion" }
    if ((Get-FileHash -LiteralPath $App).Hash -ne $PreviousDesktopHash) { throw 'The rollback did not restore the previous desktop executable.' }
    Invoke-ProfileFixture 'open'
    Assert-DesktopProfile
  }

  Invoke-Setup $Uninstaller @() 'uninstall'
  if (Test-InstallationRemains) {
    throw 'Uninstall did not remove the app, registration and shortcut.'
  }
  if ($PreviousInstaller) {
    Invoke-ProfileFixture 'files'
    $StructuredProfile = [ordered]@{
      previous_cli_version = $PreviousCliVersion
      previous_installer_sha256 = (Get-FileHash -LiteralPath $PreviousInstaller).Hash.ToLowerInvariant()
      installer_sha256 = (Get-FileHash -LiteralPath $Installer).Hash.ToLowerInvariant()
      upgraded_cli_reopened = 'passed'
      rolled_back_cli_reopened = 'passed'
      upgraded_desktop_reopened = 'passed'
      rolled_back_desktop_reopened = 'passed'
      mailbox_message_and_folder_ids = 'passed'
      message_and_draft_bodies_and_attachments = 'passed'
      outbox_record_retained = 'passed'
      account_settings_sending_alias_rule_and_reference = 'passed'
      profile_files_preserved_after_uninstall = 'passed'
      synthetic_windows_credential_retained = 'passed'
    }
  } elseif ((Get-FileHash -LiteralPath $ProfileMarker).Hash -ne $ProfileHash) {
    throw 'Uninstall modified the synthetic profile.'
  }
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
    structured_profile = $StructuredProfile
  } | ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $Results 'installer-smoke.json')
} catch {
  $TestFailed = $true
  throw
} finally {
  $UninstallFailed = Test-InstallationRemains
  try {
    # Inno's bootstrap can remain on disk briefly after a successful uninstall.
    if ($UninstallFailed) {
      Invoke-Setup $Uninstaller @() 'uninstall-cleanup'
      $UninstallFailed = Test-InstallationRemains
      if ($UninstallFailed) { throw 'Cleanup could not uninstall MailCraft from the disposable account.' }
    }
  } catch {
    if (-not $TestFailed) { throw }
    Write-Warning $_.Exception.Message
  } finally {
    try {
      if ($PreviousInstaller -and $ProfileCreated) { Invoke-ProfileFixture 'clean' }
    } finally {
      try {
        if ($ProfileCreated) { Remove-Item -Recurse -Force -LiteralPath $TestProfileDirectory }
      } finally {
        if (-not $UninstallFailed) {
          $CleanupWait = [Diagnostics.Stopwatch]::StartNew()
          while ((Test-Path -LiteralPath $InstallDirectory) -and $CleanupWait.Elapsed.TotalSeconds -lt 10) {
            Start-Sleep -Milliseconds 100
          }
          Remove-Item -Recurse -Force -LiteralPath $Root
        }
      }
    }
  }
}
