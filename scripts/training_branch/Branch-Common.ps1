Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$script:BranchRoot = [IO.Path]::GetFullPath($PSScriptRoot)

function Get-BranchPath([string]$Relative) {
    if ([string]::IsNullOrWhiteSpace($Relative) -or $Relative -match '[\\:\x00]' -or
        ($Relative.Split('/') | Where-Object { $_ -eq '' -or $_ -eq '.' -or $_ -eq '..' -or $_ -match '[. ]$' })) {
        throw "Unsafe package path: $Relative"
    }
    $full = [IO.Path]::GetFullPath((Join-Path $script:BranchRoot $Relative))
    $prefix = $script:BranchRoot + [IO.Path]::DirectorySeparatorChar
    if (-not $full.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) { throw 'Path escapes package' }
    $current = $full
    while ($current.Length -ge $script:BranchRoot.Length) {
        if ([IO.File]::Exists($current) -or [IO.Directory]::Exists($current)) {
            if (([IO.File]::GetAttributes($current) -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
                throw "Reparse/symlink package path: $current"
            }
        }
        if ($current -eq $script:BranchRoot) { break }
        $current = [IO.Path]::GetDirectoryName($current)
    }
    return $full
}

function Get-BranchBytes([string]$Relative) { return ,([IO.File]::ReadAllBytes((Get-BranchPath $Relative))) }
function Get-BranchHash([byte[]]$Bytes) {
    $hash = [Security.Cryptography.SHA256]::Create()
    try { return [BitConverter]::ToString($hash.ComputeHash($Bytes)).Replace('-', '').ToLowerInvariant() }
    finally { $hash.Dispose() }
}
function Read-BranchJson([string]$Relative) {
    return ([Text.Encoding]::UTF8.GetString((Get-BranchBytes $Relative)) | ConvertFrom-Json)
}
function Test-BranchInputs {
    $manifest = Read-BranchJson 'manifest.json'
    if ($manifest.format -ne 'minibar.training_package' -or $manifest.version -ne 1) { throw 'Unsupported package manifest' }
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    foreach ($file in $manifest.files) {
        if (-not $seen.Add([string]$file.path)) { throw 'Duplicate package path' }
        [byte[]]$bytes = Get-BranchBytes $file.path
        if ($bytes.Length -ne $file.bytes -or (Get-BranchHash $bytes) -ne $file.sha256) { throw "Package checksum mismatch: $($file.path)" }
    }
    foreach ($name in @($manifest.binary, $manifest.config, 'branch.json')) {
        if (-not $seen.Contains($name)) { throw "Missing manifest input: $name" }
    }
    $lineage = Read-BranchJson 'branch.json'
    if ($lineage.format -ne 'minibar.training_branch' -or $lineage.version -ne 1) { throw 'Unsupported training branch' }
    return [PSCustomObject]@{ Manifest = $manifest; Lineage = $lineage }
}
