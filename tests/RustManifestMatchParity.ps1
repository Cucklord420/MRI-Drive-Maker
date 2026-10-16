# Original Windows PowerShell Assert-MRIManifestMatch versus Rust.
# Pure in-memory report comparisons; never enumerate, write, or erase USB disks.
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$repo=Split-Path $PSScriptRoot -Parent
Import-Module (Join-Path $repo 'MRI.Batch.psm1') -Force -DisableNameChecking
$binary=Join-Path $repo 'rust/mri-manifest/target/debug/mri-manifest.exe'
if (-not (Test-Path -LiteralPath $binary)) { throw 'Rust manifest compare binary not built' }
$temporary=Join-Path $env:TEMP ('mri-matching-fixtures-'+[guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($temporary)|Out-Null
$cases=0
try {
    for($i=0;$i -lt 50;$i++){
        for($variant=0;$variant -lt 12;$variant++){
            $filename=('EFI\BOOT\boot{0:D3}.efi' -f $i)
            $base=[pscustomobject]@{Entries=@(
                [pscustomobject]@{RelativePath='EFI';IsDirectory=$true;Length=0;Sha256=''}
                [pscustomobject]@{RelativePath=$filename;IsDirectory=$false;Length=100+$i;Sha256='AABBCCDD'}
            )}
            $actual=(ConvertFrom-Json -InputObject (ConvertTo-Json -InputObject $base -Depth 8))
            $mutable=@()
            $exact=$true
            $layoutOnly=$false
            switch ($variant) {
                0 {}
                1 {$actual.Entries[1].Sha256='FFFF0000'}
                2 {$actual.Entries[1].Sha256='FFFF0000';$layoutOnly=$true}
                3 {$actual.Entries=@($actual.Entries[0]);$exact=$false}
                4 {$actual.Entries+=([pscustomobject]@{RelativePath='UNEXPECTED.TXT';IsDirectory=$false;Length=1;Sha256='CC'})}
                5 {$actual.Entries+=([pscustomobject]@{RelativePath='UNEXPECTED.TXT';IsDirectory=$false;Length=1;Sha256='CC'});$exact=$false}
                6 {$actual.Entries[1].Length=101+$i}
                7 {$actual.Entries[1].Length=101+$i;$mutable=@($filename)}
                8 {$actual.Entries[1].Length=0;$mutable=@($filename)}
                9 {$actual.Entries[1].Sha256='aabbccdd'}
                10 {$actual.Entries[1].IsDirectory=$true}
                11 {$actual.Entries[1].RelativePath=$filename.ToUpperInvariant()}
            }
            $input=[pscustomobject]@{
                Expected=$base;Actual=$actual;MutableFiles=@($mutable)
                Exact=$exact;LayoutOnly=$layoutOnly
            }
            $path=Join-Path $temporary 'comparison.json'
            [IO.File]::WriteAllText($path,
                (ConvertTo-Json -InputObject $input -Compress -Depth 10),
                [Text.UTF8Encoding]::new($false))
            $out=& $binary --compare-json-file $path
            $status=$LASTEXITCODE
            if (-not $out){throw "Rust omitted output case $i/$variant"}
            $rust=$out | ConvertFrom-Json
            $psError=$null
            try {
                Assert-MRIManifestMatch -Expected $base -Actual $actual -MutableFiles $mutable -Exact:$exact -LayoutOnly:$layoutOnly
            } catch {
                $psError=$_.Exception.Message
            }
            if ($null -eq $psError) {
                if ($status -ne 0 -or -not $rust.Ok -or $null -ne $rust.Error){
                    throw "Allowed-case mismatch case $i/$variant Rust=$($rust.Error)"
                }
            } else {
                if ($status -eq 0 -or $rust.Ok -or $rust.Error -cne $psError){
                    throw "Rejected-case mismatch case $i/$variant PS=$psError Rust=$($rust.Error)"
                }
            }
            $cases++
        }
    }
    Write-Host "PASS: $cases direct PowerShell vs Rust final-manifest comparison fixtures, no disks touched."
} finally {
    Remove-Item -LiteralPath $temporary -Recurse -Force -ErrorAction SilentlyContinue
}
exit 0
