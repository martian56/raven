# Compile-time benchmark runner for local Windows use.
#
# Mirrors run.sh: builds the release raven compiler, times how long it
# takes to compile the corpus (best of a few runs), prints the result,
# and gates on a regression allowance over the stored baseline and an
# absolute wall-clock ceiling. CI uses run.sh; this is for local checks.
#
# Note: MSVC linking on Windows is markedly slower than the gcc link on
# the Linux CI runner, so local numbers run higher than CI numbers. The
# baseline in baseline.txt tracks the Linux CI measurement.
#
# Environment overrides match run.sh:
#   RAVEN_BENCH_THRESHOLD_PCT  (default 25)
#   RAVEN_BENCH_CEILING_S      (default 60)
#   RAVEN_BENCH_RUNS           (default 3)
#   RAVEN_BENCH_SKIP_BUILD     (set to 1 to skip cargo build)

$ErrorActionPreference = "Stop"

$ScriptDir = $PSScriptRoot
$RepoRoot = (Resolve-Path (Join-Path $ScriptDir "..\..")).Path
$CorpusEntry = Join-Path $ScriptDir "corpus\main.rv"
$BaselineFile = Join-Path $ScriptDir "baseline.txt"

function Env-Default($name, $fallback) {
    $v = [Environment]::GetEnvironmentVariable($name)
    if ([string]::IsNullOrEmpty($v)) { return $fallback }
    return $v
}

$ThresholdPct = [int](Env-Default "RAVEN_BENCH_THRESHOLD_PCT" 25)
$CeilingS = [int](Env-Default "RAVEN_BENCH_CEILING_S" 60)
$Runs = [int](Env-Default "RAVEN_BENCH_RUNS" 3)

if (-not (Test-Path $CorpusEntry)) {
    Write-Error "benchmark: corpus entry not found at $CorpusEntry (run generate.sh first)"
}

$RavenBin = Join-Path $RepoRoot "target\release\raven.exe"
if ((Env-Default "RAVEN_BENCH_SKIP_BUILD" "0") -ne "1") {
    Write-Host "benchmark: building release raven"
    & cargo build --release --bin raven --manifest-path (Join-Path $RepoRoot "Cargo.toml")
    if ($LASTEXITCODE -ne 0) { Write-Error "benchmark: cargo build failed" }
}
if (-not (Test-Path $RavenBin)) {
    Write-Error "benchmark: release raven binary not found at $RavenBin"
}

$OutDir = New-Item -ItemType Directory -Path (Join-Path $env:TEMP ("ravenbench_" + [Guid]::NewGuid().ToString("N"))) -Force
$OutBin = Join-Path $OutDir "corpus_out.exe"

Write-Host "benchmark: verifying corpus compiles"
& $RavenBin build $CorpusEntry -o $OutBin
if ($LASTEXITCODE -ne 0) { Write-Error "benchmark: corpus failed to compile" }

$bestMs = $null
for ($r = 1; $r -le $Runs; $r++) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $RavenBin build $CorpusEntry -o $OutBin | Out-Null
    $sw.Stop()
    $runMs = [int]$sw.Elapsed.TotalMilliseconds
    Write-Host "benchmark: run $r compiled corpus in $runMs ms"
    if ($null -eq $bestMs -or $runMs -lt $bestMs) { $bestMs = $runMs }
}

Remove-Item -Recurse -Force $OutDir

$bestS = [math]::Round($bestMs / 1000, 2)
Write-Host "benchmark: best corpus compile time $bestMs ms ($bestS s)"

$status = 0
$ceilingMs = $CeilingS * 1000
if ($bestMs -gt $ceilingMs) {
    Write-Host "benchmark: FAIL best $bestMs ms exceeds absolute ceiling $CeilingS s"
    $status = 1
} else {
    Write-Host "benchmark: OK under absolute ceiling $CeilingS s"
}

if (Test-Path $BaselineFile) {
    $baselineMs = [int](((Get-Content $BaselineFile -Raw) -replace '[^0-9]', ''))
    if ($baselineMs -gt 0) {
        $allowedMs = [int]($baselineMs * (100 + $ThresholdPct) / 100)
        Write-Host "benchmark: baseline $baselineMs ms, allowance $ThresholdPct%, ceiling $allowedMs ms"
        if ($bestMs -gt $allowedMs) {
            Write-Host "benchmark: FAIL best $bestMs ms regressed past baseline allowance $allowedMs ms"
            $status = 1
        } else {
            Write-Host "benchmark: OK within $ThresholdPct% of baseline"
        }
    }
} else {
    Write-Host "benchmark: no baseline file, skipping regression check (printed time only)"
}

exit $status
