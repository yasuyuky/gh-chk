use std::process::Command;

fn run_cmd(args: &[&str], data: &str) -> String {
    let output = Command::new("cargo")
        .args(["run", "--quiet", "--"])
        .args(args)
        .env("NO_COLOR", "1")
        .env("GH_CHK_MOCK_FILE", format!("tests/data/{data}"))
        .output()
        .expect("run command");
    assert!(output.status.success(), "command failed: {:?}", output);
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn prs_output() {
    let out = run_cmd(&["-f", "json", "prs", "foo"], "prs.json");
    assert!(out.contains("\"mergeStateStatus\": \"CLEAN\""));
    assert!(out.contains("\"reviewDecision\": \"APPROVED\""));
}

#[test]
fn prs_text_includes_review_status() {
    let out = run_cmd(&["-f", "text", "prs", "foo"], "prs.json");
    assert!(out.contains("[approved]"));
}

#[test]
fn issues_output() {
    let out = run_cmd(&["-f", "json", "issues", "foo"], "issues.json");
    assert!(out.contains("Test Issue"));
}
