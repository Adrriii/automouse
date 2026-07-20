# Bumps the version, publishes the binaries, and cuts a GitHub release.
#
# Binaries are NOT uploaded to GitHub: they live on the CDN, and the release
# notes link to them. The CDN filenames never change, so those links stay valid
# for every future release.
#
#   .\installer\release.ps1              # asks which part of the version to bump
#   .\installer\release.ps1 -Bump minor  # non-interactive
#   .\installer\release.ps1 -Bump none   # re-release the current version
#   .\installer\release.ps1 -DryRun      # show what would happen, touch nothing
#
# Bumping rewrites Cargo.toml (the single source of truth), rebuilds, uploads to
# the CDN, commits, tags, and publishes the release.
param(
    [ValidateSet('major', 'minor', 'patch', 'none')]
    [string]$Bump,
    [switch]$DryRun,
    # Skip the confirmation prompt (for CI or when you're sure).
    [switch]$Yes
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot

$current = (Select-String -Path "$root\Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' |
    Select-Object -First 1).Matches[0].Groups[1].Value
if ($current -notmatch '^(\d+)\.(\d+)\.(\d+)$') {
    throw "Could not read a semver version from Cargo.toml (got '$current')"
}
$major, $minor, $patch = [int]$Matches[1], [int]$Matches[2], [int]$Matches[3]

function Get-Bumped([string]$part) {
    switch ($part) {
        'major' { "$($major + 1).0.0" }
        'minor' { "$major.$($minor + 1).0" }
        'patch' { "$major.$minor.$($patch + 1)" }
        default { $current }
    }
}

if (-not $Bump) {
    Write-Host ""
    Write-Host "Current version: $current" -ForegroundColor Cyan
    Write-Host "  [1] patch  -> $(Get-Bumped 'patch')   (bug fixes)"
    Write-Host "  [2] minor  -> $(Get-Bumped 'minor')   (new features)"
    Write-Host "  [3] major  -> $(Get-Bumped 'major')   (breaking changes)"
    Write-Host "  [4] keep   -> $current   (re-release, e.g. fixing the notes)"
    Write-Host ""
    $choice = Read-Host "Which bump? [1]"
    if (-not $choice) { $choice = '1' }
    $Bump = switch ($choice.Trim().ToLower()) {
        { $_ -in '1', 'patch' } { 'patch' }
        { $_ -in '2', 'minor' } { 'minor' }
        { $_ -in '3', 'major' } { 'major' }
        { $_ -in '4', 'keep', 'none' } { 'none' }
        default { throw "Unrecognized choice '$choice'" }
    }
}

$version = Get-Bumped $Bump
$tag = "v$version"
Write-Host "Releasing $tag" -ForegroundColor Cyan

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

if ($DryRun) {
    Write-Host "`n--- DRY RUN, nothing was changed ---" -ForegroundColor Yellow
    if ($version -eq $current) {
        Write-Host "Would re-release the current version $current"
    } else {
        Write-Host "Would set Cargo.toml version to $version ($Bump bump from $current)"
    }
    Write-Host "Would build, upload to $base, commit, tag $tag, and publish the release."
    return
}

# This publishes to the CDN and GitHub, so confirm before touching anything.
if (-not $Yes) {
    Write-Host ""
    Write-Host "About to release $tag :" -ForegroundColor Yellow
    if ($version -ne $current) {
        Write-Host "  - set version $current -> $version in Cargo.toml and the README badge"
    } else {
        Write-Host "  - keep version $current (re-release)"
    }
    Write-Host "  - build, then upload the binaries to $base"
    Write-Host "  - commit, tag $tag, and publish the GitHub release"
    Write-Host ""
    $confirm = Read-Host "Proceed? [y/N]"
    if ($confirm.Trim().ToLower() -notin 'y', 'yes') {
        Write-Host "Aborted, nothing changed." -ForegroundColor Yellow
        return
    }
}

# Apply the new version. Cargo.toml is the source of truth; the README badge is
# the only other place the number is written out.
if ($version -ne $current) {
    $cargoPath = Join-Path $root 'Cargo.toml'
    $cargo = [System.IO.File]::ReadAllText($cargoPath)
    $cargo = [regex]::new('(?m)^version\s*=\s*"[^"]+"').Replace($cargo, "version = `"$version`"", 1)
    [System.IO.File]::WriteAllText($cargoPath, $cargo, (New-Object System.Text.UTF8Encoding($false)))

    $readmePath = Join-Path $root 'README.md'
    if (Test-Path $readmePath) {
        $readme = [System.IO.File]::ReadAllText($readmePath)
        $readme = [regex]::new('release-v[0-9]+\.[0-9]+\.[0-9]+').Replace($readme, "release-v$version")
        [System.IO.File]::WriteAllText($readmePath, $readme, (New-Object System.Text.UTF8Encoding($false)))
    }
    Write-Host "Bumped version $current -> $version"
}

# Build and publish the binaries first, so the checksums in the notes describe
# what people will actually download. This also refreshes the README table.
& (Join-Path $PSScriptRoot 'upload.ps1') -Build
if ($LASTEXITCODE -ne 0 -and $LASTEXITCODE) { throw "build/upload failed" }

# Reuse the checksums that were just published.
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

| Installer | Portable |
|---|---|
| [AutoMouse-Setup.exe]($base/AutoMouse-Setup.exe) | [AutoMouse.exe]($base/AutoMouse.exe) |

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

$ErrorActionPreference = 'Continue'

# Commit the version bump and refreshed checksums so the tag points at them.
git -C $root add Cargo.toml Cargo.lock README.md
if (git -C $root diff --cached --name-only) {
    git -C $root commit -q -m "Release $tag"
    if ($LASTEXITCODE -ne 0) { throw "git commit failed" }
    git -C $root push -q origin HEAD
    if ($LASTEXITCODE -ne 0) { throw "git push failed" }
    Write-Host "Committed and pushed $tag"
}

# Tag the current commit if it isn't tagged already.
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
