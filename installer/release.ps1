# Cuts a GitHub release for the version in Cargo.toml.
#
# Binaries are NOT uploaded to GitHub: they live on the CDN, and the release
# notes link to them. The CDN filenames never change, so those links stay valid
# for every future release.
#
#   .\installer\release.ps1              # tag + release notes for the current version
#   .\installer\release.ps1 -DryRun      # show what would happen, touch nothing
#
# Requires the binaries to be built and uploaded first (installer\upload.ps1 -Build),
# so the published checksums match what the notes advertise.
param(
    [switch]$DryRun
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

$version = (Select-String -Path "$root\Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
if (-not $version) { throw "Could not read version from Cargo.toml" }
$tag = "v$version"

# owner/repo derived from the git remote, so this isn't hardcoded.
$originUrl = git -C $root remote get-url origin
if ($originUrl -match '[:/]([^/:]+)/([^/]+?)(\.git)?$') {
    $repo = "$($Matches[1])/$($Matches[2])"
} else {
    throw "Could not parse owner/repo from origin: $originUrl"
}

# Public download links come from .env so they aren't hardcoded here.
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
$base = $env:AUTOMOUSE_PUBLIC_BASE
if (-not $base) { throw "AUTOMOUSE_PUBLIC_BASE not set (see .env.example)" }

# Reuse the checksums that were published with the binaries.
$sumsPath = Join-Path $root 'dist\SHA256SUMS.txt'
if (-not (Test-Path $sumsPath)) {
    throw "dist\SHA256SUMS.txt not found. Run installer\upload.ps1 -Build first."
}
$sums = @{}
foreach ($line in Get-Content $sumsPath) {
    if ($line -match '^\s*([0-9a-fA-F]{64})\s+(.+?)\s*$') { $sums[$Matches[2]] = $Matches[1] }
}

$notes = @"
## Download

| | |
|---|---|
| **Installer** | [AutoMouse-Setup.exe]($base/AutoMouse-Setup.exe) |
| **Portable** | [AutoMouse.exe]($base/AutoMouse.exe) |

Downloads are hosted on the CDN, not attached here. These links always serve the
latest build.

## Checksums (SHA-256)

``````
$(($sums.Keys | Sort-Object | ForEach-Object { "$($sums[$_])  $_" }) -join "`n")
``````

Verify with [``verify.ps1``](https://github.com/$repo/blob/$tag/verify.ps1):

``````powershell
.\verify.ps1 -Path `$HOME\Downloads
``````
"@

$notesPath = Join-Path $root 'dist\RELEASE_NOTES.md'
[System.IO.File]::WriteAllText($notesPath, $notes, (New-Object System.Text.UTF8Encoding($false)))
Write-Host "Release notes written to dist\RELEASE_NOTES.md"

if ($DryRun) {
    Write-Host "`n--- DRY RUN, nothing was tagged or published ---`n" -ForegroundColor Yellow
    Write-Host "Tag would be: $tag"
    Write-Host $notes
    return
}

# Tag the current commit if it isn't tagged already.
$ErrorActionPreference = 'Continue'
$existing = git -C $root tag --list $tag
if ($existing) {
    Write-Host "Tag $tag already exists locally."
} else {
    git -C $root tag -a $tag -m "AutoMouse $version"
    if ($LASTEXITCODE -ne 0) { throw "git tag failed" }
    Write-Host "Created tag $tag"
}

git -C $root push origin $tag
if ($LASTEXITCODE -ne 0) { throw "git push failed" }
Write-Host "Pushed tag $tag"

# winget's PATH update doesn't reach already-open shells, so look in the
# install location too.
$gh = (Get-Command gh -ErrorAction SilentlyContinue).Source
if (-not $gh) {
    $gh = @(
        "$env:ProgramFiles\GitHub CLI\gh.exe",
        "${env:ProgramFiles(x86)}\GitHub CLI\gh.exe",
        "$env:LOCALAPPDATA\Programs\GitHub CLI\gh.exe"
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
}
if (-not $gh) {
    throw "gh CLI not found. Install it with: winget install GitHub.cli"
}

& $gh auth status *> $null
if ($LASTEXITCODE -ne 0) {
    throw "gh is not logged in. Run once:  gh auth login   (or set GH_TOKEN in .env)"
}

& $gh release view $tag --repo $repo *> $null
if ($LASTEXITCODE -eq 0) {
    & $gh release edit $tag --repo $repo --title "AutoMouse $version" --notes-file $notesPath
} else {
    & $gh release create $tag --repo $repo --title "AutoMouse $version" --notes-file $notesPath
}
if ($LASTEXITCODE -ne 0) { throw "gh release failed" }
Write-Host "`nRelease published: https://github.com/$repo/releases/tag/$tag" -ForegroundColor Green
