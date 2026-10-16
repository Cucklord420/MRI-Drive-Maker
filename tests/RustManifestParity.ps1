# Offline Windows PowerShell Get-MRIManifest vs Rust parity.
# Everything below is created in a disposable directory, never a USB drive.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = Split-Path $PSScriptRoot -Parent
Import-Module (Join-Path $repo 'MRI.Batch.psm1') -Force -DisableNameChecking
$rust = Join-Path $repo 'rust/mri-manifest/target/debug/mri-manifest.exe'
if (-not (Test-Path -LiteralPath $rust)) { throw 'Rust manifest binary missing' }

$base=Join-Path $env:TEMP ('mri-manifest-acceptance-' + [guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($base) | Out-Null
$cases=0
try {
    for($n=0;$n -lt 100;$n++){
        $root=Join-Path $base ('synthetic-{0:D3}' -f $n)
        $nested=Join-Path $root 'EFI/BOOT'
        [IO.Directory]::CreateDirectory($nested)|Out-Null
        [IO.File]::WriteAllBytes((Join-Path $root 'bootmgr'),[byte[]]@(0,1,2,3,$n))
        [IO.File]::WriteAllBytes((Join-Path $nested 'BOOTX64.EFI'),[byte[]]@(255,128,0,$n))
        if($n%2 -eq 0){
            [IO.File]::WriteAllBytes((Join-Path $root 'empty.bin'),[byte[]]@())
        }
        if($n%5 -eq 0){
            $system=Join-Path $root 'System Volume Information'
            [IO.Directory]::CreateDirectory($system)|Out-Null
            [IO.File]::WriteAllText((Join-Path $system 'Ignored.txt'),'synthetic')
        }
        foreach($ignore in @( $false,$true )){
            foreach($nohash in @($false,$true)){
                $arglist=@($root)
                if($ignore){ $arglist+='--ignore-system-directories' }
                if($nohash){ $arglist+='--no-hash' }
                $raw=& $rust @arglist
                $exit=$LASTEXITCODE
                if($exit -ne 0){throw "Rust manifest returned $exit for case $n"}
                $result=($raw | Out-String | ConvertFrom-Json)
                $parameters=@{Root=$root}
                if($ignore){$parameters['IgnoreSystemDirectories']=$true}
                if($nohash){$parameters['NoHash']=$true}
                $actual=Get-MRIManifest @parameters
                if($result.Digest -cne $actual.Digest){throw "Digest mismatch case $n ignore=$ignore nohash=$nohash Rust=$($result.Digest) PowerShell=$($actual.Digest)"}
                if([long]$result.TotalBytes -ne [long]$actual.TotalBytes){throw "Total size mismatch case $n"}
                $originalEntries=@($actual.Entries)
                $rustEntries=@($result.Entries)
                if($originalEntries.Count -ne $rustEntries.Count){throw "Entry count mismatch case $n"}
                for($j=0;$j -lt $rustEntries.Count;$j++){
                    foreach($property in @('RelativePath','IsDirectory','Length','Sha256')){
                        if([string]$rustEntries[$j].$property -cne [string]$originalEntries[$j].$property){
                            throw "Entry mismatch case $n, index $j, field $property"
                        }
                    }
                }
                $cases++
            }
        }
    }
    $missing=Join-Path $base 'nonexistent-source'
    & $rust $missing | Out-Null
    if($LASTEXITCODE -eq 0){throw 'Rust accepted a missing source directory'}
    Write-Host "PASS: $cases Rust vs original PowerShell manifest cases; read-only, synthetic directories, no disk operations."
} finally {
    Remove-Item -LiteralPath $base -Recurse -Force -ErrorAction SilentlyContinue
}
exit 0
