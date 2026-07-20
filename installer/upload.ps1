# Uploads the installer and the portable exe to the CDN over SSH.
#
#   .\installer\upload.ps1            # upload what's already built
#   .\installer\upload.ps1 -Build     # build first, then upload
#
# Deployment details come from .env at the repo root (see .env.example).
# Environment variables and the -Target / -RemoteDir parameters override it.
param(
    [string]$Target,
    [string]$RemoteDir,
    [switch]$Build
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

# Local file -> name it gets on the CDN.
$artifacts = [ordered]@{
    'dist\AutoMouse-Setup.exe'    = 'AutoMouse-Setup.exe'
    'target\release\automouse.exe' = 'AutoMouse.exe'
}

# Load .env without clobbering variables already set in the environment.
$envFile = Join-Path $root '.env'
if (Test-Path $envFile) {
    foreach ($line in Get-Content $envFile) {
        $line = $line.Trim()
        if (-not $line -or $line.StartsWith('#')) { continue }
        $i = $line.IndexOf('=')
        if ($i -lt 1) { continue }
        $key = $line.Substring(0, $i).Trim()
        $val = $line.Substring($i + 1).Trim().Trim('"', "'")
        if (-not [Environment]::GetEnvironmentVariable($key)) {
            Set-Item -Path "Env:$key" -Value $val
        }
    }
}

if (-not $Target) { $Target = $env:AUTOMOUSE_UPLOAD_TARGET }
if (-not $RemoteDir) { $RemoteDir = $env:AUTOMOUSE_UPLOAD_DIR }
$publicBase = $env:AUTOMOUSE_PUBLIC_BASE

if (-not $Target -or -not $RemoteDir) {
    throw "Missing upload settings. Copy .env.example to .env and fill it in, or pass -Target / -RemoteDir."
}

if ($Build) {
    & (Join-Path $PSScriptRoot 'build.ps1')
}

foreach ($local in $artifacts.Keys) {
    if (-not (Test-Path (Join-Path $root $local))) {
        throw "Not found: $local  (run installer\build.ps1, or pass -Build)"
    }
}

# Publish checksums alongside the binaries, and mirror them into the README so
# users can cross-check a download without trusting the CDN copy alone.
$hashes = [ordered]@{}
foreach ($local in $artifacts.Keys) {
    $hashes[$artifacts[$local]] = (Get-FileHash (Join-Path $root $local) -Algorithm SHA256).Hash.ToLower()
}

$sumsPath = Join-Path $root 'dist\SHA256SUMS.txt'
$sumsText = ($hashes.Keys | ForEach-Object { "$($hashes[$_])  $_" }) -join "`n"
[System.IO.File]::WriteAllText($sumsPath, "$sumsText`n", (New-Object System.Text.UTF8Encoding($false)))
$artifacts['dist\SHA256SUMS.txt'] = 'SHA256SUMS.txt'

$readmePath = Join-Path $root 'README.md'
if (Test-Path $readmePath) {
    $table = @('| File | SHA-256 |', '|---|---|')
    foreach ($name in $hashes.Keys) { $table += "| ``$name`` | ``$($hashes[$name])`` |" }
    $block = "<!-- checksums:start -->`n" + ($table -join "`n") + "`n<!-- checksums:end -->"
    $readme = [System.IO.File]::ReadAllText($readmePath)
    $updated = [regex]::Replace(
        $readme,
        '(?s)<!-- checksums:start -->.*?<!-- checksums:end -->',
        { $block })
    if ($updated -ne $readme) {
        [System.IO.File]::WriteAllText($readmePath, $updated, (New-Object System.Text.UTF8Encoding($false)))
        Write-Host "Updated checksum table in README.md"
    } elseif ($readme -notmatch 'checksums:start') {
        Write-Warning "README.md has no <!-- checksums:start --> block; skipped."
    }
}

# scp writes progress to stderr; don't treat that as fatal.
$ErrorActionPreference = 'Continue'

foreach ($local in $artifacts.Keys) {
    $path = Join-Path $root $local
    $name = $artifacts[$local]
    $size = '{0:N1} MB' -f ((Get-Item $path).Length / 1MB)
    Write-Host "Uploading $name ($size) to ${Target}:$RemoteDir"

    # Upload to a temp name, then move into place, so nobody can download a
    # half-written file if the transfer is interrupted.
    scp $path "${Target}:$RemoteDir/$name.part"
    if ($LASTEXITCODE -ne 0) { throw "scp failed for $name (exit $LASTEXITCODE)" }

    ssh $Target "mv -f '$RemoteDir/$name.part' '$RemoteDir/$name' && chmod 644 '$RemoteDir/$name'"
    if ($LASTEXITCODE -ne 0) { throw "remote move failed for $name (exit $LASTEXITCODE)" }
}

Write-Host ""
Write-Host "Done." -ForegroundColor Green
if ($publicBase) {
    foreach ($name in $artifacts.Values) {
        Write-Host "  $publicBase/$name" -ForegroundColor Green
    }
}
