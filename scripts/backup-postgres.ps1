[CmdletBinding()]
param(
    [string]$EnvFile = ".env",
    [string]$OutputDirectory = (Join-Path $HOME "AgoraBackups")
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
$OutputDirectoryPath = Resolve-RepositoryPath -Path $OutputDirectory -RepositoryRoot $RepoRoot

if (-not (Test-Path -LiteralPath $EnvFilePath -PathType Leaf)) {
    throw "Environment file '$EnvFilePath' does not exist"
}

$outputParent = Split-Path -Parent $OutputDirectoryPath
if (-not (Test-Path -LiteralPath $outputParent -PathType Container)) {
    throw "Backup output parent '$outputParent' does not exist"
}
if (-not (Test-Path -LiteralPath $OutputDirectoryPath -PathType Container)) {
    New-Item -ItemType Directory -Path $OutputDirectoryPath | Out-Null
}

Push-Location -LiteralPath $RepoRoot
try {
    $composeArgs = @("--env-file", $EnvFilePath, "-f", "docker-compose.prod.yml")
    Invoke-DockerStep "Validating production Compose configuration" {
        docker compose @composeArgs config --quiet
    }

    $postgresContainerId = (& docker compose @composeArgs ps -q postgres).Trim()
    if ($LASTEXITCODE -ne 0) {
        throw "Could not identify the PostgreSQL container"
    }
    if ([string]::IsNullOrWhiteSpace($postgresContainerId)) {
        throw "PostgreSQL is not running; start the production stack before creating a backup"
    }

    $timestamp = (Get-Date).ToUniversalTime().ToString("yyyyMMddTHHmmssZ")
    $backupName = "agora-postgres-$timestamp.dump"
    $backupPath = Join-Path $OutputDirectoryPath $backupName
    $partialBackupPath = "$backupPath.partial"
    $checksumPath = "$backupPath.sha256"
    $containerBackupPath = "/tmp/$backupName"
    $containerBackupCreated = $false

    if (Test-Path -LiteralPath $backupPath) {
        throw "Backup '$backupPath' already exists; refusing to overwrite it"
    }
    if (Test-Path -LiteralPath $partialBackupPath) {
        throw "Partial backup '$partialBackupPath' already exists; inspect or remove it before retrying"
    }

    try {
        $containerBackupCreated = $true
        Invoke-DockerStep "Creating PostgreSQL custom-format dump" {
            docker compose @composeArgs exec -T postgres pg_dump `
                --username=agora `
                --dbname=agora `
                --format=custom `
                --compress=9 `
                --no-owner `
                --no-privileges `
                "--file=$containerBackupPath"
        }

        Invoke-DockerStep "Copying backup from PostgreSQL container" {
            docker cp "$postgresContainerId`:$containerBackupPath" $partialBackupPath
        }
        Move-Item -LiteralPath $partialBackupPath -Destination $backupPath

        $checksum = (Get-FileHash -LiteralPath $backupPath -Algorithm SHA256).Hash.ToLowerInvariant()
        "$checksum  $backupName" | Set-Content -LiteralPath $checksumPath -Encoding ascii -NoNewline

        ""
        "Backup created:"
        "  $backupPath"
        "  $checksumPath"
        "Copy both files to encrypted off-host storage and test restores regularly."
    }
    finally {
        if ($containerBackupCreated) {
            & docker compose @composeArgs exec -T postgres rm -f $containerBackupPath
            if ($LASTEXITCODE -ne 0) {
                Write-Warning "Could not remove temporary PostgreSQL backup '$containerBackupPath'"
            }
        }
        if (Test-Path -LiteralPath $partialBackupPath) {
            Remove-Item -LiteralPath $partialBackupPath -Force
        }
    }
}
finally {
    Pop-Location
}
