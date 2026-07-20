# Verifies downloaded AutoMouse files against the published SHA-256 checksums.
#
#   .\verify.ps1                          # check files next to this script
#   .\verify.ps1 -Path $HOME\Downloads    # check files in another folder
#   .\verify.ps1 -SumsFile .\SHA256SUMS.txt   # offline: use a local checksum file
#
# Exits 0 if every file found matches, 1 otherwise.
param(
    [string]$Path = $PWD,
    [string]$SumsUrl = 'https://rhythmgamers.net/cdn/SHA256SUMS.txt',
    [string]$SumsFile
)

$ErrorActionPreference = 'Stop'

if ($SumsFile) {
    if (-not (Test-Path $SumsFile)) { throw "Checksum file not found: $SumsFile" }
    $lines = Get-Content $SumsFile
    Write-Host "Checksums: $SumsFile"
} else {
    Write-Host "Fetching checksums from $SumsUrl"
    try {
        $lines = (Invoke-WebRequest -Uri $SumsUrl -UseBasicParsing -TimeoutSec 30).Content -split "`r?`n"
    } catch {
        throw "Could not download checksums: $($_.Exception.Message)"
    }
}

# Standard sha256sum format: "<64 hex>  <filename>"
$expected = @{}
foreach ($line in $lines) {
    if ($line -match '^\s*([0-9a-fA-F]{64})\s+\*?(.+?)\s*$') {
        $expected[$Matches[2]] = $Matches[1].ToLower()
    }
}
if ($expected.Count -eq 0) { throw "No checksums found." }

Write-Host ""
$checked = 0
$failed = 0

foreach ($name in $expected.Keys) {
    $file = Join-Path $Path $name
    if (-not (Test-Path $file)) {
        Write-Host ("  {0,-22} not found, skipping" -f $name) -ForegroundColor DarkGray
        continue
    }
    $checked++
    $actual = (Get-FileHash $file -Algorithm SHA256).Hash.ToLower()
    if ($actual -eq $expected[$name]) {
        Write-Host ("  {0,-22} OK" -f $name) -ForegroundColor Green
    } else {
        $failed++
        Write-Host ("  {0,-22} MISMATCH" -f $name) -ForegroundColor Red
        Write-Host "      expected $($expected[$name])"
        Write-Host "      actual   $actual"
    }
}

Write-Host ""
if ($checked -eq 0) {
    Write-Host "No AutoMouse files found in $Path" -ForegroundColor Yellow
    exit 1
} elseif ($failed -gt 0) {
    Write-Host "$failed of $checked file(s) FAILED verification. Do not run them." -ForegroundColor Red
    exit 1
} else {
    Write-Host "All $checked file(s) verified." -ForegroundColor Green
    exit 0
}
