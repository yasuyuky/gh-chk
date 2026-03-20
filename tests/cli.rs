use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const GH_CHK_TEST_STUB_BASE_URL: &str = "GH_CHK_TEST_STUB_BASE_URL";

struct StubServer {
    child: Option<Child>,
    graphql_base_url: String,
}

impl Drop for StubServer {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn find_python() -> &'static str {
    for cmd in ["python3", "python"] {
        if Command::new(cmd)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
        {
            return cmd;
        }
    }
    panic!("python3 or python is required to run stub tests");
}

fn reserve_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind port")
        .local_addr()
        .expect("read local addr")
        .port()
}

fn stub_script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/stub/server.py")
}

fn start_stub() -> StubServer {
    if let Ok(url) = std::env::var(GH_CHK_TEST_STUB_BASE_URL) {
        return StubServer {
            child: None,
            graphql_base_url: url.trim_end_matches('/').to_owned(),
        };
    }

    let port = reserve_port();
    let child = Command::new(find_python())
        .arg(stub_script())
        .args(["--port", &port.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start stub server");

    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return StubServer {
                child: Some(child),
                graphql_base_url: format!("http://127.0.0.1:{port}/graphql"),
            };
        }
        thread::sleep(Duration::from_millis(50));
    }

    panic!("stub server did not start");
}

fn run_cmd(args: &[&str], scenario: &str) -> String {
    let stub = start_stub();
    let output = Command::new("cargo")
        .args(["run", "--quiet", "--"])
        .args(args)
        .env("NO_COLOR", "1")
        .env("GITHUB_TOKEN", "test-token")
        .env(
            "GH_CHK_GRAPHQL_URL",
            format!("{}/{}", stub.graphql_base_url, scenario),
        )
        .output()
        .expect("run command");
    assert!(output.status.success(), "command failed: {:?}", output);
    String::from_utf8_lossy(&output.stdout).to_string()
}

#[test]
fn prs_output() {
    let out = run_cmd(&["-f", "json", "prs", "foo"], "prs");
    assert!(out.contains("\"mergeStateStatus\": \"CLEAN\""));
    assert!(out.contains("\"reviewDecision\": \"APPROVED\""));
}

#[test]
fn prs_text_includes_review_status() {
    let out = run_cmd(&["-f", "text", "prs", "foo"], "prs");
    assert!(out.contains("[approved]"));
}

#[test]
fn prs_pagination() {
    let out = run_cmd(&["-f", "json", "prs", "foo"], "prs_paginated");
    assert!(out.contains("\"title\": \"Test PR Page 1\""));
    assert!(out.contains("\"title\": \"Test PR Page 2\""));
}

#[test]
fn issues_output() {
    let out = run_cmd(&["-f", "json", "issues", "foo"], "issues");
    assert!(out.contains("Test Issue"));
}
