use std::fs;
use std::path::PathBuf;

#[test]
fn copy_worker_delegates_authenticated_plans_to_the_rust_helper() {
    let worker = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("Copy-MRIFiles.ps1");
    let source = fs::read_to_string(&worker).expect("Copy-MRIFiles.ps1 must exist");

    assert!(source.contains("Get-MRIWorkerExecutable"));
    assert!(source.contains("& $worker --copy $PlanPath $ExpectedHash"));
    assert!(source.contains("if ($LASTEXITCODE -ne 0)"));
    assert!(!source.contains("[MRIBatch.NativeFiles]::Copy"));
    assert!(!source.contains("foreach ($entry in @($plan.Manifest.Entries))"));
}
