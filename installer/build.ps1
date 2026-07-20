# Builds the release binary and packages it into dist\AutoMouse-Setup.exe
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

# Cargo.toml is the single source of truth for the version.
$version = (Select-String -Path "$root\Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $version) { throw "Could not read version from Cargo.toml" }

$iscc = @(
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "$env:ProgramFiles\Inno Setup 6\ISCC.exe"
) | Where-Object { Test-Path $_ } | Select-Object -First 1

if (-not $iscc) {
    throw "Inno Setup not found. Install it with: winget install JRSoftware.InnoSetup"
}

Write-Host "Building AutoMouse $version..."
Push-Location $root
try {
    # cargo and ISCC write progress to stderr; don't treat that as fatal.
    $ErrorActionPreference = 'Continue'
    cargo build --release
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

    Write-Host "Compiling installer..."
    & $iscc "/DAppVersion=$version" "$PSScriptRoot\automouse.iss"
    if ($LASTEXITCODE -ne 0) { throw "ISCC failed" }
} finally {
    Pop-Location
}

Get-ChildItem "$root\dist\*.exe" | ForEach-Object {
    "{0}  ({1:N1} MB)" -f $_.FullName, ($_.Length / 1MB)
}
