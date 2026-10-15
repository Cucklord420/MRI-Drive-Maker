# Offline parity against Get/ConvertTo-MRIRelativePath; no disks are accessed.
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$root=Split-Path $PSScriptRoot -Parent
Import-Module (Join-Path $root 'MRI.Batch.psm1') -Force -DisableNameChecking
$binary=Join-Path $root 'rust/mri-path-safety/target/debug/mri-path-safety.exe'
if (-not (Test-Path -LiteralPath $binary)) { throw 'Rust mri-path-safety binary not built' }

function Test-PathParity {
    param([string]$InputPath)
    $data=[pscustomobject]@{Path=$InputPath}
    $path=Join-Path $env:TEMP ([guid]::NewGuid().ToString('N')+'.json')
    try {
        [System.IO.File]::WriteAllText($path,(ConvertTo-Json -InputObject $data -Compress -Depth 4),
            [System.Text.UTF8Encoding]::new($false))
        $raw=& $binary --input-file $path
        $status=$LASTEXITCODE
        if (-not $raw) {throw 'Rust emitted no JSON'}
        $rust=$raw | ConvertFrom-Json
    } finally {
        Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
    }
    $original=$null
    $errorText=$null
    try {
        $original=ConvertTo-MRIRelativePath -Path $InputPath
    } catch {
        $errorText=$_.Exception.Message
    }
    if ($null -ne $errorText) {
        if ($status -eq 0 -or $null -ne $rust.Value -or $rust.Error -cne $errorText) {
            throw "Rejected-path mismatch for '$InputPath': Rust=$($rust.Error); Original=$errorText"
        }
    } elseif ($status -ne 0 -or $null -ne $rust.Error -or $rust.Value -cne $original) {
        throw "Accepted-path mismatch for '$InputPath': Rust=$($rust.Value); Original=$original"
    }
}

$cases=@('',' ','/root','\root','C:\root','C:root','a\b','a/b','a//b',
    'a/../b','a/./b','a/b.','a/b ','CON','con.txt','COM1','COM²',
    'LPT³.txt','NUL.log','PRN','AUX','CLOCK$','COM10','LPT10.txt',
    'CONTEXT.txt','name file.txt','first\second.txt','first/second.txt',
    'bad:name','foo?bar','a|b','a*b','x"y')
foreach($value in $cases){ Test-PathParity -InputPath $value }
for($i=0;$i -lt 200;$i++){
    Test-PathParity -InputPath ("folder{0}/sub{0}/file{0}.bin" -f $i)
    Test-PathParity -InputPath ("folder{0}/../file{0}.bin" -f $i)
    Test-PathParity -InputPath ("folder{0}/COM1.txt" -f $i)
}
Write-Host 'PASS: 635 original PowerShell vs Rust path validation fixtures; no disk operations.'
exit 0
