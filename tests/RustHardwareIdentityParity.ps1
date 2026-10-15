# Read-only native Rust vs original PowerShell hardware fingerprint parity.
$ErrorActionPreference='Stop'
Set-StrictMode -Version Latest
$root=Split-Path $PSScriptRoot -Parent
Import-Module (Join-Path $root 'MRI.Batch.psm1') -Force -DisableNameChecking
$tool=Join-Path $root 'rust/mri-safety/target/debug/mri-safety.exe'
if (-not (Test-Path -LiteralPath $tool)) { throw 'Rust fingerprint CLI is not built.' }

function Test-Identity {
    param([string]$Serial,[string]$PnpId,[string]$Model,[long]$Size)
    $data=[pscustomobject]@{Serial=$Serial;PnpId=$PnpId;Model=$Model;Size=$Size}
    $path=Join-Path $env:TEMP ([guid]::NewGuid().ToString('N')+'.json')
    try {
        $json=ConvertTo-Json -InputObject $data -Depth 6 -Compress
        [System.IO.File]::WriteAllText($path,$json,[System.Text.UTF8Encoding]::new($false))
        $resultText=& $tool --identity-json $path
        $resultCode=$LASTEXITCODE
        $result=$resultText | ConvertFrom-Json
    } finally {
        Remove-Item -LiteralPath $path -Force -ErrorAction SilentlyContinue
    }
    $original=$null
    $expectedError=$null
    try {
        $original=Get-MRIDriveKey -Serial $Serial -PnpId $PnpId -Model $Model -Size $Size
    } catch {
        $expectedError=$_.Exception.Message
    }
    if ($null -ne $expectedError) {
        if ($resultCode -eq 0 -or $result.Key -or $result.Error -cne $expectedError) {
            throw "Identity error mismatch for synthetic serial '$Serial': Rust '$($result.Error)' / PowerShell '$expectedError'"
        }
    } else {
        if ($resultCode -ne 0 -or $result.Error -or $result.Key -cne $original) {
            throw "Hardware identity mismatch for synthetic serial '$Serial'"
        }
    }
}
for($i=0;$i -lt 100;$i++){
    Test-Identity -Serial (" SER{0:D4} " -f $i) -PnpId (" usb\vid_{0:X4}&pid_1234 " -f $i) -Model ' Synthetic MRI media ' -Size ([long]32000000000+$i)
}
foreach($serial in @('',' ','0','0000','UNKNOWN','unknown','N/A','none','SERIAL 1')){
    Test-Identity -Serial $serial -PnpId 'USB\VID_1234' -Model 'Model' -Size 32000000000
}
Test-Identity -Serial 'SERIAL' -PnpId '' -Model 'Model' -Size 32000000000
Test-Identity -Serial 'SERIAL' -PnpId 'USB\VID_1234' -Model '' -Size 32000000000
Test-Identity -Serial 'SERIAL' -PnpId 'USB\VID_1234' -Model 'Model' -Size 0
Test-Identity -Serial 'SERIAL' -PnpId 'USB\VID_1234' -Model 'Model' -Size -1
Write-Host 'PASS: 113 synthetic SHA-256 hardware identity parity cases; no physical devices touched.'
exit 0
