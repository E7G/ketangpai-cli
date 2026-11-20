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
    assert_eq!(tools.len(), 10);
    assert!(
        tools
            .iter()
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
}
