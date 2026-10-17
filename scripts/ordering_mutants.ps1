# Mutation test for the triple buffer's memory orderings.
#
# Rewrites the two `swap` orderings in triple_buffer.rs to each weaker variant, runs the
# loom models, and restores the file. The baseline (AcqRel/AcqRel) must pass and every
# weakened variant must fail; otherwise the loom models are too weak to guard the code.
#
# Usage: powershell -File scripts/ordering_mutants.ps1
$ErrorActionPreference = "Continue"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$file = Join-Path $root "crates/densa-sync/src/triple_buffer.rs"
# Byte-exact UTF-8 I/O: Windows PowerShell's Get-Content/Set-Content would re-encode
# the file through the ANSI codepage and add a BOM, corrupting non-ASCII characters.
$utf8 = New-Object System.Text.UTF8Encoding($false)
$original = [IO.File]::ReadAllText($file, $utf8)
$writerSwap = '.swap(self.back | DIRTY_BIT, Ordering::AcqRel)'
$readerSwap = '.swap(self.front, Ordering::AcqRel)'

if (-not ($original.Contains($writerSwap) -and $original.Contains($readerSwap))) {
    Write-Error "swap call sites not found in $file; update the patterns in this script"
    exit 2
}

# name = writer ordering, reader ordering, must pass
$variants = [ordered]@{
    "baseline (AcqRel/AcqRel)"                = @('AcqRel', 'AcqRel', $true)
    "prompt (Release/Acquire)"                = @('Release', 'Acquire', $false)
    "writer missing Acquire (Release/AcqRel)" = @('Release', 'AcqRel', $false)
    "reader missing Release (AcqRel/Acquire)" = @('AcqRel', 'Acquire', $false)
    "all Relaxed"                             = @('Relaxed', 'Relaxed', $false)
}

$env:RUSTFLAGS = "--cfg loom"
$env:CARGO_TARGET_DIR = "target/loom"
$unexpected = 0
try {
    foreach ($name in $variants.Keys) {
        $w, $r, $mustPass = $variants[$name]
        $mutated = $original.Replace($writerSwap, ".swap(self.back | DIRTY_BIT, Ordering::$w)").Replace($readerSwap, ".swap(self.front, Ordering::$r)")
        [IO.File]::WriteAllText($file, $mutated, $utf8)
        $out = cargo test -p densa-sync --release --test loom_triple_buffer 2>&1 | Out-String
        $passed = $out -match 'test result: ok'
        $failed = [regex]::Matches($out, '(?m)^test (\S+) \.\.\. FAILED') | ForEach-Object { $_.Groups[1].Value }
        $why = if ($out -match 'Causality violation') { "causality violation" } elseif ($out -match 'torn') { "torn snapshot" } else { "" }
        $verdict = if ($passed -eq $mustPass) { "ok" } else { $unexpected++; "UNEXPECTED" }
        "{0,-42} {1,-4} {2,-10} {3} {4}" -f $name, $(if ($passed) { "PASS" } else { "FAIL" }), $verdict, ($failed -join ","), $why
    }
} finally {
    [IO.File]::WriteAllText($file, $original, $utf8)
    Remove-Item Env:RUSTFLAGS
    Remove-Item Env:CARGO_TARGET_DIR
}

if ($unexpected -gt 0) {
    Write-Error "$unexpected variant(s) behaved unexpectedly"
    exit 1
}
"all ordering mutants behaved as expected"
