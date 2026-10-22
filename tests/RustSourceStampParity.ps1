# Compare Rust source-stamp guards with the original Windows PowerShell.
# Disposable local files only; never enumerates or modifies physical USB media.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$repo = Split-Path $PSScriptRoot -Parent
Import-Module (Join-Path $repo 'MRI.Batch.psm1') -Force -DisableNameChecking
$binary = Join-Path $repo 'rust/mri-manifest/target/debug/source_stamp.exe'
if (-not [IO.File]::Exists($binary)) { throw 'Rust source-stamp binary was not built.' }

$tempRoot = Join-Path $env:TEMP ('mri-source-stamp-'+[guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($tempRoot)
$tested = 0
try {
    for ($case = 0; $case -lt 20; $case++) {
        $file = Join-Path $tempRoot ('source-{0:D2}.iso' -f $case)
        [IO.File]::WriteAllBytes($file, [byte[]]@(0, 1, 2, [byte]$case))
        if ($case -ge 10 -and $case -lt 15) {
            # This date is representable in .NET/Windows but precedes Unix time.
            [IO.File]::SetLastWriteTimeUtc(
                $file,
                [datetime]::SpecifyKind([datetime]'1965-06-15 12:00:00', [DateTimeKind]::Utc)
            )
        }
        $info = Get-Item -LiteralPath $file -Force
        $stamp = [ordered]@{
            Path = [string]$file
            Length = [long]$info.Length
            WriteTicks = [long]$info.LastWriteTimeUtc.Ticks
        }
        $config = [ordered]@{ InputStamp = $stamp }
        switch ($case) {
            0 { $config = [ordered]@{} }
            1 { $config.InputStamp = $null }
            2 { $config = [ordered]@{ inputstamp = $stamp } }
            3 {
                $config = [ordered]@{ INPUTSTAMP = [ordered]@{
                    path=$stamp.Path; length=$stamp.Length; writeticks=$stamp.WriteTicks
                }}
            }
            4 { $stamp.Length++ }
            5 { $stamp.WriteTicks++ }
            6 { [IO.File]::WriteAllBytes($file, [byte[]]@(1,2,3,4,5)) }
            7 { [IO.File]::SetLastWriteTimeUtc($file, $info.LastWriteTimeUtc.AddSeconds(3)) }
            8 { $stamp.Path = Join-Path $tempRoot 'missing.iso' }
            9 { $stamp.Remove('WriteTicks') }
            10 { }
            11 { $stamp.Length++ }
            12 { $stamp.WriteTicks++ }
            13 { $config = [ordered]@{ inputstamp = [ordered]@{
                    PATH=$stamp.Path; LENGTH=$stamp.Length; WRITETICKS=$stamp.WriteTicks
                }} }
            14 { [IO.File]::SetLastWriteTimeUtc($file, $info.LastWriteTimeUtc.AddSeconds(1)) }
            15 { $config = [ordered]@{ InputStamp = $false } }
            16 { $config = [ordered]@{ InputStamp = [ordered]@{ Path=$file } } }
            17 { $stamp.Path = Join-Path $tempRoot 'nonexistent.iso' }
            18 { $stamp.Length = 0 }
            19 { $config = [ordered]@{ InputStamp = $stamp } }
        }
        $json = ConvertTo-Json -InputObject $config -Compress -Depth 10
        $referenceOk = $false
        try {
            $originalConfig = ConvertFrom-Json -InputObject $json
            Assert-MRIInputSource $originalConfig
            $referenceOk = $true
        } catch {
            $referenceOk = $false
        }
        $inputFile = Join-Path $tempRoot 'request.json'
        [IO.File]::WriteAllText($inputFile, $json, [Text.UTF8Encoding]::new($false))
        $raw = & $binary $inputFile
        $exitCode = $LASTEXITCODE
        if (-not $raw) { throw "Rust did not return a JSON decision for case $case." }
        $result = ($raw | Out-String | ConvertFrom-Json)
        $rustOk = $exitCode -eq 0 -and [bool]$result.Ok
        if ($referenceOk -ne $rustOk) {
            throw "Source stamp original/Rust mismatch case $case (original=$referenceOk Rust=$rustOk)."
        }
        $tested++
    }
    Write-Host "PASS: $tested original PowerShell/Rust source-stamp decisions, including pre-1970 UTC."
} finally {
    Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
}
