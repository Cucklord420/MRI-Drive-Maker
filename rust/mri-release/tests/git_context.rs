use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use zip::ZipArchive;

fn environment_without_git() -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    std::env::vars_os()
        .filter(|(key, _)| {
            !key.to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("GIT_")
        })
        .collect()
}

fn git(repo: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .args(args)
        .current_dir(repo)
        .env_clear()
        .envs(environment_without_git())
        .output()
        .unwrap()
}

fn git_success(repo: &Path, args: &[&str]) -> Vec<u8> {
    let output = git(repo, args);
    assert!(
        output.status.success(),
        "Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn synthetic_alternate(path: &Path) -> String {
    fs::create_dir_all(path).unwrap();
    git_success(path, &["init", "-q"]);
    git_success(path, &["config", "core.autocrlf", "false"]);
    git_success(
        path,
        &["config", "user.name", "MRI release adversarial fixture"],
    );
    git_success(
        path,
        &["config", "user.email", "mri-release@example.invalid"],
    );
    fs::write(path.join("SOURCE.txt"), "alternate source bytes\n").unwrap();
    fs::write(
        path.join("START-MRI-DRIVE-MAKER.cmd"),
        "@echo alternate\r\n",
    )
    .unwrap();
    git_success(path, &["add", "SOURCE.txt", "START-MRI-DRIVE-MAKER.cmd"]);
    git_success(path, &["commit", "-m", "alternate source fixture"]);
    String::from_utf8(git_success(path, &["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_owned()
}

fn run_release(arguments: &[&Path], environment: &[(String, String)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_mri-release"));
    command
        .args(arguments)
        .env_clear()
        .envs(environment_without_git());
    command.envs(environment.iter().map(|(key, value)| (key, value)));
    command.output().unwrap()
}

fn run_python(arguments: &[&Path], environment: &[(String, String)]) -> Output {
    let mut command = Command::new("python3");
    command
        .args(arguments)
        .env_clear()
        .envs(environment_without_git());
    command.envs(environment.iter().map(|(key, value)| (key, value)));
    command.output().unwrap()
}

fn environment_with_overrides(overrides: &[(&str, String)]) -> Vec<(String, String)> {
    overrides
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

#[test]
fn rust_command_line_build_and_verify_ignore_inherited_git_context() {
    let temp = tempfile::tempdir().unwrap();
    let alternate = temp.path().join("alternate");
    let alternate_commit = synthetic_alternate(&alternate);
    let alternate_git_dir = alternate.join(".git");
    let config = temp.path().join("attacker.gitconfig");
    fs::write(&config, "[mri]\n    injected = attacker\n").unwrap();
    let overrides = [
        ("GIT_DIR", alternate_git_dir.display().to_string()),
        ("GIT_WORK_TREE", alternate.display().to_string()),
        ("GIT_COMMON_DIR", alternate_git_dir.display().to_string()),
        (
            "GIT_OBJECT_DIRECTORY",
            alternate_git_dir.join("objects").display().to_string(),
        ),
        (
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            alternate_git_dir.join("objects").display().to_string(),
        ),
        ("GIT_CONFIG_COUNT", "1".to_owned()),
        ("GIT_CONFIG_KEY_0", "mri.injected".to_owned()),
        ("GIT_CONFIG_VALUE_0", "attacker".to_owned()),
        ("GIT_CONFIG", config.display().to_string()),
        ("GIT_CONFIG_GLOBAL", config.display().to_string()),
        ("GIT_CONFIG_SYSTEM", config.display().to_string()),
        (
            "GIT_CONFIG_PARAMETERS",
            "'mri.injected'='attacker'".to_owned(),
        ),
    ];
    let environment = environment_with_overrides(&overrides);
    let archive = temp.path().join("protected-release.zip");
    let archive_arg = PathBuf::from("--output");
    let output_path = archive.as_path();
    let args = [archive_arg.as_path(), output_path];
    let built = run_release(&args, &environment);
    assert!(
        built.status.success(),
        "Rust CLI build failed under Git overrides: {}",
        String::from_utf8_lossy(&built.stderr)
    );
    let output: serde_json::Value = serde_json::from_slice(&built.stdout).unwrap();
    let current_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let python_script = current_root.join("scripts/Build-Download.py");
    let python_archive = temp.path().join("python-protected-release.zip");
    let python_args = [
        python_script.as_path(),
        Path::new("--output"),
        python_archive.as_path(),
    ];
    let python_built = run_python(&python_args, &environment);
    assert!(
        python_built.status.success(),
        "Python CLI build failed under Git overrides: {}",
        String::from_utf8_lossy(&python_built.stderr)
    );
    let python_output: serde_json::Value = serde_json::from_slice(&python_built.stdout).unwrap();
    let intended_commit = String::from_utf8(git_success(&current_root, &["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_owned();
    assert_eq!(output["source_commit"], intended_commit);
    assert_eq!(python_output["source_commit"], intended_commit);
    assert_ne!(intended_commit, alternate_commit);
    assert_eq!(
        fs::read(&archive).unwrap(),
        fs::read(&python_archive).unwrap(),
        "Rust and original Python output differ under hostile Git environment variables"
    );

    let mut zip = ZipArchive::new(fs::File::open(&archive).unwrap()).unwrap();
    let mut release_manifest = String::new();
    zip.by_name("MRI-Drive-Maker-v0.3.0/RELEASE.json")
        .unwrap()
        .read_to_string(&mut release_manifest)
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&release_manifest).unwrap();
    assert_eq!(manifest["source_commit"], intended_commit);

    let verify_args = [Path::new("--verify"), python_archive.as_path()];
    let verified = run_release(&verify_args, &environment);
    assert!(
        verified.status.success(),
        "Rust CLI --verify failed under Git overrides: {}",
        String::from_utf8_lossy(&verified.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&verified.stdout).unwrap();
    assert_eq!(result["source_commit"], intended_commit);
    let python_verify_args = [
        python_script.as_path(),
        Path::new("--verify"),
        archive.as_path(),
    ];
    let python_verified = run_python(&python_verify_args, &environment);
    assert!(
        python_verified.status.success(),
        "Python CLI --verify failed under Git overrides: {}",
        String::from_utf8_lossy(&python_verified.stderr)
    );
    let python_result: serde_json::Value = serde_json::from_slice(&python_verified.stdout).unwrap();
    assert_eq!(python_result["source_commit"], intended_commit);
}
