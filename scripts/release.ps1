<#
.SYNOPSIS
  Release closure: merge dev into master with --no-ff, tag the merge, push both, return.
.DESCRIPTION
  Owner-only. Run from any directory once the closure commit is on origin/dev.
  Refuses to start on a dirty tree, on an unpushed dev, or when the tag already
  exists locally or on origin. Never force-pushes and never replaces a tag.
.EXAMPLE
  & ".\scripts\release.ps1" -Version 1.19.0 -Description "Linux and rules"
#>
param(
    [Parameter(Mandatory = $true)][ValidatePattern('^\d+\.\d+\.\d+$')][string]$Version,
    [Parameter(Mandatory = $true)][string]$Description
)

$ErrorActionPreference = "Stop"
$Tag    = "v$Version"
$Main   = "master"
$Source = "dev"

# Windows PowerShell 5.1 does not throw on a failing native command, so every
# git call that changes state goes through this check.
function Invoke-Git {
    git @args
    if ($LASTEXITCODE -ne 0) { throw "git $($args -join ' ') failed with exit code $LASTEXITCODE" }
}

Push-Location (Split-Path $PSScriptRoot -Parent)
try {
    if (git status --porcelain) { throw "Working tree not clean - commit or stash first" }
    $start = (git branch --show-current).Trim()

    $null = git rev-parse -q --verify "refs/tags/$Tag"
    if ($LASTEXITCODE -eq 0) { throw "Tag $Tag already exists locally" }
    $remoteTag = git ls-remote --tags origin "refs/tags/$Tag"
    if ($LASTEXITCODE -ne 0) { throw "Cannot reach origin" }
    if ($remoteTag) { throw "Tag $Tag already exists on origin" }

    Invoke-Git fetch -q origin $Source
    if ((git rev-parse $Source) -ne (git rev-parse "origin/$Source")) {
        throw "$Source differs from origin/$Source - push $Source first"
    }

    try {
        Invoke-Git switch $Main
        Invoke-Git pull --ff-only origin $Main
        Invoke-Git merge --no-ff $Source -m "Release $Tag $([char]0x2014) $Description"
        Invoke-Git push origin $Main
        Invoke-Git tag -a $Tag -m $Tag
        Invoke-Git push origin $Tag
        Write-Host "Released $Tag" -ForegroundColor Green
    } catch {
        $null = git rev-parse -q --verify MERGE_HEAD
        if ($LASTEXITCODE -eq 0) { git merge --abort }
        throw
    } finally {
        if ((git branch --show-current).Trim() -ne $start) { git switch $start }
    }
} finally {
    Pop-Location
}
