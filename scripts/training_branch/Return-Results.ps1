param([string]$Out = ($PSScriptRoot + '-results.zip'))
. (Join-Path $PSScriptRoot 'Branch-Common.ps1')
$documents = Test-BranchInputs
if (-not (Test-Path -LiteralPath (Get-BranchPath 'run/state.json'))) { throw 'Run Train.ps1 before returning results' }
$outputPath = [IO.Path]::GetFullPath($Out)
if ([IO.File]::Exists($outputPath) -or [IO.Directory]::Exists($outputPath)) { throw "Output exists: $outputPath" }
$lock = [IO.File]::Open((Get-BranchPath 'run/.campaign.lock'), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::ReadWrite)
$locked = $false; $created = $false; $complete = $false; $stream = $null; $archive = $null
try {
    try { $lock.Lock(0, 1); $locked = $true } catch { throw 'Trainer is active; stop it and wait for exit before returning results' }
    $payloads = [Collections.Generic.Dictionary[string,byte[]]]::new([StringComparer]::Ordinal)
    foreach ($file in $documents.Manifest.files) {
        [byte[]]$bytes = Get-BranchBytes $file.path
        if ((Get-BranchHash $bytes) -ne $file.sha256) { throw "Input changed while snapshotting: $($file.path)" }
        $payloads.Add([string]$file.path, $bytes)
    }
    $payloads.Add('manifest.json', (Get-BranchBytes 'manifest.json'))
    $payloads.Add('run/state.json', (Get-BranchBytes 'run/state.json'))
    foreach ($folder in @('jobs', 'results')) {
        $relative = "run/work/$folder"
        $path = Get-BranchPath $relative
        if (Test-Path -LiteralPath $path) {
            foreach ($file in Get-ChildItem -LiteralPath $path -Filter '*.json' -File) {
                if ($file.Name -notmatch '^fnv1a64-[0-9a-f]{16}\.json$') { throw "Unexpected work file: $($file.Name)" }
                $name = "$relative/$($file.Name)"; $payloads.Add($name, (Get-BranchBytes $name))
            }
        }
    }
    $state = [Text.Encoding]::UTF8.GetString($payloads['run/state.json']) | ConvertFrom-Json
    if ($state.input_identity -ne $documents.Lineage.input_identity -or $state.backend_identity -ne $documents.Lineage.backend_identity) { throw 'Checkpoint identity differs from exported branch' }
    $records = @(foreach ($name in ($payloads.Keys | Sort-Object)) {
        [PSCustomObject]@{ path = $name; sha256 = (Get-BranchHash $payloads[$name]); bytes = $payloads[$name].Length }
    })
    $returned = [ordered]@{ format = 'minibar.branch_return'; version = 1; branch_id = $documents.Lineage.branch_id;
        state_sha256 = (Get-BranchHash $payloads['run/state.json']); completed_generations = $state.completed_generations; files = $records }
    $payloads.Add('return.json', ([Text.UTF8Encoding]::new($false).GetBytes(($returned | ConvertTo-Json -Depth 100))))
    [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($outputPath))
    Add-Type -AssemblyName System.IO.Compression
    $stream = [IO.File]::Open($outputPath, [IO.FileMode]::CreateNew, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None); $created = $true
    $archive = [IO.Compression.ZipArchive]::new($stream, [IO.Compression.ZipArchiveMode]::Create, $true)
    foreach ($name in ($payloads.Keys | Sort-Object)) {
        $entry = $archive.CreateEntry($name, [IO.Compression.CompressionLevel]::Optimal)
        $entryStream = $entry.Open()
        try { $bytes = $payloads[$name]; $entryStream.Write($bytes, 0, $bytes.Length) } finally { $entryStream.Dispose() }
    }
    $archive.Dispose(); $archive = $null; $stream.Dispose(); $stream = $null; $complete = $true
    Write-Host "Return archive: $outputPath (generation $($state.completed_generations))"
} finally {
    if ($null -ne $archive) { $archive.Dispose() }
    if ($null -ne $stream) { $stream.Dispose() }
    if ($locked) { $lock.Unlock(0, 1) }
    $lock.Dispose()
    if ($created -and -not $complete) { [IO.File]::Delete($outputPath) }
}
