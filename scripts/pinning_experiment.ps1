# Measures how worker placement affects unbounded ingest throughput on hybrid CPUs.
#
# Runs the headless PoC several times per placement (OS-scheduled, pinned to the
# `--pin auto` performance core, pinned to an efficiency core) and prints ticks/s and
# CPU migrations from each run's summary line.
#
# Usage: powershell -File scripts/pinning_experiment.ps1 [-Runs 3] [-Seconds 5] [-ECore 20]
param([int]$Runs = 3, [int]$Seconds = 5, [int]$ECore = 20)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)

cargo build --release -q -p trading-demo
$exe = ".\target\release\trading-demo.exe"
$placements = [ordered]@{ "os-scheduled" = "none"; "p-core (auto)" = "auto"; "e-core $ECore" = "$ECore" }

foreach ($name in $placements.Keys) {
    for ($i = 1; $i -le $Runs; $i++) {
        $summary = & $exe --headless --unbounded --batch 1024 --seconds $Seconds --pin $placements[$name] |
            Select-Object -Last 1
        $rate = [regex]::Match($summary, '\(([\d.]+[kM]?) ticks/s').Groups[1].Value
        $cpu = [regex]::Match($summary, 'worker cpu (\S+), migrations (\d+)')
        "{0,-16} run {1}: {2,8} ticks/s  last cpu {3,-3} migrations {4}" -f `
            $name, $i, $rate, $cpu.Groups[1].Value.TrimEnd(','), $cpu.Groups[2].Value
    }
}
