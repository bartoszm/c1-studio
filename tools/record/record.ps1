# Records the feature clips for docs/FEATURES.md, and the README's tour (docs/media/demo.gif).
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
$order = 'search', 'browse', 'sort', 'match', 'edit', 'hidden', 'trash', 'keywords', 'albums', 'analysis', 'alike', 'map', 'refine', 'trips', 'history', 'folders', 'demo'
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
    # The README's tour goes beside the README's stills, at a low rate to keep it small.
    $gif = if ($name -eq 'demo') { Join-Path $media '..\demo.gif' } else { Join-Path $media "$name.gif" }
    $rate = if ($name -eq 'demo') { 3 } else { $Fps }
    python (Join-Path $here 'assemble.py') $frames $gif --width $Width --fps $rate
    if ($LASTEXITCODE) { throw "assembling $name failed" }
    if ($Mp4) {
        python (Join-Path $here 'to_mp4.py') $frames (Join-Path $media "$name.mp4")
        if ($LASTEXITCODE) { throw "encoding $name failed" }
    }
}
