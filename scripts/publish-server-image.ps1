[CmdletBinding()]
param(
    [string]$Image = "ghcr.io/lero-aom/agora-server",
    [string]$Version,
    [switch]$SkipLatest
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

$RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path

Push-Location -LiteralPath $RepoRoot
try {
    if ([string]::IsNullOrWhiteSpace($Version)) {
        $metadataJson = & cargo metadata --no-deps --format-version 1
        if ($LASTEXITCODE -ne 0) {
            throw "cargo metadata failed with exit code $LASTEXITCODE"
        }

        $metadata = $metadataJson | ConvertFrom-Json
        $serverPackage = $metadata.packages | Where-Object { $_.name -eq "agora-server" } | Select-Object -First 1
        if ($null -eq $serverPackage) {
            throw "Could not find agora-server package metadata"
        }
        $Version = $serverPackage.version
    }

    $tags = @("$Image`:$Version")
    if (-not $SkipLatest) {
        $tags += "$Image`:latest"
    }

    $tagArgs = @()
    foreach ($tag in $tags) {
        $tagArgs += @("-t", $tag)
    }

    Invoke-Step "Publishing linux/amd64 server image" {
        docker buildx build `
            --platform linux/amd64 `
            -f crates/agora-server/Dockerfile `
            @tagArgs `
            --push `
            .
    }

    ""
    "Published image tags:"
    foreach ($tag in $tags) {
        "  $tag"
    }
}
finally {
    Pop-Location
}
