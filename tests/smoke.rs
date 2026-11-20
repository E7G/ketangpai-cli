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
    assert_eq!(tools.len(), 21);
    let login = tools.iter().find(|t| t["name"] == "login").unwrap();
    assert_eq!(login["annotations"]["readOnlyHint"], false);
    assert!(
        tools
            .iter()
            .filter(|t| !matches!(
                t["name"].as_str(),
                Some("login" | "download_material" | "export_schedule_ics")
            ))
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
        json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"list_school_exams","arguments":{}}}),
    );
    let result = receive();
    assert_eq!(
        result["result"]["structuredContent"]["items"][0]["kind"], "exam",
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
        json!({"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"list_pan_files","arguments":{"folder_id":"示例资料"}}}),
    );
    let result = receive();
    assert_eq!(
        result["result"]["structuredContent"]["items"][0]["name"], "实验笔记.pdf",
        "{result}"
    );
    send(
        json!({"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"download_material","arguments":{"course":"数字电路","data_id":"mat-1"}}}),
    );
    let result = receive();
    assert_eq!(result["result"]["isError"], true, "{result}");
}

#[test]
fn week_recurrence_grid_and_export_alarm() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("registrar.json");
    let out = dir.path().join("classes.ics");
    std::fs::write(&f,r#"{"source":"样本","semester_start":"2026-11-23","events":[{"kind":"class","title":"跨周实验","start":"2026-11-24 15:00","end":"2026-11-24 16:30","repeat_weeks":[1,3,3]}]}"#).unwrap();
    let read = |args: &[&str]| {
        Command::new(BIN)
            .args(args)
            .arg("--file")
            .arg(&f)
            .output()
            .unwrap()
    };
    let third = read(&["week", "3", "--json"]);
    assert!(
        third.status.success(),
        "{}",
        String::from_utf8_lossy(&third.stderr)
    );
    let third: Value = serde_json::from_slice(&third.stdout).unwrap();
    assert_eq!(third["items"].as_array().unwrap().len(), 1);
    assert_eq!(third["items"][0]["start"], "2026-12-08T15:00:00+08:00");
    let grid = read(&["week", "3", "--grid"]);
    assert!(String::from_utf8_lossy(&grid.stdout).contains("15:00 #01"));
    let exported = read(&["schedule-ics", "--output", out.to_str().unwrap()]);
    assert!(exported.status.success());
    let ics = std::fs::read_to_string(out).unwrap();
    assert_eq!(ics.matches("BEGIN:VEVENT").count(), 2);
    assert_eq!(ics.matches("TRIGGER:-PT30M").count(), 2);
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
fn imports_local_school_agenda_without_credentials_and_exports_ics() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("register.json");
    let output = tmp.path().join("schedule.ics");
    std::fs::write(&file, r#"{"source":"教务示例","events":[
        {"kind":"exam","title":"高数","start":"2026-11-21 10:30","end":"2026-11-21 12:30","location":"A101"},
        {"kind":"class","title":"跨日实验","start":"2026-11-20 23:00","end":"2026-11-21 01:00","location":"B202"}
    ]}"#).unwrap();
    let run = |name: &str| {
        Command::new(BIN)
            .args([
                "--json",
                name,
                "--from",
                "2026-11-21",
                "--to",
                "2026-11-21",
                "--file",
            ])
            .arg(&file)
            .env("KETANGPAI_TOKEN", "invalid-fixture")
            .output()
            .unwrap()
    };
    for (cmd, title) in [("school-exams", "高数"), ("schedule", "跨日实验")] {
        let out = run(cmd);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let data: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(data["items"].as_array().unwrap().len(), 1);
        assert_eq!(data["items"][0]["title"], title);
    }
    let out = Command::new(BIN)
        .args(["school-exams", "--file"])
        .arg(&file)
        .arg("-o")
        .arg(&output)
        .output()
        .unwrap();
    assert!(out.status.success());
    let ics = std::fs::read_to_string(&output).unwrap();
    assert!(ics.contains("DTSTART:20261121T023000Z\r\n"));
    assert!(ics.contains("LOCATION:A101\r\n"));
}

#[test]
fn school_csv_template_imports_with_week_filter() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("academic.csv");
    std::fs::write(&file, include_bytes!("../academic.example.csv")).unwrap();
    let out = Command::new(BIN)
        .args(["schedule", "--week", "2", "--file"])
        .arg(file)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let data: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(data["items"].as_array().unwrap().len(), 1);
    assert_eq!(data["items"][0]["start"], "2026-11-30T08:30:00+08:00");
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
