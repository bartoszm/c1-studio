# Starts a C1 Studio instance for recording, on a copy of the newest catalog backup.
#
# usage: tools\record\start.ps1 [-Exe <c1studio.exe>] [-Work <dir>] [-NewCopy] [-Port 9223]
#
# The copy lives in <Work>\cat with a junction to the live catalog's Cache (thumbnails and
# previews), so the instance shows real photos but can only ever write to the copy. Its
# index, settings and WebView data live under <Work> too, so the instance is separate from
# the one you use. The work dir is kept between runs: the first start indexes the whole
# catalog, later ones only sync. -NewCopy replaces the catalog copy with the newest backup.
param(
    [string]$Exe = (Join-Path $PSScriptRoot '..\..\target\release\c1studio.exe'),
    [string]$Work = (Join-Path $env:TEMP 'c1studio-record'),
    [string]$Live = 'C:\c1_catalogs\fuji\fiuji.cocatalogdb',
    [string]$Backups = (Join-Path $env:LOCALAPPDATA 'c1studio'),
    [int]$Port = 9223,
    [switch]$NewCopy
)
$ErrorActionPreference = 'Stop'

try {
    Invoke-RestMethod "http://127.0.0.1:$Port/json/version" -TimeoutSec 1 | Out-Null
    Write-Host "an instance already listens on $Port"
    return
} catch {}
if (-not (Test-Path $Exe)) { throw "no app at ${Exe}: build it (cd app; pnpm tauri build --no-bundle) or pass -Exe" }

$cat = Join-Path $Work 'cat'
$copy = Join-Path $cat (Split-Path $Live -Leaf)
if ((Resolve-Path -LiteralPath $Live).Path -eq [IO.Path]::GetFullPath($copy)) { throw 'the copy must not be the live catalog' }
New-Item -ItemType Directory -Force $cat | Out-Null

if ($NewCopy -or -not (Test-Path $copy)) {
    $stem = [IO.Path]::GetFileNameWithoutExtension($Live)
    $backup = Get-ChildItem $Backups -Recurse -Filter "$stem.*.cocatalogdb" |
        Where-Object { $_.Directory.Name -eq 'backups' -and $_.Name -notmatch 'before-restore' } |
        Sort-Object Name -Descending | Select-Object -First 1
    if (-not $backup) { throw "no backup of $stem under $Backups" }
    Write-Host "copying $($backup.FullName)"
    Copy-Item -LiteralPath $backup.FullName $copy -Force
}

$cache = Join-Path $cat 'Cache'
if (-not (Test-Path $cache)) {
    New-Item -ItemType Junction -Path $cache -Target (Join-Path (Split-Path $Live) 'Cache') | Out-Null
}

$env:C1_CATALOG = $copy
$env:C1_DATA_DIR = Join-Path $Work 'data'
$env:APPDATA = Join-Path $Work 'appdata'
$env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $Work 'wv'
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=$Port"
Start-Process (Resolve-Path $Exe) -ArgumentList ('"' + $copy + '"') | Out-Null

foreach ($i in 1..60) {
    try {
        Invoke-RestMethod "http://127.0.0.1:$Port/json/version" -TimeoutSec 1 | Out-Null
        Write-Host "instance on $copy, CDP on $Port"
        return
    } catch { Start-Sleep -Milliseconds 500 }
}
throw "the instance did not open CDP on $Port"
