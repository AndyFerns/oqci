<#
.SYNOPSIS
    Runs oqci-server and the visualization frontend together, and guarantees
    both process trees are stopped together on exit — including Ctrl+C.

.DESCRIPTION
    Starts `cargo run -p oqci-server` and `npm run dev` (in frontend/) as
    child processes, waits until either exits or you press Ctrl+C, then
    force-kills both process trees via `taskkill /T` — which is necessary
    because `npm run dev` spawns its own node/vite child process that a
    plain Stop-Process would leave running.

    See docs/visualization.md and server/README.md for what these two
    processes actually are.

.PARAMETER Root
    Directory oqci-server confines `watch_file` requests to. Default: repo root.

.PARAMETER Backend
    Backend every watched file is compiled against. Default: simulator-nisq.

.PARAMETER ServerPort
    Port oqci-server listens on. Default: 4173.

.PARAMETER FrontendPort
    Port the Vite dev server listens on. Default: 5173 (Vite's own default;
    passed through as --port so a stray existing instance can't silently
    shift this one to 5174).

.EXAMPLE
    scripts\dev-visualization.ps1
    scripts\dev-visualization.ps1 -Backend simulator -ServerPort 4200
#>

param(
    [string]$Root = ".",
    [string]$Backend = "simulator-nisq",
    [int]$ServerPort = 4173,
    [int]$FrontendPort = 5173
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$frontendDir = Join-Path $repoRoot "frontend"

$serverProc = $null
$frontendProc = $null

function Stop-ProcessTree {
    param(
        [System.Diagnostics.Process]$Proc,
        [string]$Name
    )
    if ($null -eq $Proc) { return }
    $Proc.Refresh()
    if ($Proc.HasExited) { return }
    Write-Host "Stopping $Name (PID $($Proc.Id)) and its child processes..."
    try {
        & taskkill /PID $Proc.Id /T /F 2>$null | Out-Null
    } catch {
        Write-Warning "Could not stop ${Name}: $_"
    }
}

try {
    if (-not (Test-Path (Join-Path $frontendDir "package.json"))) {
        throw "frontend/package.json not found under $frontendDir — run 'npm install' first, or check you're running this from the repo."
    }

    Write-Host "Starting oqci-server on port $ServerPort (backend: $Backend)..."
    $serverProc = Start-Process -FilePath "cargo" `
        -ArgumentList @("run", "-p", "oqci-server", "--", "--root", $Root, "--backend", $Backend, "--port", $ServerPort) `
        -WorkingDirectory $repoRoot -PassThru -NoNewWindow

    Write-Host "Starting frontend dev server on port $FrontendPort..."
    $frontendProc = Start-Process -FilePath "npm" `
        -ArgumentList @("run", "dev", "--", "--port", $FrontendPort) `
        -WorkingDirectory $frontendDir -PassThru -NoNewWindow

    Write-Host ""
    Write-Host "  server:   http://localhost:$ServerPort"
    Write-Host "  frontend: http://localhost:$FrontendPort"
    Write-Host ""
    Write-Host "Press Ctrl+C to stop both."
    Write-Host ""

    # Ctrl+C interrupts Start-Sleep and unwinds through this try/finally,
    # which is what guarantees the joint teardown below actually runs.
    while (-not $serverProc.HasExited -and -not $frontendProc.HasExited) {
        Start-Sleep -Seconds 1
    }

    if ($serverProc.HasExited) {
        Write-Warning "oqci-server exited on its own (exit code $($serverProc.ExitCode))."
    }
    if ($frontendProc.HasExited) {
        Write-Warning "frontend dev server exited on its own (exit code $($frontendProc.ExitCode))."
    }
}
finally {
    Stop-ProcessTree -Proc $serverProc -Name "oqci-server"
    Stop-ProcessTree -Proc $frontendProc -Name "frontend dev server"
    Write-Host "Both stopped."
}
