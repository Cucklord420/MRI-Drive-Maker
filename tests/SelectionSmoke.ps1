#Requires -Version 5.1
# SPDX-License-Identifier: GPL-3.0-only
# Added 10-15-2026. Selection/consent tests; no physical disk commands.
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
Import-Module (Join-Path $root 'MRI.Batch.psm1') -Force -DisableNameChecking
$temp=Join-Path ([IO.Path]::GetTempPath()) ('mri-selection-'+[guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($temp)
$checks=0
function Check { param([bool]$Condition); if (-not $Condition) { throw 'Assertion failed.' } }
function Pass { param([string]$Name); $script:checks++; Write-Host "PASS $Name" }
function Throws {
    param([scriptblock]$Code)
    $caught=$false
    try { & $Code | Out-Null } catch { $caught=$true }
    Check $caught
}
function Drive {
    param([int]$Number,[string]$Serial)
    [pscustomobject]@{Number=$Number;Serial=$Serial;PnpId=('USB\'+$Serial);Model='TEST USB';Size=[long]8000000000
        Key=(Get-MRIDriveKey $Serial ('USB\'+$Serial) 'TEST USB' 8000000000)
        IsBoot=$false;IsSystem=$false;IsReadOnly=$false;IsOffline=$false;BusType='USB';IdentityError=''}
}
try {
    $cfg=Read-MRIJson (Join-Path $root 'mri-config.example.json')
    $source=Join-Path $temp 'Source'
    [void][IO.Directory]::CreateDirectory($source)
    [IO.File]::WriteAllText((Join-Path $source 'payload.txt'),'owned source')
    $cfg.SourcePath=$source; $cfg.SourceKind='Folder'
    $inventory=@((Drive 6 'A'),(Drive 7 'B'),(Drive 8 'C'))
    Check (@(Select-MRIUsbDrives ' 7,6,7 ' $inventory $cfg).Count -eq 2)
    Check (@(Select-MRIUsbDrives '' $inventory $cfg).Count -eq 0)
    Pass 'Explicit selection accepts comma-separated disks and removes duplicates'
    foreach ($bad in @('all','6,','6;7','-1','99999999999999999999','6,90')) { Throws { Select-MRIUsbDrives $bad $inventory $cfg } }
    Pass 'Malformed or partly missing selections cannot authorize a batch'
    Throws { Select-MRIUsbDrives '6,7' $inventory $cfg @($inventory[1].Key) }
    $inventory[1].IsSystem=$true
    Throws { Select-MRIUsbDrives '7' $inventory $cfg }
    $inventory[1].IsSystem=$false
    $duplicate=Drive 9 'A'
    Throws { Select-MRIUsbDrives '6' ($inventory+@($duplicate)) $cfg }
    Pass 'Protected source/system disks and duplicate serials remain excluded'

    # Load just the real selection/confirmation function without running a menu.
    $tokens=$null; $errors=$null
    $ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $root 'MRI-Batch.ps1'),[ref]$tokens,[ref]$errors)
    Check ($errors.Count -eq 0)
    $definition=$ast.Find({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Build-SelectedMedia'},$false)
    . ([scriptblock]::Create($definition.Extent.Text))
    $script:LiveInventory=$inventory; $script:Answers=[Collections.Generic.Queue[string]]::new()
    $script:Captured=@(); $script:SwapOnConfirm=$false
    $DataPath=Join-Path $temp 'Data'
    function Assert-WindowsAdmin { }
    function Get-Config { return $cfg }
    function Get-MRIInventory { return $script:LiveInventory }
    function Get-MRIProtectedKeys { return }
    function Show-Drives { }
    function Assert-MRIDrive {
        param($Record,$Config,$ProtectedKeys,$ExpectedNumber)
        $live=@($script:LiveInventory | Where-Object { $_.Key -eq $Record.Key -and $_.Number -eq $ExpectedNumber })
        if ($live.Count -ne 1) { throw 'Displayed physical identity changed.' }
    }
    function Watch-Media { param([array]$SessionRecords); $script:Captured=@($SessionRecords) }
    function Read-Host {
        param([string]$Prompt)
        $answer=$script:Answers.Dequeue()
        if ($script:SwapOnConfirm -and $Prompt -like 'Type*') { $script:LiveInventory=@((Drive 6 'REPLACEMENT'),$inventory[1],$inventory[2]) }
        return $answer
    }
    $script:Answers.Enqueue('6,7'); $script:Answers.Enqueue('yes')
    Build-SelectedMedia
    Check ($script:Captured.Count -eq 0)
    Pass 'A generic yes does not confirm the displayed erase list'
    $script:Answers.Enqueue('7,6,7'); $script:Answers.Enqueue('ERASE 6,7')
    Build-SelectedMedia
    Check ($script:Captured.Count -eq 2)
    Check ($script:Captured[0].Key -eq $inventory[1].Key -and $script:Captured[0].ExpectedNumber -eq 7)
    Check ($script:Captured[1].Key -eq $inventory[0].Key -and $script:Captured[1].AllowErase)
    Check (-not [IO.Directory]::Exists($DataPath))
    Pass 'Exact list confirmation creates only session approvals and writes no registrations'
    $approved=$script:Captured[1]
    $script:Captured=@(); $script:SwapOnConfirm=$true
    $script:Answers.Enqueue('6'); $script:Answers.Enqueue('ERASE 6')
    Throws { Build-SelectedMedia }
    Check ($script:Captured.Count -eq 0)
    Pass 'A USB replaced during confirmation cannot reach the batch runner'
    # A selected USB that disappears before its slot must end the finite
    # batch, record failure, and make the actual runner report failure.
    $definition=$ast.Find({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Watch-Media'},$false)
    # Dynamically compiled test functions have no file-based PSScriptRoot.
    $script:SelectionAppRoot=$root
    . ([scriptblock]::Create($definition.Extent.Text.Replace('$PSScriptRoot','$script:SelectionAppRoot')))
    $script:LiveInventory=@()
    $errorText=''
    try { Watch-Media -SessionRecords @($approved) } catch { $errorText=$_.Exception.Message }
    if ($errorText -notlike 'Batch failed or cancelled for 1 selected USB*') { throw "Unexpected batch result: $errorText" }
    $summaries=@(Get-ChildItem -LiteralPath $DataPath -Filter summary.json -Recurse)
    Check ($summaries.Count -eq 1)
    $summary=Read-MRIJson $summaries[0].FullName
    Check (@($summary.Failed).Count -eq 1 -and @($summary.Verified).Count -eq 0)
    Pass 'A missing selected USB ends the finite batch with a failed result'
    Write-Host "$checks/$checks selection checks passed."
} finally { Remove-Item -LiteralPath $temp -Recurse -Force -ErrorAction SilentlyContinue }
exit 0
