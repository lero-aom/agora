[CmdletBinding()]
param(
    [string]$ServerUrl = "https://chat.aomagora.com",
    [string]$UpdateBaseUrl = "https://github.com/lero-aom/agora/releases/latest/download/",
    [switch]$SkipPostgresTests
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest
$UpdaterTarget = "x86_64-pc-windows-msvc"

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
        throw "Release packaging must run from a Git worktree"
    }

    $changes = @(& git status --porcelain=v1 --untracked-files=all)
    if ($LASTEXITCODE -ne 0) {
        throw "git status failed with exit code $LASTEXITCODE"
    }
    if ($changes.Count -gt 0) {
        throw "Refusing to package a dirty worktree. Commit, stash, or remove these changes first:`n$($changes -join "`n")"
    }
}

function Assert-ZipEntries {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [Parameter(Mandatory = $true)]
        [string[]]$RequiredEntries
    )

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [System.IO.Compression.ZipFile]::OpenRead($Path)
    try {
        $entries = @($archive.Entries | ForEach-Object { $_.FullName })
        foreach ($requiredEntry in $RequiredEntries) {
            if ($entries -notcontains $requiredEntry) {
                throw "Archive '$Path' is missing required entry '$requiredEntry'"
            }
        }
    }
    finally {
        $archive.Dispose()
    }
}

function Assert-ChecksumSidecar {
    param(
        [Parameter(Mandatory = $true)]
        [string]$FilePath,
        [Parameter(Mandatory = $true)]
        [string]$SidecarPath
    )

    $sidecar = [IO.File]::ReadAllText($SidecarPath)
    if ($sidecar -notmatch '^(?<hash>[0-9a-f]{64})  (?<name>.+)$') {
        throw "Checksum sidecar '$SidecarPath' is malformed"
    }
    $expectedName = Split-Path -Leaf $FilePath
    if ($Matches.name -ne $expectedName) {
        throw "Checksum sidecar '$SidecarPath' names '$($Matches.name)' instead of '$expectedName'"
    }
    $actualHash = (Get-FileHash -LiteralPath $FilePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($Matches.hash -ne $actualHash) {
        throw "Checksum sidecar '$SidecarPath' does not match '$FilePath'"
    }
}

function Assert-ReleasePackage {
    param(
        [Parameter(Mandatory = $true)]
        [string]$ArchivePath,
        [Parameter(Mandatory = $true)]
        [string]$ArchiveChecksumPath,
        [Parameter(Mandatory = $true)]
        [string]$ClientArtifactPath,
        [Parameter(Mandatory = $true)]
        [string]$ClientChecksumPath,
        [Parameter(Mandatory = $true)]
        [string]$SourceArchivePath,
        [Parameter(Mandatory = $true)]
        [string]$SourceChecksumPath,
        [Parameter(Mandatory = $true)]
        [string]$ManifestPath,
        [Parameter(Mandatory = $true)]
        [string]$ManifestSignaturePath,
        [Parameter(Mandatory = $true)]
        [string]$SignToolPath,
        [Parameter(Mandatory = $true)]
        [string]$UpdatePublicKey
    )

    Assert-ChecksumSidecar -FilePath $ArchivePath -SidecarPath $ArchiveChecksumPath
    Assert-ChecksumSidecar -FilePath $ClientArtifactPath -SidecarPath $ClientChecksumPath
    Assert-ChecksumSidecar -FilePath $SourceArchivePath -SidecarPath $SourceChecksumPath

    $extractDirectory = Join-Path ([IO.Path]::GetTempPath()) "agora-release-verify-$([Guid]::NewGuid().ToString('N'))"
    New-Item -ItemType Directory -Path $extractDirectory -Force | Out-Null
    try {
        [IO.Compression.ZipFile]::ExtractToDirectory($ArchivePath, $extractDirectory)
        $bundledClientPath = Join-Path $extractDirectory "agora-client.exe"
        $bundledClientChecksumPath = "$bundledClientPath.sha256"
        $bundledSourcePath = Join-Path $extractDirectory (Split-Path -Leaf $SourceArchivePath)
        $bundledSourceChecksumPath = "$bundledSourcePath.sha256"
        $bundledManifestPath = Join-Path $extractDirectory (Split-Path -Leaf $ManifestPath)
        $bundledManifestSignaturePath = Join-Path $extractDirectory (Split-Path -Leaf $ManifestSignaturePath)

        Assert-ChecksumSidecar -FilePath $bundledClientPath -SidecarPath $bundledClientChecksumPath
        Assert-ChecksumSidecar -FilePath $bundledSourcePath -SidecarPath $bundledSourceChecksumPath
        if ((Get-FileHash -LiteralPath $ManifestPath -Algorithm SHA256).Hash -ne (Get-FileHash -LiteralPath $bundledManifestPath -Algorithm SHA256).Hash) {
            throw "The packaged update manifest differs from the publishable manifest"
        }
        if ((Get-FileHash -LiteralPath $ManifestSignaturePath -Algorithm SHA256).Hash -ne (Get-FileHash -LiteralPath $bundledManifestSignaturePath -Algorithm SHA256).Hash) {
            throw "The packaged update signature differs from the publishable signature"
        }
        & $SignToolPath verify --manifest $bundledManifestPath --signature $bundledManifestSignaturePath --public-key $UpdatePublicKey
        if ($LASTEXITCODE -ne 0) {
            throw "The packaged update manifest signature is invalid"
        }

        $manifest = Get-Content -LiteralPath $bundledManifestPath -Raw | ConvertFrom-Json
        $clientHash = (Get-FileHash -LiteralPath $ClientArtifactPath -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($manifest.filename -ne (Split-Path -Leaf $ClientArtifactPath) -or
            [int64]$manifest.size -ne (Get-Item -LiteralPath $ClientArtifactPath).Length -or
            $manifest.sha256 -ne $clientHash) {
            throw "The packaged update manifest does not describe the publishable client"
        }
    }
    finally {
        if (Test-Path -LiteralPath $extractDirectory) {
            Remove-Item -LiteralPath $extractDirectory -Recurse -Force
        }
    }
}

function Assert-ReleaseHttpsUrl {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Value,
        [Parameter(Mandatory = $true)]
        [string]$Name,
        [switch]$RequireTrailingSlash
    )

    $uri = $null
    if (-not [Uri]::TryCreate($Value, [UriKind]::Absolute, [ref]$uri) -or
        $uri.Scheme -ne "https" -or
        [string]::IsNullOrWhiteSpace($uri.Host) -or
        -not [string]::IsNullOrEmpty($uri.UserInfo) -or
        -not [string]::IsNullOrEmpty($uri.Query) -or
        -not [string]::IsNullOrEmpty($uri.Fragment)) {
        throw "$Name must be an absolute HTTPS URL without credentials, query, or fragment"
    }
    if ($RequireTrailingSlash -and -not $Value.EndsWith("/")) {
        throw "$Name must end with '/' so update assets remain pinned below that URL"
    }
}

function ConvertTo-ClientServerOrigin {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Value
    )

    $value = $Value.Trim()
    if ([string]::IsNullOrWhiteSpace($value)) {
        throw "ServerUrl must be an absolute HTTP(S) origin"
    }

    $uri = $null
    if (-not [Uri]::TryCreate($value, [UriKind]::Absolute, [ref]$uri) -or
        ($uri.Scheme -ne "http" -and $uri.Scheme -ne "https") -or
        [string]::IsNullOrWhiteSpace($uri.Host)) {
        throw "ServerUrl must be an absolute HTTP(S) origin"
    }

    # Match the client-side raw-authority check so an empty user name is rejected too.
    $authority = [regex]::Match($value, '^[^:]+://([^/?#]*)').Groups[1].Value
    if ($authority.Contains("@") -or -not [string]::IsNullOrEmpty($uri.UserInfo)) {
        throw "ServerUrl must not include credentials"
    }
    if ($uri.AbsolutePath -ne "/" -or $value.Contains("?") -or $value.Contains("#")) {
        throw "ServerUrl must be an origin without a path, query, or fragment"
    }

    $serverHost = $uri.Host
    $ipHost = $serverHost
    if ($ipHost.StartsWith("[") -and $ipHost.EndsWith("]")) {
        $ipHost = $ipHost.Substring(1, $ipHost.Length - 2)
    }
    $ipAddress = $null
    $isLoopback = $serverHost.Equals("localhost", [System.StringComparison]::OrdinalIgnoreCase)
    if ([System.Net.IPAddress]::TryParse($ipHost, [ref]$ipAddress) -and [System.Net.IPAddress]::IsLoopback($ipAddress)) {
        $isLoopback = $true
    }
    if ($uri.Scheme -eq "http" -and -not $isLoopback) {
        throw "ServerUrl must use HTTPS unless it targets a loopback host"
    }

    $canonicalHost = $uri.IdnHost.ToLowerInvariant()
    if ($uri.HostNameType -eq [System.UriHostNameType]::IPv6) {
        $canonicalHost = "[$([System.Net.IPAddress]::Parse($ipHost).ToString())]"
    }
    $port = if ($uri.IsDefaultPort) { "" } else { ":$($uri.Port)" }
    return "$($uri.Scheme.ToLowerInvariant())://$canonicalHost$port"
}

$RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
$PreviousDefaultServerUrl = [Environment]::GetEnvironmentVariable("AGORA_DEFAULT_SERVER_URL", "Process")
$PreviousSourceDateEpoch = [Environment]::GetEnvironmentVariable("SOURCE_DATE_EPOCH", "Process")
$PreviousUpdateBaseUrl = [Environment]::GetEnvironmentVariable("AGORA_UPDATE_BASE_URL", "Process")
$PreviousUpdatePublicKey = [Environment]::GetEnvironmentVariable("AGORA_UPDATE_PUBLIC_KEY_B64", "Process")
$PreviousUpdateSigningKey = [Environment]::GetEnvironmentVariable("AGORA_UPDATE_SIGNING_KEY_B64", "Process")

Push-Location -LiteralPath $RepoRoot
try {
    $ServerUrl = ConvertTo-ClientServerOrigin -Value $ServerUrl
    $UpdateBaseUrl = $UpdateBaseUrl.Trim()
    Assert-ReleaseHttpsUrl -Value $UpdateBaseUrl -Name "UpdateBaseUrl" -RequireTrailingSlash
    if ([string]::IsNullOrWhiteSpace($PreviousUpdateSigningKey)) {
        throw "AGORA_UPDATE_SIGNING_KEY_B64 is required to package a signed client release"
    }
    $updateSigningKey = $PreviousUpdateSigningKey
    [Environment]::SetEnvironmentVariable("AGORA_UPDATE_SIGNING_KEY_B64", $null, "Process")

    Assert-CleanGitWorktree
    $commit = (& git rev-parse --verify HEAD).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "git rev-parse failed with exit code $LASTEXITCODE"
    }
    $sourceDateEpoch = (& git show -s --format=%ct HEAD).Trim()
    if ($LASTEXITCODE -ne 0 -or -not ($sourceDateEpoch -match '^\d+$')) {
        throw "Could not determine SOURCE_DATE_EPOCH from HEAD"
    }
    [Environment]::SetEnvironmentVariable("SOURCE_DATE_EPOCH", $sourceDateEpoch, "Process")

    Invoke-Step "Checking formatting" { cargo fmt --all -- --check }
    Invoke-Step "Running clippy" { cargo clippy --locked --workspace --all-targets --all-features -- -D warnings }
    Invoke-Step "Running unit tests" { cargo test --locked --workspace }
    if ($SkipPostgresTests) {
        "Skipping PostgreSQL feature and invariant tests by request"
    }
    else {
        if ([string]::IsNullOrWhiteSpace($env:DATABASE_URL)) {
            throw "DATABASE_URL is required to run PostgreSQL release checks"
        }
        Invoke-Step "Running PostgreSQL feature and invariant tests" {
            cargo test --locked -p agora-server --features postgres-tests -- --test-threads=1
        }
    }

    Invoke-Step "Building release server" { cargo build --locked --release -p agora-server }
    Invoke-Step "Building release update signer" {
        cargo build --locked --release -p agora-update-sign --features release-tool
    }
    $signToolPath = Join-Path $RepoRoot "target\release\agora-update-sign.exe"
    if (-not (Test-Path -LiteralPath $signToolPath)) {
        throw "Could not find the release update signer"
    }
    [Environment]::SetEnvironmentVariable("AGORA_UPDATE_SIGNING_KEY_B64", $updateSigningKey, "Process")
    $publicKeyLines = @(& $signToolPath public-key)
    $publicKeyExitCode = $LASTEXITCODE
    [Environment]::SetEnvironmentVariable("AGORA_UPDATE_SIGNING_KEY_B64", $null, "Process")
    if ($publicKeyExitCode -ne 0) {
        throw "Could not derive the update public key"
    }
    $updatePublicKey = ($publicKeyLines -join "").Trim()
    if ($updatePublicKey -notmatch '^[A-Za-z0-9+/]{43}=$') {
        throw "The update signer returned an invalid public key"
    }

    [Environment]::SetEnvironmentVariable("AGORA_DEFAULT_SERVER_URL", $ServerUrl, "Process")
    [Environment]::SetEnvironmentVariable("AGORA_UPDATE_BASE_URL", $UpdateBaseUrl, "Process")
    [Environment]::SetEnvironmentVariable("AGORA_UPDATE_PUBLIC_KEY_B64", $updatePublicKey, "Process")
    Invoke-Step "Building release client for $ServerUrl" {
        cargo build --locked --release --target $UpdaterTarget -p agora-client
    }

    # Ensure generated artifacts still correspond to the committed source revision.
    Assert-CleanGitWorktree

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
    if ($version -notmatch '^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$') {
        throw "Client version must be a stable semantic version for the update manifest"
    }
    $distRoot = Join-Path $RepoRoot "dist"
    $releaseName = "agora-$version-windows-x64"
    $stagingDir = Join-Path $distRoot $releaseName
    $archivePath = Join-Path $distRoot "$releaseName.zip"
    $sourceArchiveName = "$releaseName-source.zip"
    $sourceArchivePath = Join-Path $distRoot $sourceArchiveName
    $clientArtifactName = "agora-client-$version-windows-x64.exe"
    $bundledClientArtifactName = "agora-client.exe"
    $clientArtifactPath = Join-Path $distRoot $clientArtifactName
    $clientChecksumPath = "$clientArtifactPath.sha256"
    $manifestName = "agora-update-manifest.json"
    $manifestPath = Join-Path $distRoot $manifestName
    $manifestSignatureName = "$manifestName.sig"
    $manifestSignaturePath = Join-Path $distRoot $manifestSignatureName
    $clientOutputPath = Join-Path $RepoRoot "target\$UpdaterTarget\release\agora-client.exe"
    if (-not (Test-Path -LiteralPath $clientOutputPath)) {
        throw "Could not find the targeted release client executable"
    }

    New-Item -ItemType Directory -Path $distRoot -Force | Out-Null
    if (Test-Path -LiteralPath $stagingDir) {
        Remove-Item -LiteralPath $stagingDir -Recurse -Force
    }
    if (Test-Path -LiteralPath $archivePath) {
        Remove-Item -LiteralPath $archivePath -Force
    }
    if (Test-Path -LiteralPath $sourceArchivePath) {
        Remove-Item -LiteralPath $sourceArchivePath -Force
    }
    foreach ($artifactPath in @($clientArtifactPath, $clientChecksumPath, $manifestPath, $manifestSignaturePath)) {
        if (Test-Path -LiteralPath $artifactPath) {
            Remove-Item -LiteralPath $artifactPath -Force
        }
    }

    New-Item -ItemType Directory -Path $stagingDir -Force | Out-Null
    Copy-Item -LiteralPath $clientOutputPath -Destination $clientArtifactPath
    $clientSize = (Get-Item -LiteralPath $clientArtifactPath).Length
    if ($clientSize -le 0) {
        throw "The release client executable is empty"
    }
    $clientChecksum = (Get-FileHash -LiteralPath $clientArtifactPath -Algorithm SHA256).Hash.ToLowerInvariant()
    "$clientChecksum  $clientArtifactName" | Set-Content -LiteralPath $clientChecksumPath -Encoding ascii -NoNewline

    $manifest = [ordered]@{
        schema_version = 1
        version = $version
        target = $UpdaterTarget
        filename = $clientArtifactName
        size = $clientSize
        sha256 = $clientChecksum
    } | ConvertTo-Json -Compress
    [IO.File]::WriteAllText(
        $manifestPath,
        $manifest,
        [Text.UTF8Encoding]::new($false)
    )
    [Environment]::SetEnvironmentVariable("AGORA_UPDATE_SIGNING_KEY_B64", $updateSigningKey, "Process")
    try {
        Invoke-Step "Signing raw update manifest" {
            & $signToolPath sign --manifest $manifestPath --output $manifestSignaturePath
        }
    }
    finally {
        [Environment]::SetEnvironmentVariable("AGORA_UPDATE_SIGNING_KEY_B64", $null, "Process")
    }
    if (-not (Test-Path -LiteralPath $manifestSignaturePath)) {
        throw "The update signer did not write a detached manifest signature"
    }

    Invoke-Step "Creating complete corresponding source archive" {
        git archive --format=zip "--prefix=$releaseName-source/" "--output=$sourceArchivePath" HEAD
    }
    Assert-ZipEntries -Path $sourceArchivePath -RequiredEntries @(
        "$releaseName-source/Cargo.toml",
        "$releaseName-source/Cargo.lock",
        "$releaseName-source/LICENSE",
        "$releaseName-source/crates/agora-server/Dockerfile"
    )

    $sourceChecksumPath = "$sourceArchivePath.sha256"
    $sourceChecksum = (Get-FileHash -LiteralPath $sourceArchivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    "$sourceChecksum  $sourceArchiveName" | Set-Content -LiteralPath $sourceChecksumPath -Encoding ascii -NoNewline

    Copy-Item -LiteralPath $clientArtifactPath -Destination (Join-Path $stagingDir $bundledClientArtifactName)
    "$clientChecksum  $bundledClientArtifactName" | Set-Content -LiteralPath (Join-Path $stagingDir "$bundledClientArtifactName.sha256") -Encoding ascii -NoNewline
    Copy-Item -LiteralPath (Join-Path $RepoRoot "target\release\agora-server.exe") -Destination (Join-Path $stagingDir "agora-server.exe")
    Copy-Item -LiteralPath (Join-Path $RepoRoot "README.md") -Destination (Join-Path $stagingDir "README.md")
    Copy-Item -LiteralPath (Join-Path $RepoRoot "LICENSE") -Destination (Join-Path $stagingDir "LICENSE")
    Copy-Item -LiteralPath $manifestPath -Destination (Join-Path $stagingDir $manifestName)
    Copy-Item -LiteralPath $manifestSignaturePath -Destination (Join-Path $stagingDir $manifestSignatureName)
    Copy-Item -LiteralPath $sourceArchivePath -Destination (Join-Path $stagingDir $sourceArchiveName)
    Copy-Item -LiteralPath $sourceChecksumPath -Destination (Join-Path $stagingDir "$sourceArchiveName.sha256")

    @"
# Source and License Notice

Agora is licensed under the GNU Affero General Public License version 3 only (AGPL-3.0-only). See LICENSE for the complete license text.

The complete corresponding source for this binary release is bundled as $sourceArchiveName. It was created directly from the committed source listed below and includes Cargo.lock.

The directly publishable updater package is $clientArtifactName. Its SHA-256 is published in $clientArtifactName.sha256 and its signed raw metadata is $manifestName with detached signature $manifestSignatureName. The bundled $bundledClientArtifactName has a matching $bundledClientArtifactName.sha256 sidecar.

- Version: $version
- Git commit: $commit
- Source repository: https://github.com/lero-aom/agora/tree/$commit

Network operators must make the corresponding source of their running modified version available to users as required by AGPLv3 section 13.
"@ | Set-Content -LiteralPath (Join-Path $stagingDir "SOURCE.md") -Encoding ascii -NoNewline

    Compress-Archive -Path (Join-Path $stagingDir "*") -DestinationPath $archivePath -Force
    Assert-ZipEntries -Path $archivePath -RequiredEntries @(
        $bundledClientArtifactName,
        "$bundledClientArtifactName.sha256",
        "agora-server.exe",
        "README.md",
        "LICENSE",
        "SOURCE.md",
        $manifestName,
        $manifestSignatureName,
        $sourceArchiveName,
        "$sourceArchiveName.sha256"
    )

    $checksumPath = "$archivePath.sha256"
    $checksum = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    "$checksum  $(Split-Path -Leaf $archivePath)" | Set-Content -LiteralPath $checksumPath -Encoding ascii -NoNewline
    Invoke-Step "Verifying packaged release bytes" {
        Assert-ReleasePackage `
            -ArchivePath $archivePath `
            -ArchiveChecksumPath $checksumPath `
            -ClientArtifactPath $clientArtifactPath `
            -ClientChecksumPath $clientChecksumPath `
            -SourceArchivePath $sourceArchivePath `
            -SourceChecksumPath $sourceChecksumPath `
            -ManifestPath $manifestPath `
            -ManifestSignaturePath $manifestSignaturePath `
            -SignToolPath $signToolPath `
            -UpdatePublicKey $updatePublicKey
    }

    ""
    "Release artifacts:"
    "  $stagingDir"
    "  $clientArtifactPath"
    "  $clientChecksumPath"
    "  $manifestPath"
    "  $manifestSignaturePath"
    "  $archivePath"
    "  $checksumPath"
    "  $sourceArchivePath"
    "  $sourceChecksumPath"
}
finally {
    [Environment]::SetEnvironmentVariable("AGORA_DEFAULT_SERVER_URL", $PreviousDefaultServerUrl, "Process")
    [Environment]::SetEnvironmentVariable("SOURCE_DATE_EPOCH", $PreviousSourceDateEpoch, "Process")
    [Environment]::SetEnvironmentVariable("AGORA_UPDATE_BASE_URL", $PreviousUpdateBaseUrl, "Process")
    [Environment]::SetEnvironmentVariable("AGORA_UPDATE_PUBLIC_KEY_B64", $PreviousUpdatePublicKey, "Process")
    [Environment]::SetEnvironmentVariable("AGORA_UPDATE_SIGNING_KEY_B64", $PreviousUpdateSigningKey, "Process")
    Pop-Location
}
