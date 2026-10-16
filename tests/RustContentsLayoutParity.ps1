# Original PowerShell Assert-MRIContentsSafe versus the Rust layout preflight.
# Uses only disposable local fixture folders; never enumerates, formats or writes USB targets.
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repo = Split-Path $PSScriptRoot -Parent
Import-Module (Join-Path $repo 'MRI.Batch.psm1') -Force -DisableNameChecking
$binary = Join-Path $repo 'rust/mri-manifest/target/debug/contents_layout.exe'
if (-not (Test-Path -LiteralPath $binary)) { throw 'Rust contents layout binary not built' }
$tempRoot = Join-Path $env:TEMP ('mri-contents-parity-' + [Guid]::NewGuid().ToString('N'))
[IO.Directory]::CreateDirectory($tempRoot) | Out-Null
$cases = 0

function Assert-DecisionParity {
    param([string]$Root, $Receipt, [string]$FixturePath, [string]$CaseName)
    $current = Get-MRIManifest -Root $Root -IgnoreSystemDirectories -NoHash
    $payload = [pscustomobject]@{ CurrentEntries = @($current.Entries); Receipt = $Receipt }
    [IO.File]::WriteAllText(
        $FixturePath,
        (ConvertTo-Json -InputObject $payload -Compress -Depth 12),
        [Text.UTF8Encoding]::new($false)
    )
    $output = & $binary $FixturePath
    $status = $LASTEXITCODE
    if (-not $output) { throw "Rust did not produce JSON for $CaseName" }
    $rust = $output | Out-String | ConvertFrom-Json

    $oracleResult = $false
    $oracleError = $null
    try {
        $oracleResult = [bool](Assert-MRIContentsSafe -Root $Root -Receipt $Receipt)
    } catch {
        $oracleError = $_.Exception.Message
    }

    if ($null -eq $oracleError) {
        if ($status -ne 0 -or -not $rust.Ok -or [bool]$rust.Result -ne $oracleResult) {
            throw "Success decision mismatch: $CaseName Rust=$($rust | ConvertTo-Json -Compress)"
        }
    } elseif ($status -eq 0 -or $rust.Ok -or $rust.Error -cne $oracleError) {
        throw "Failure decision mismatch: $CaseName PowerShell=$oracleError Rust=$($rust.Error)"
    }
}

try {
    for ($n = 0; $n -lt 50; $n++) {
        $root = Join-Path $tempRoot ("root-{0:D3}" -f $n)
        [IO.Directory]::CreateDirectory((Join-Path $root 'EFI/BOOT')) | Out-Null
        $boot = Join-Path $root 'bootmgr'
        $efi = Join-Path $root 'EFI/BOOT/BOOTX64.EFI'
        $content = [byte[]]@(0,1,2,($n % 256))
        [IO.File]::WriteAllBytes($boot, $content)
        [IO.File]::WriteAllBytes($efi, [byte[]]@(255,128,0,($n % 256)))
        $manifest = Get-MRIManifest -Root $root -IgnoreSystemDirectories
        $receipt = [pscustomobject]@{ TargetManifest = $manifest }
        $fixturePath = Join-Path $tempRoot 'input.json'

        Assert-DecisionParity $root $receipt $fixturePath "valid-$n"
        $cases++

        $extra = Join-Path $root 'UNEXPECTED.TXT'
        [IO.File]::WriteAllText($extra, 'fictional')
        Assert-DecisionParity $root $receipt $fixturePath "extra-$n"
        $cases++
        [IO.File]::Delete($extra)

        [IO.File]::Delete($boot)
        Assert-DecisionParity $root $receipt $fixturePath "missing-$n"
        $cases++
        [IO.File]::WriteAllBytes($boot, $content)

        [IO.File]::WriteAllBytes($boot, [byte[]]@(0))
        Assert-DecisionParity $root $receipt $fixturePath "resized-$n"
        $cases++
        [IO.File]::WriteAllBytes($boot, $content)

        # PowerShell's hashtable deduplicates expected keys before the count
        # check. A duplicate receipt entry must not fabricate an extra path.
        $duplicate = [pscustomobject]@{
            TargetManifest = [pscustomobject]@{
                Entries = @($manifest.Entries) + @($manifest.Entries[0])
            }
        }
        Assert-DecisionParity $root $duplicate $fixturePath "duplicate-receipt-$n"
        $cases++
    }
    Write-Host "PASS: $cases Rust layout preflight / original PowerShell decisions using disposable folders."
} finally {
    Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
}
exit 0
