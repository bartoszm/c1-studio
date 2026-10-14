# Builds the Windows installer as releases are built: paths on this machine (the user
# profile, where cargo and rustup keep sources, and this checkout) are not written into
# the binary's panic messages. Output: target\release\bundle\nsis\ (or $CARGO_TARGET_DIR).
$root = Split-Path $PSScriptRoot -Parent
# One flag per \x1f-separated entry, so paths with spaces stay whole.
$env:CARGO_ENCODED_RUSTFLAGS = @(
    "--remap-path-prefix=$env:USERPROFILE=~"
    "--remap-path-prefix=$root=."
) -join [char]0x1f
# Exit codes, not $ErrorActionPreference: Windows PowerShell takes any stderr line from a
# native command as an error.
pnpm --dir "$root\app" install --frozen-lockfile
if ($LASTEXITCODE) { exit $LASTEXITCODE }
pnpm --dir "$root\app" tauri build
exit $LASTEXITCODE
