use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

const BIN: &str = env!("CARGO_BIN_EXE_ktp");

#[test]
fn cli_json_and_atomic_ics_export() {
    let out = Command::new(BIN)
        .args(["--demo", "hw", "--days", "7", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let data: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(data["upcoming"][0]["title"], "组合逻辑练习");
    assert_eq!(data["overdue"][0]["status"], "被退回");
    assert_eq!(data["term"], "1");

    let out = Command::new(BIN)
        .args([
            "--demo",
            "courses",
            "--semester",
            "2025-2026",
            "--term",
            "2",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let data: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(data["items"][0]["title"], "计算机导论");

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("作业截止.ics");
    let export = || {
        Command::new(BIN)
            .args(["--demo", "ics", "-o"])
            .arg(&path)
            .output()
            .unwrap()
    };
    assert!(export().status.success());
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(text.matches("BEGIN:VEVENT").count(), 2);
    assert_eq!(text.matches("TRIGGER:-PT1H").count(), 2);
    assert!(!export().status.success());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
}

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn mcp_stdio_lists_and_calls_read_only_tools() {
    let mut server = Server(
        Command::new(BIN)
            .args(["mcp", "--demo"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    );
    let mut writer = server.0.stdin.take().unwrap();
    let reader = BufReader::new(server.0.stdout.take().unwrap());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in reader.lines() {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut send = |v: Value| {
        writeln!(writer, "{v}").unwrap();
        writer.flush().unwrap();
    };
    let receive = || -> Value {
        let line = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("no stdio MCP reply")
            .unwrap();
        serde_json::from_str(&line).unwrap_or_else(|_| panic!("non-JSON MCP stdout: {line}"))
    };
    send(
        json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}
        }}),
    );
    let init = receive();
    assert_eq!(init["id"], 1);
    assert!(
        init["result"]["capabilities"]["tools"].is_object(),
        "{init}"
    );
    send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    send(json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}));
    let tools = receive();
    let tools = tools["result"]["tools"].as_array().unwrap();
    let names = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(tools.len(), 17);
    assert_eq!(
        names,
        std::collections::BTreeSet::from([
            "login",
            "get_profile",
            "list_semesters",
            "list_courses",
            "list_homework",
            "get_homework_detail",
            "get_homework_deadlines",
            "list_exams",
            "list_notices",
            "list_notifications",
            "list_materials",
            "find_materials",
            "plan_material_download",
            "download_material",
            "list_chapters",
            "get_chapter_progress",
            "get_progress_overview",
        ])
    );
    let login = tools.iter().find(|t| t["name"] == "login").unwrap();
    assert_eq!(login["annotations"]["readOnlyHint"], false);
    assert!(
        tools
            .iter()
            .filter(|t| !matches!(t["name"].as_str(), Some("login" | "download_material")))
            .all(|t| t["annotations"]["readOnlyHint"] == true)
    );
    send(
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_homework_deadlines","arguments":{"days":7}}}),
    );
    let result = receive();
    assert_eq!(
        result["result"]["structuredContent"]["upcoming"][0]["id"], "hw-1",
        "{result}"
    );
    send(
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_materials","arguments":{"course":"数字电路","directory":"dir-1","keyword":"模板"}}}),
    );
    let result = receive();
    assert_eq!(
        result["result"]["structuredContent"]["items"][0]["attachments"][0]["name"],
        "逻辑门实验报告模板.pdf",
        "{result}"
    );
    send(
        json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"login","arguments":{"phone":"demo-account","password":"sensitive-fixture-only"}}}),
    );
    let result = receive();
    assert_eq!(result["result"]["isError"], true, "{result}");
    assert!(!result.to_string().contains("sensitive-fixture-only"));
    send(
        json!({"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"list_notifications","arguments":{"unread":true}}}),
    );
    let result = receive();
    assert_eq!(
        result["result"]["structuredContent"]["items"][0]["id"], "inbox-1",
        "{result}"
    );
    send(
        json!({"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"find_materials","arguments":{"course":"数字电路","keyword":"模板"}}}),
    );
    let result = receive();
    assert_eq!(
        result["result"]["structuredContent"]["items"][0]["id"], "mat-1",
        "{result}"
    );
    send(
        json!({"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"get_progress_overview","arguments":{}}}),
    );
    let result = receive();
    assert_eq!(
        result["result"]["structuredContent"]["items"]
            .as_array()
            .unwrap()
            .len(),
        2,
        "{result}"
    );
    send(
        json!({"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"download_material","arguments":{"course":"数字电路","data_id":"mat-1"}}}),
    );
    let result = receive();
    assert_eq!(result["result"]["isError"], true, "{result}");
}

#[test]
fn cli_rejects_removed_local_substitutes() {
    for command in [
        "schedule",
        "today",
        "next",
        "week",
        "schedule-ics",
        "school-exams",
        "pan",
    ] {
        let output = Command::new(BIN)
            .args(["--demo", command])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "removed command: {command}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("unrecognized subcommand"));
    }
}

#[test]
fn status_headless_watch_and_notification_search() {
    let run = |args: &[&str]| {
        Command::new(BIN)
            .args(["--demo", "--json"])
            .args(args)
            .output()
            .unwrap()
    };
    let feed = run(&["watch", "--once", "--all-events"]);
    assert!(
        feed.status.success(),
        "{}",
        String::from_utf8_lossy(&feed.stderr)
    );
    let feed: Value = serde_json::from_slice(&feed.stdout).unwrap();
    assert!(!feed["events"].as_array().unwrap().is_empty());
    let result = run(&["notifications", "--keyword", "组合逻辑", "--full"]);
    assert!(result.status.success());
    let result: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(result["items"][0]["content"], "范围： 组合逻辑 & 时序电路");
    let school = run(&["notifications", "--keyword", "考务地点", "--full"]);
    assert!(school.status.success());
    let school: Value = serde_json::from_slice(&school.stdout).unwrap();
    assert_eq!(school["items"][0]["content"], "请核对示例考务地点。");
    let status = run(&["status", "--probe"]);
    let status: Value = serde_json::from_slice(&status.stdout).unwrap();
    assert_eq!(status["probe"], json!({"ok":true,"network":false}));
}

#[test]
fn new_cli_features_work_offline_and_never_download_in_demo() {
    let run = |args: &[&str]| {
        Command::new(BIN)
            .args(["--demo", "--json"])
            .args(args)
            .output()
            .unwrap()
    };
    let json_result = |args: &[&str]| {
        let out = run(args);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<Value>(&out.stdout).unwrap()
    };
    assert_eq!(
        json_result(&["notifications", "--unread", "--keyword", "期中"])["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        json_result(&["progress", "数字电路", "--todo"])["items"][0]["state"],
        "in_progress"
    );
    assert_eq!(
        json_result(&["progress", "数字电路", "--depth", "0"])["limited"],
        true
    );
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("download-dir");
    let path = missing.to_str().unwrap();
    let plan = json_result(&[
        "download",
        "数字电路",
        "--dir",
        path,
        "--dry-run",
        "--keyword",
        "模板",
    ]);
    assert_eq!(plan["items"][0]["status"], "planned");
    assert!(!plan.to_string().contains("downloadv5"));
    assert!(
        !run(&["download", "数字电路", "--dir", path])
            .status
            .success()
    );
    assert!(!missing.exists());
}

#[test]
fn cli_does_not_try_network_with_incomplete_credentials() {
    let out = Command::new(BIN)
        .arg("login")
        .env("KETANGPAI_PHONE", "account-fixture")
        .env_remove("KETANGPAI_PASSWORD")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let error = String::from_utf8_lossy(&out.stderr);
    assert!(error.contains("环境变量"));
    assert!(!error.contains("account-fixture"));
}
