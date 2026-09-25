<#
.SYNOPSIS
    Force-stops oqci-server and the visualization frontend dev server, by
    port and by process name — a safety net for when they weren't stopped
    through dev-visualization.ps1 (terminal closed out from under them, a
    crash, a manually-started instance, etc.), so nothing keeps a port
    bound in the background.

.EXAMPLE
    scripts\stop-visualization.ps1
    scripts\stop-visualization.ps1 -ServerPort 4200
#>

param(
    [int]$ServerPort = 4173,
    [int]$FrontendPort = 5173
)

function Stop-ByPort {
    param([int]$Port)
    $conns = Get-NetTCPConnection -LocalPort $Port -State Listen -ErrorAction SilentlyContinue
    if (-not $conns) {
        Write-Host "Nothing listening on port $Port."
        return
    }
    foreach ($c in $conns) {
        $procId = $c.OwningProcess
        $proc = Get-Process -Id $procId -ErrorAction SilentlyContinue
        $label = if ($proc) { "$($proc.ProcessName) (PID $procId)" } else { "PID $procId" }
        Write-Host "Killing $label listening on port $Port"
        & taskkill /PID $procId /T /F 2>$null | Out-Null
    }
}

Stop-ByPort -Port $ServerPort
Stop-ByPort -Port $FrontendPort

# Backstop in case either process is listening on a non-default port: catch
# oqci-server by name directly. (The frontend dev server has no stable
# process name of its own — it's just "node" — so it relies entirely on the
# port check above.)
$byName = Get-Process -Name "oqci-server" -ErrorAction SilentlyContinue
foreach ($p in $byName) {
    Write-Host "Killing oqci-server (PID $($p.Id)) by name"
    & taskkill /PID $p.Id /T /F 2>$null | Out-Null
}

Write-Host "Done."
