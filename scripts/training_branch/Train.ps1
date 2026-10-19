param(
    [ValidateRange(1, 4096)][int]$Threads = [Environment]::ProcessorCount,
    [ValidateRange(0, 1000000)][int]$Generations = 0,
    [double]$MaxSeconds = 0
)
. (Join-Path $PSScriptRoot 'Branch-Common.ps1')
if ([double]::IsNaN($MaxSeconds) -or [double]::IsInfinity($MaxSeconds) -or $MaxSeconds -lt 0) { throw 'MaxSeconds must be finite and nonnegative' }
$documents = Test-BranchInputs
$arguments = @('--train-campaign', (Get-BranchPath $documents.Manifest.config), '--train-out', (Get-BranchPath 'run'), '--threads', [string]$Threads)
if (Test-Path -LiteralPath (Get-BranchPath 'run/state.json')) { $arguments += '--resume' }
if ($Generations -gt 0) { $arguments += @('--generations', [string]$Generations) }
if ($MaxSeconds -gt 0) { $arguments += @('--max-seconds', $MaxSeconds.ToString('R', [Globalization.CultureInfo]::InvariantCulture)) }
Write-Host "Training independent branch $($documents.Lineage.branch_id); progress: run/progress.json"
$runPath = Get-BranchPath 'run'
$busyCommitPattern = 'commit ' + [regex]::Escape($runPath) + '[\\/][^\r\n]+:.*\(os error (5|32|33)\)'
$retry = 0
$trainingClock = [Diagnostics.Stopwatch]::StartNew()
while ($true) {
    $busyCommit = $false
    # Windows readers can briefly block atomic replacement. Resume the same
    # executable and committed optimizer/cache; never alter checkpoint identity.
    # Continue here also lets Windows PowerShell 5 stream native stderr without
    # treating each redirected progress line as a terminating PowerShell error.
    $savedErrorPreference = $ErrorActionPreference
    try {
        $ErrorActionPreference = 'Continue'
        & (Get-BranchPath $documents.Manifest.binary) @arguments 2>&1 | ForEach-Object {
            $line = $_.ToString()
            if ($line -match $busyCommitPattern) { $busyCommit = $true }
            Write-Output $line
        }
        $nativeExit = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $savedErrorPreference
    }
    if ($nativeExit -eq 0) { break }
    if (-not $busyCommit -or $retry -ge 5) { throw "Native training failed (exit $nativeExit)" }
    $retry++
    $delay = [int][Math]::Min(2000, 250 * [Math]::Pow(2, $retry - 1))
    Write-Host "Checkpoint temporarily busy; retrying resume ($retry/5) in ${delay}ms."
    Start-Sleep -Milliseconds $delay
    if ((Test-Path -LiteralPath (Get-BranchPath 'run/state.json')) -and $arguments -notcontains '--resume') {
        $arguments += '--resume'
    }
    $documents = Test-BranchInputs
    if ($MaxSeconds -gt 0) {
        # Keep one cooperative time budget across retries. If it has expired,
        # a minimal positive budget lets native resume repair exports and stop.
        $remaining = [Math]::Max(0.000000001, $MaxSeconds - $trainingClock.Elapsed.TotalSeconds)
        $limitIndex = [Array]::IndexOf($arguments, '--max-seconds') + 1
        $arguments[$limitIndex] = $remaining.ToString('R', [Globalization.CultureInfo]::InvariantCulture)
    }
}
