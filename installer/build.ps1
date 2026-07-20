# Builds the release binary and packages it into dist\AutoMouse-<ver>-Setup.exe
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

$iscc = @(
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1

if (-not $iscc) {
    throw "Inno Setup not found. Install it with: winget install JRSoftware.InnoSetup"
}

Write-Host "Building release binary..."
Push-Location $root
try {
    # cargo and ISCC write progress to stderr; don't treat that as fatal.
    $ErrorActionPreference = 'Continue'
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

    Write-Host "Compiling installer..."
    & $iscc "$PSScriptRoot\automouse.iss"
    if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }
} finally {
    Pop-Location
}

Get-ChildItem "$root\dist\*.exe" | ForEach-Object {
    "{0}  ({1:N1} MB)" -f $_.FullName, ($_.Length / 1MB)
}
