# Windows PowerShell Assert-MRIContentsSafe versus read-only Rust --contents-safe.
# Every target is a disposable local directory; no USB enumeration or erasure.
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$repo=Split-Path $PSScriptRoot -Parent
Import-Module (Join-Path $repo 'MRI.Batch.psm1') -Force -DisableNameChecking
$binary=Join-Path $repo 'rust/mri-manifest/target/debug/mri-manifest.exe'
if (-not (Test-Path -LiteralPath $binary)) { throw 'Rust manifest binary not built' }
$temporary=Join-Path $env:TEMP ('mri-contents-safe-'+[guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($temporary)|Out-Null
$cases=0
try {
    for($n=0;$n -lt 60;$n++) {
        for($variant=0;$variant -lt 13;$variant++) {
            $root=Join-Path $temporary ('case-{0:D3}-{1:D2}' -f $n,$variant)
            $nested=Join-Path $root 'EFI/BOOT'
            [IO.Directory]::CreateDirectory($nested)|Out-Null
            $boot=Join-Path $root 'bootmgr'
            [IO.File]::WriteAllBytes($boot,[byte[]]@(0,1,2,[byte]$n))
            $efi=Join-Path $nested 'BOOTX64.EFI'
            [IO.File]::WriteAllBytes($efi,[byte[]]@(255,128,0,[byte]$n))
            $system=Join-Path $root 'System Volume Information'
            if ($n%3 -eq 0) {
                [IO.Directory]::CreateDirectory($system)|Out-Null
                [IO.File]::WriteAllText((Join-Path $system 'Ignored.txt'),'synthetic')
            }
            $receipt=[pscustomobject]@{
                TargetManifest=(Get-MRIManifest -Root $root -IgnoreSystemDirectories)
            }
            switch($variant) {
                0 {}
                1 {$receipt=$null}
                2 {[IO.File]::WriteAllBytes((Join-Path $root 'unexpected.bin'),[byte[]]@(4,5,6))}
                3 {Remove-Item -LiteralPath $boot -Force}
                4 {[IO.File]::WriteAllBytes($boot,[byte[]]@(9,8,7,[byte]$n))}
                5 {Move-Item -LiteralPath $boot -Destination (Join-Path $root 'changed-name')}
                6 {[IO.File]::WriteAllBytes($boot,[byte[]]@(0,1,2,3,[byte]$n))}
                7 {
                    foreach($entry in @($receipt.TargetManifest.Entries)) {
                        if ($entry.RelativePath -eq 'bootmgr') { $entry.Sha256='0000' }
                    }
                }
                8 {
                    Remove-Item -LiteralPath $nested -Recurse -Force
                    Remove-Item -LiteralPath (Join-Path $root 'EFI') -Force
                    Remove-Item -LiteralPath $boot -Force
                    if(Test-Path -LiteralPath $system){ Remove-Item -LiteralPath $system -Recurse -Force }
                }
                9 {
                    Remove-Item -LiteralPath (Join-Path $root 'EFI') -Recurse -Force
                    Remove-Item -LiteralPath $boot -Force
                    [IO.Directory]::CreateDirectory($system)|Out-Null
                    [IO.File]::WriteAllText((Join-Path $system 'Ignored.txt'),'synthetic')
                }
                10 {[IO.Directory]::CreateDirectory((Join-Path $root 'rogue-directory'))|Out-Null}
                11 {
                    $receipt.TargetManifest.Entries=@($receipt.TargetManifest.Entries | Where-Object { $_.RelativePath -ne 'bootmgr' })
                }
                12 {
                    foreach($entry in @($receipt.TargetManifest.Entries)) {
                        if (-not $entry.IsDirectory) { $entry.Sha256=([string]$entry.Sha256).ToLowerInvariant() }
                    }
                }
            }
            $receiptFile='-'
            if($null -ne $receipt) {
                $receiptFile=Join-Path $temporary 'receipt.json'
                $json=ConvertTo-Json -InputObject $receipt -Compress -Depth 12
                [IO.File]::WriteAllText($receiptFile,$json,[Text.UTF8Encoding]::new($false))
            }
            $raw=& $binary --contents-safe $root $receiptFile
            $status=$LASTEXITCODE
            if(-not $raw){throw "Rust produced no output for $n/$variant"}
            $rust=($raw | Out-String | ConvertFrom-Json)
            $psError=$null
            $psSafe=$false
            try {
                $psSafe=[bool](Assert-MRIContentsSafe -Root $root -Receipt $receipt)
            } catch {
                $psError=$_.Exception.Message
            }
            if($null -eq $psError) {
                if($status -ne 0 -or $null -ne $rust.Error -or [bool]$rust.Safe -ne $psSafe) {
                    throw "Decision mismatch case $n/$variant PS=$psSafe Rust=$($rust.Safe) Error=$($rust.Error)"
                }
            } else {
                if($status -eq 0 -or $rust.Safe -or $rust.Error -cne $psError) {
                    throw "Rejection mismatch case $n/$variant PS=$psError Rust=$($rust.Error)"
                }
            }
            $cases++
        }
    }
    Write-Host "PASS: $cases direct PowerShell/Rust receipt-guard fixtures; all local, synthetic and read-only."
} finally {
    Remove-Item -LiteralPath $temporary -Recurse -Force -ErrorAction SilentlyContinue
}
exit 0
