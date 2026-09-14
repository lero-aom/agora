[CmdletBinding()]
param(
    [string]$ServerUrl = "https://chat.aomagora.com"
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Invoke-CargoStep {
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
$PreviousDefaultServerUrl = [Environment]::GetEnvironmentVariable("AGORA_DEFAULT_SERVER_URL", "Process")

Push-Location -LiteralPath $RepoRoot
try {
    if (-not ($ServerUrl -match '^https?://[^\s/]+')) {
        throw "ServerUrl must be an absolute http(s) URL"
    }

    Invoke-CargoStep "Checking formatting" { cargo fmt --all -- --check }
    Invoke-CargoStep "Running clippy" { cargo clippy --locked --workspace --all-targets --all-features -- -D warnings }
    Invoke-CargoStep "Running unit tests" { cargo test --locked --workspace }
    if ([string]::IsNullOrWhiteSpace($env:DATABASE_URL)) {
        throw "DATABASE_URL is required to run PostgreSQL release checks"
    }
    Invoke-CargoStep "Running PostgreSQL invariant tests" {
        cargo test --locked -p agora-server --features postgres-tests --test postgres_invariants
    }

    Invoke-CargoStep "Building release server" { cargo build --locked --release -p agora-server }
    [Environment]::SetEnvironmentVariable("AGORA_DEFAULT_SERVER_URL", $ServerUrl.TrimEnd('/'), "Process")
    Invoke-CargoStep "Building release client for $($ServerUrl.TrimEnd('/'))" { cargo build --locked --release -p agora-client }

    $metadataJson = & cargo metadata --no-deps --format-version 1
    if ($LASTEXITCODE -ne 0) {
        throw "cargo metadata failed with exit code $LASTEXITCODE"
    }

    $metadata = $metadataJson | ConvertFrom-Json
    $clientPackage = $metadata.packages | Where-Object { $_.name -eq "agora-client" } | Select-Object -First 1
    if ($null -eq $clientPackage) {
        throw "Could not find agora-client package metadata"
    }

    $version = $clientPackage.version
    $distRoot = Join-Path $RepoRoot "dist"
    $releaseName = "agora-$version-windows-x64"
    $stagingDir = Join-Path $distRoot $releaseName
    $archivePath = Join-Path $distRoot "$releaseName.zip"

    if (Test-Path -LiteralPath $stagingDir) {
        Remove-Item -LiteralPath $stagingDir -Recurse -Force
    }
    if (Test-Path -LiteralPath $archivePath) {
        Remove-Item -LiteralPath $archivePath -Force
    }

    New-Item -ItemType Directory -Path $stagingDir -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $RepoRoot "target\release\agora-client.exe") -Destination (Join-Path $stagingDir "agora-client.exe")
    Copy-Item -LiteralPath (Join-Path $RepoRoot "target\release\agora-server.exe") -Destination (Join-Path $stagingDir "agora-server.exe")
    Copy-Item -LiteralPath (Join-Path $RepoRoot "README.md") -Destination (Join-Path $stagingDir "README.md")

    Compress-Archive -Path (Join-Path $stagingDir "*") -DestinationPath $archivePath -Force
    $checksumPath = "$archivePath.sha256"
    $checksum = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    "$checksum  $(Split-Path -Leaf $archivePath)" | Set-Content -LiteralPath $checksumPath -NoNewline

    ""
    "Release artifacts:"
    "  $stagingDir"
    "  $archivePath"
    "  $checksumPath"
}
finally {
    [Environment]::SetEnvironmentVariable("AGORA_DEFAULT_SERVER_URL", $PreviousDefaultServerUrl, "Process")
    Pop-Location
}
