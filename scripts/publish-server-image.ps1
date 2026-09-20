[CmdletBinding()]
param(
    [string]$Image = "ghcr.io/lero-aom/agora-server",
    [string]$Version,
    [switch]$PublishLatest,
    [string]$RustImage,
    [string]$RuntimeImage
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Invoke-Step {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [Parameter(Mandatory = $true)]
        [scriptblock]$Command
    )

    ""
    "==> $Name"
    & $Command
    if ($LASTEXITCODE -ne 0) {
        throw "$Name failed with exit code $LASTEXITCODE"
    }
}

function Assert-CleanGitWorktree {
    $insideWorktree = (& git rev-parse --is-inside-work-tree).Trim()
    if ($LASTEXITCODE -ne 0 -or $insideWorktree -ne "true") {
        throw "Image publishing must run from a Git worktree"
    }

    $changes = @(& git status --porcelain=v1 --untracked-files=all)
    if ($LASTEXITCODE -ne 0) {
        throw "git status failed with exit code $LASTEXITCODE"
    }
    if ($changes.Count -gt 0) {
        throw "Refusing to publish from a dirty worktree. Commit, stash, or remove these changes first:`n$($changes -join "`n")"
    }
}

$RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path

Push-Location -LiteralPath $RepoRoot
try {
    Assert-CleanGitWorktree
    if ([string]::IsNullOrWhiteSpace($Image) -or $Image -match '\s') {
        throw "Image must be a non-empty image repository without whitespace"
    }

    $metadataJson = & cargo metadata --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata failed with exit code $LASTEXITCODE"
    }

    $metadata = $metadataJson | ConvertFrom-Json
    $serverPackage = $metadata.packages | Where-Object { $_.name -eq "agora-server" } | Select-Object -First 1
    if ($null -eq $serverPackage) {
        throw "Could not find agora-server package metadata"
    }
    if ([string]::IsNullOrWhiteSpace($Version)) {
        $Version = $serverPackage.version
    }
    elseif ($Version -match '\s') {
        throw "Version must not contain whitespace"
    }
    elseif ($Version -ne $serverPackage.version) {
        throw "Version '$Version' does not match agora-server package version '$($serverPackage.version)'"
    }

    $revision = (& git rev-parse --verify HEAD).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "git rev-parse failed with exit code $LASTEXITCODE"
    }

    $tags = @("$Image`:$Version")
    if ($PublishLatest) {
        $tags += "$Image`:latest"
    }

    $tagArgs = @()
    foreach ($tag in $tags) {
        $tagArgs += @("-t", $tag)
    }

    $buildArgArgs = @(
        "--build-arg", "VERSION=$Version",
        "--build-arg", "REVISION=$revision"
    )
    if (-not [string]::IsNullOrWhiteSpace($RustImage)) {
        $buildArgArgs += @("--build-arg", "RUST_IMAGE=$RustImage")
    }
    if (-not [string]::IsNullOrWhiteSpace($RuntimeImage)) {
        $buildArgArgs += @("--build-arg", "RUNTIME_IMAGE=$RuntimeImage")
    }

    $metadataPath = Join-Path ([System.IO.Path]::GetTempPath()) "agora-server-image-$Version.json"

    try {
        Assert-CleanGitWorktree
        Invoke-Step "Publishing linux/amd64 server image" {
            docker buildx build `
                --platform linux/amd64 `
                -f crates/agora-server/Dockerfile `
                @tagArgs `
                @buildArgArgs `
                --pull `
                --provenance=mode=max `
                --sbom=true `
                --metadata-file $metadataPath `
                --push `
                .
        }

        $metadata = Get-Content -LiteralPath $metadataPath -Raw | ConvertFrom-Json
        $digest = $metadata.'containerimage.digest'
        if ([string]::IsNullOrWhiteSpace($digest)) {
            throw "Buildx did not return a container image digest"
        }

        ""
        "Published image tags:"
        foreach ($tag in $tags) {
            "  $tag"
        }
        "Immutable deployment reference:"
        "  $Image@$digest"
    }
    finally {
        if (Test-Path -LiteralPath $metadataPath) {
            Remove-Item -LiteralPath $metadataPath -Force
        }
    }
}
finally {
    Pop-Location
}
