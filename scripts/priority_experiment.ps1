# Measures whether worker thread priority protects a pinned worker from a noisy neighbor.
#
# A second densa process busy-spins on the same CPU at normal priority (the neighbor);
# the measured worker runs rate-limited on that CPU at each priority level. Prints ticks/s,
# pacer resets (stalls over 10 ms) and snapshot age for the measured run, and the
# neighbor's own throughput (how much CPU it got).
#
# Usage: powershell -File scripts/priority_experiment.ps1 [-Cpu 14] [-Seconds 6] [-Runs 2]
param([int]$Cpu = 14, [int]$Seconds = 6, [int]$Runs = 2)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)

cargo build --release -q -p densa-poc
$exe = (Resolve-Path ".\target\release\densa-poc.exe").Path
$neighborOut = Join-Path ([IO.Path]::GetTempPath()) "densa_neighbor_$PID.txt"

function Format-Summary($line) {
    $rate = [regex]::Match($line, '\(([\d.]+[kM]?) ticks/s').Groups[1].Value
    $resets = [regex]::Match($line, '(\d+) pacer resets').Groups[1].Value
    $age = [regex]::Match($line, 'snapshot age p50 \S+ p99 (\S+) max (\S+)')
    "{0,8} ticks/s  {1,3} pacer resets  snapshot age p99 {2,-9} max {3}" -f `
        $rate, $resets, $age.Groups[1].Value, $age.Groups[2].Value
}

function Invoke-MeasuredRun($priority) {
    (& $exe --headless --rate 200000 --pin $Cpu --priority $priority --seconds $Seconds |
        Select-Object -Last 1)
}

"baseline (no neighbor), priority normal: " + (Format-Summary (Invoke-MeasuredRun "normal"))
foreach ($priority in "normal", "above", "high", "critical") {
    for ($i = 1; $i -le $Runs; $i++) {
        $neighbor = Start-Process -FilePath $exe -PassThru -NoNewWindow `
            -RedirectStandardOutput $neighborOut `
            -ArgumentList "--headless", "--unbounded", "--pin", "$Cpu", "--priority", "normal", `
                "--seconds", "$($Seconds + 2)"
        Start-Sleep -Seconds 1
        $measured = Invoke-MeasuredRun $priority
        $neighbor.WaitForExit()
        $neighborRate = [regex]::Match((Get-Content $neighborOut | Select-Object -Last 1),
            '\(([\d.]+[kM]?) ticks/s').Groups[1].Value
        "{0,-8} run {1}: {2} | neighbor {3} ticks/s" -f $priority, $i, (Format-Summary $measured), $neighborRate
    }
}
Remove-Item $neighborOut -ErrorAction SilentlyContinue
