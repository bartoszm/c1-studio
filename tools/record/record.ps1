# Records the feature clips for docs/FEATURES.md.
#
# usage: tools\record\record.ps1 [-Only search,map] [-Mp4] [-FromFrames] [-Exe <c1studio.exe>] [-Work <dir>] [-Port 9223]
#
# Runs each scenario in scenarios\ against the recording instance (start.ps1 starts it if
# it isn't running), keeps the frames in <Work>\frames\<name>\, and writes
# docs\media\features\<name>.gif (and <name>.mp4 with -Mp4, which needs ffmpeg).
# Frames from an earlier run can be encoded again without recording: -FromFrames.
param(
    [string[]]$Only,
    [string]$Exe = (Join-Path $PSScriptRoot '..\..\target\release\c1studio.exe'),
    [switch]$Mp4,
    [switch]$FromFrames,
    [string]$Work = (Join-Path $env:TEMP 'c1studio-record'),
    [int]$Port = 9223,
    [int]$Width = 960,
    [int]$Fps = 10
)
$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
$media = Join-Path $here '..\..\docs\media\features'
$order = 'search', 'browse', 'sort', 'match', 'edit', 'hidden', 'keywords', 'albums', 'analysis', 'alike', 'map', 'trips', 'history'
$names = if ($Only) { $Only } else { $order }

if (-not $FromFrames) { & (Join-Path $here 'start.ps1') -Exe $Exe -Work $Work -Port $Port }

foreach ($name in $names) {
    $scenario = Join-Path $here "scenarios\$name.mjs"
    if (-not (Test-Path $scenario)) { throw "no scenario $scenario" }
    $frames = Join-Path $Work "frames\$name"
    if (-not $FromFrames) {
        Write-Host "== $name"
        node (Join-Path $here 'drive.mjs') $scenario $frames --port $Port
        if ($LASTEXITCODE) { throw "$name failed" }
    }
    python (Join-Path $here 'assemble.py') $frames (Join-Path $media "$name.gif") --width $Width --fps $Fps
    if ($LASTEXITCODE) { throw "assembling $name failed" }
    if ($Mp4) {
        python (Join-Path $here 'to_mp4.py') $frames (Join-Path $media "$name.mp4")
        if ($LASTEXITCODE) { throw "encoding $name failed" }
    }
}
