use dreamseq::DreamseqConfig;
use std::fs;
use std::process::Command;

#[test]
fn run_and_report_are_read_only_for_dreams_by_default() {
    let root = std::env::temp_dir().join(format!("dreamseq-cli-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let config_path = root.join("config.json");
    let config = DreamseqConfig {
        harnesses: Vec::new(),
        output_dir: root.join("output"),
        anthologies_dir: root.join("anthologies"),
        enable_kaptaind: false,
        ..DreamseqConfig::default()
    };
    fs::write(&config_path, serde_json::to_vec_pretty(&config).unwrap()).unwrap();

    let run = Command::new(env!("CARGO_BIN_EXE_dreamseq"))
        .args(["run", "--config"])
        .arg(&config_path)
        .arg("--json")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let run_report: serde_json::Value = serde_json::from_slice(&run.stdout).unwrap();
    assert_eq!(run_report["analysis"]["raw_entries"], 0);
    assert!(!root.join(".dreams").exists());

    let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let report = Command::new(env!("CARGO_BIN_EXE_dreamseq"))
        .arg("report")
        .arg(date)
        .args(["--config"])
        .arg(&config_path)
        .arg("--json")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        report.status.success(),
        "report failed: {}",
        String::from_utf8_lossy(&report.stderr)
    );
    let report_json: serde_json::Value = serde_json::from_slice(&report.stdout).unwrap();
    assert_eq!(report_json["analysis"]["raw_entries"], 0);
    assert!(!root.join(".dreams").exists());

    fs::remove_dir_all(root).unwrap();
}
