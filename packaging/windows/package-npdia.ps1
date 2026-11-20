# SPDX-FileCopyrightText: MailCraft contributors
# SPDX-License-Identifier: MIT OR Apache-2.0

[CmdletBinding()]
param(
  [Parameter(Mandatory)] [ValidatePattern('^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?$')] [string] $Version,
  [Parameter(Mandatory)] [ValidatePattern('^[a-fA-F0-9]{40}$')] [string] $SourceCommit,
  [string] $Compiler = 'ISCC.exe',
  [string] $OutputDirectory = 'dist\installer'
)

$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$BaseCommit = 'adab7f7f9c02e52f15cd39aee325dfc0b1f1cc04'
$SourceCommit = $SourceCommit.ToLowerInvariant()
$ActualCommit = git -C $Root rev-parse HEAD
if ($LASTEXITCODE -ne 0 -or $ActualCommit -cne $SourceCommit) { throw 'SourceCommit does not match the checked-out commit.' }
git -C $Root merge-base --is-ancestor $BaseCommit $SourceCommit
if ($LASTEXITCODE -ne 0) { throw 'The source commit is not based on the reconstruction commit.' }
$ChangedFiles = git -C $Root status --porcelain
if ($LASTEXITCODE -ne 0 -or $ChangedFiles) { throw 'Installer builds require a clean source checkout.' }

$Manifest = Get-Content -Raw (Join-Path $Root 'Cargo.toml')
$ManifestVersion = [regex]::Match($Manifest, '(?ms)^\[workspace\.package\]\r?\n[^\[]*?^version\s*=\s*"([^"]+)"').Groups[1].Value
if ($ManifestVersion -cne $Version) { throw "Installer version $Version does not match the Cargo workspace version $ManifestVersion." }

$ISCC = (Get-Command -CommandType Application $Compiler).Source
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory, $Root)
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$TargetDirectory = if ($env:CARGO_TARGET_DIR) { [IO.Path]::GetFullPath($env:CARGO_TARGET_DIR, $Root) } else { Join-Path $Root 'target' }
$env:CARGO_TARGET_DIR = $TargetDirectory
$env:MAILCRAFT_VERSION = $Version
$env:MAILCRAFT_BUILD_SHA = $SourceCommit
$env:MAILCRAFT_BUILD_DATE = git -C $Root show -s --format=%cs $SourceCommit
if ($LASTEXITCODE -ne 0) { throw 'Could not read the source date.' }

Push-Location $Root
try {
  & pwsh -NoProfile -File (Join-Path $PSScriptRoot 'package.ps1') -Arch x64 -BuildOnly
  if ($LASTEXITCODE -ne 0) { throw "The Windows build failed with exit code $LASTEXITCODE." }
} finally {
  Pop-Location
}

$BinDirectory = Join-Path $TargetDirectory 'x86_64-pc-windows-msvc\release'
foreach ($program in @(@('mailcraft.exe', 2), @('mailcraft-cli.exe', 3))) {
  $Bin = [IO.File]::ReadAllBytes((Join-Path $BinDirectory $program[0]))
  if ($Bin.Length -lt 64 -or [BitConverter]::ToUInt16($Bin, 0) -ne 0x5A4D) { throw "$($program[0]) has no DOS header." }
  $PeOffset = [BitConverter]::ToInt32($Bin, 0x3C)
  if ($PeOffset -lt 64 -or $PeOffset -gt ($Bin.Length - 0x5E) -or
    [BitConverter]::ToUInt32($Bin, $PeOffset) -ne 0x4550 -or
    [BitConverter]::ToUInt16($Bin, $PeOffset + 4) -ne 0x8664 -or
    [BitConverter]::ToUInt16($Bin, $PeOffset + 0x5C) -ne $program[1]) {
    throw "$($program[0]) is not a Windows x64 executable with the expected subsystem."
  }
}

$NumericVersion = ($Version -split '-', 2)[0] + '.0'
& $ISCC "/DAppVersion=$Version" "/DNumericVersion=$NumericVersion" "/DBinDir=$BinDirectory" "/DOutputDir=$OutputDirectory" (Join-Path $PSScriptRoot 'npdia-installer.iss')
if ($LASTEXITCODE -ne 0) { throw "Inno Setup compilation failed with exit code $LASTEXITCODE." }

$Installer = Get-Item (Join-Path $OutputDirectory "MailCraft_NPDIA_Custom-$Version-windows-x64.exe")
$SourceDiff = New-TemporaryFile
try {
  git -C $Root diff --binary "--output=$($SourceDiff.FullName)" $BaseCommit $SourceCommit
  if ($LASTEXITCODE -ne 0) { throw 'Could not record the source delta.' }
  $ChangeCommits = @(git -C $Root log --reverse --format=%H "$BaseCommit..$SourceCommit")
  if ($LASTEXITCODE -ne 0) { throw 'Could not read the source changes.' }
  $Hash = (Get-FileHash -Algorithm SHA256 $Installer.FullName).Hash.ToLowerInvariant()
  [ordered]@{
    product = 'MailCraft_NPDIA_Custom'
    version = $Version
    source_commit = $SourceCommit
    base_commit = $BaseCommit
    change_commits = $ChangeCommits
    source_delta_sha256 = (Get-FileHash -Algorithm SHA256 $SourceDiff.FullName).Hash.ToLowerInvariant()
    target = 'x86_64-pc-windows-msvc'
    inno_setup = (Get-Item $ISCC).VersionInfo.ProductVersion
    installer = $Installer.Name
    installer_sha256 = $Hash
  } | ConvertTo-Json | Set-Content -Encoding utf8 (Join-Path $OutputDirectory 'BUILDINFO.json')
  Set-Content -Encoding ascii (Join-Path $OutputDirectory 'SHA256SUMS.txt') "$Hash  $($Installer.Name)"
} finally {
  Remove-Item -LiteralPath $SourceDiff.FullName
}
Get-Item $Installer.FullName
