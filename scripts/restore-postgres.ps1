[CmdletBinding(SupportsShouldProcess = $true, ConfirmImpact = "High")]
param(
    [Parameter(Mandatory = $true)]
    [string]$Backup,
    [string]$EnvFile = ".env",
    [switch]$Force,
    [switch]$AllowUncheckedBackup
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

function Invoke-DockerStep {
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

function Resolve-RepositoryPath {
    param(
        [Parameter(Mandatory = $true)]
        [string]$Path,
        [Parameter(Mandatory = $true)]
        [string]$RepositoryRoot
    )

    if ([System.IO.Path]::IsPathRooted($Path)) {
        return $Path
    }
    return Join-Path $RepositoryRoot $Path
}

$RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path
$EnvFilePath = Resolve-RepositoryPath -Path $EnvFile -RepositoryRoot $RepoRoot
$BackupPath = Resolve-RepositoryPath -Path $Backup -RepositoryRoot $RepoRoot

if (-not (Test-Path -LiteralPath $EnvFilePath -PathType Leaf)) {
    throw "Environment file '$EnvFilePath' does not exist"
}
if (-not (Test-Path -LiteralPath $BackupPath -PathType Leaf)) {
    throw "Backup '$BackupPath' does not exist"
}
$BackupPath = (Resolve-Path -LiteralPath $BackupPath).Path

if (-not $Force) {
    throw "Restore replaces the current Agora database. Review the backup and rerun with -Force to continue."
}

$checksumPath = "$BackupPath.sha256"
if (Test-Path -LiteralPath $checksumPath -PathType Leaf) {
    $checksumContents = (Get-Content -LiteralPath $checksumPath -Raw).Trim()
    if ($checksumContents -notmatch '^(?<checksum>[A-Fa-f0-9]{64})(?:\s+\S+)?$') {
        throw "Backup checksum sidecar '$checksumPath' must contain one SHA-256 checksum"
    }
    $expectedChecksum = $Matches.checksum.ToLowerInvariant()
    $actualChecksum = (Get-FileHash -LiteralPath $BackupPath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($expectedChecksum -ne $actualChecksum) {
        throw "Backup checksum does not match '$checksumPath'"
    }
}
elseif (-not $AllowUncheckedBackup) {
    throw "Backup checksum sidecar '$checksumPath' is required. Use -AllowUncheckedBackup only for an independently verified emergency or legacy backup."
}
else {
    Write-Warning "No checksum sidecar found at '$checksumPath'. Continuing only because -AllowUncheckedBackup was explicitly supplied."
}

# Keep WhatIf and declined confirmations entirely local: no Docker command has run yet.
if (-not $PSCmdlet.ShouldProcess("database 'agora'", "replace all restored objects from '$BackupPath'")) {
    return
}

Push-Location -LiteralPath $RepoRoot
try {
    $composeArgs = @("--env-file", $EnvFilePath, "-f", "docker-compose.prod.yml")
    Invoke-DockerStep "Validating production Compose configuration" {
        docker compose @composeArgs config --quiet
    }

    $serverContainerId = (& docker compose @composeArgs ps -q server).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "Could not inspect the Agora server container"
    }
    if (-not [string]::IsNullOrWhiteSpace($serverContainerId)) {
        $serverRunning = (& docker inspect --format '{{.State.Running}}' $serverContainerId).Trim()
        if ($LASTEXITCODE -ne 0) {
            throw "Could not inspect Agora server state"
        }
        if ($serverRunning -eq "true") {
            throw "Stop the Agora server before restoring so it cannot write during the restore"
        }
    }

    $postgresContainerId = (& docker compose @composeArgs ps -q postgres).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "Could not identify the PostgreSQL container"
    }
    if ([string]::IsNullOrWhiteSpace($postgresContainerId)) {
        throw "PostgreSQL is not running; start PostgreSQL before restoring"
    }

    $containerRestorePath = "/tmp/agora-restore-$([Guid]::NewGuid().ToString("N")).dump"
    $backupCopied = $false
    try {
        Invoke-DockerStep "Copying backup into PostgreSQL container" {
            docker cp $BackupPath "$postgresContainerId`:$containerRestorePath"
        }
        $backupCopied = $true

        Invoke-DockerStep "Validating PostgreSQL backup" {
            docker compose @composeArgs exec -T postgres sh -c "pg_restore --list '$containerRestorePath' > /dev/null"
        }

        Invoke-DockerStep "Restoring PostgreSQL backup" {
            docker compose @composeArgs exec -T postgres pg_restore `
                --username=agora `
                --dbname=agora `
                --clean `
                --if-exists `
                --no-owner `
                --no-privileges `
                --single-transaction `
                --exit-on-error `
                $containerRestorePath
        }
        Invoke-DockerStep "Verifying restored PostgreSQL database" {
            docker compose @composeArgs exec -T postgres psql `
                --username=agora `
                --dbname=agora `
                --set=ON_ERROR_STOP=1 `
                "--command=SELECT 1"
        }

        ""
        "Restore completed. Start the Agora server and verify its /health endpoint before admitting traffic."
    }
    finally {
        if ($backupCopied) {
            & docker compose @composeArgs exec -T postgres rm -f $containerRestorePath
            if ($LASTEXITCODE -ne 0) {
                Write-Warning "Could not remove temporary PostgreSQL restore file '$containerRestorePath'"
            }
        }
    }
}
finally {
    Pop-Location
}
