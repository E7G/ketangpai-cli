mod calendar;
mod client;
mod demo;
mod server;
mod service;

use chrono::{DateTime, FixedOffset};
use clap::{Parser, Subcommand};
use client::{Client, Error, Result, err, read_token, save_token};
use serde::Serialize;
use service::{Board, Issue, Item, Kind, Listing, Scope, Service};
use std::{
    io::{IsTerminal, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

#[derive(Parser)]
#[command(name = "ktp", version, about = "课堂派学生端只读 CLI / MCP")]
struct Cli {
    /// 离线演示，不使用 Token，也不联网。
    #[arg(long, global = true)]
    demo: bool,
    /// 以 JSON 输出。
    #[arg(long, global = true)]
    json: bool,
    /// 学年，例如 2026-2027；需与 --term 同时指定。
    #[arg(long, global = true, requires = "term")]
    semester: Option<String>,
    /// 学期编号，例如 1；需与 --semester 同时指定。
    #[arg(long, global = true, requires = "semester")]
    term: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 录入并验证网页 Token；默认只验证。
    Login {
        /// 验证通过后保存到本机。
        #[arg(long)]
        save: bool,
    },
    /// 查询当前账号基础资料。
    Whoami,
    /// 查看可选学年学期。
    Semesters,
    /// 查看课程列表。
    Courses,
    /// 待交作业截止看板。
    Hw {
        #[arg(long)]
        course: Option<String>,
        #[arg(long, value_parser = clap::value_parser!(u16).range(1..=365))]
        days: Option<u16>,
    },
    /// 查看某门课的全部作业。
    HwLs { course: String },
    /// 按序号或 ID 查询作业详情。
    HwInfo { course: String, homework: String },
    /// 查看课程内测试（不是教务处统一考试）。
    Exam {
        #[arg(long)]
        course: Option<String>,
    },
    /// 查看课程公告，不标记已读。
    Notice {
        #[arg(long)]
        course: Option<String>,
        #[arg(long, short)]
        keyword: Option<String>,
    },
    /// 查看资料与网页入口；可按当前目录的附件名过滤。
    Materials {
        course: String,
        #[arg(long, default_value = "0")]
        directory: String,
        #[arg(long, short)]
        keyword: Option<String>,
    },
    /// 列出课程章节或指定目录下的内容。
    Chapters {
        course: String,
        #[arg(long, default_value = "0")]
        directory: String,
    },
    /// 导出未交作业的截止时间和提前 1 小时提醒。
    Ics {
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        course: Option<String>,
        #[arg(long, value_parser = clap::value_parser!(u16).range(1..=365))]
        days: Option<u16>,
    },
    /// 在终端定时查询截止看板；Ctrl+C 退出。
    Watch {
        #[arg(long)]
        course: Option<String>,
        #[arg(long, default_value_t = 30, value_parser = clap::value_parser!(u16).range(5..=1440))]
        interval: u16,
    },
    /// 启动 MCP stdio 服务。
    Mcp,
}

fn terminal(s: &str) -> String {
    s.chars()
        .map(|ch| {
            if ch.is_control() || matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') {
                ' '
            } else {
                ch
            }
        })
        .collect()
}

fn date(d: Option<DateTime<FixedOffset>>) -> String {
    d.map(|v| v.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "—".into())
}

fn dump<T: Serialize>(value: &T) -> Result<()> {
    serde_json::to_writer_pretty(std::io::stdout().lock(), value)
        .map_err(|_| err("无法输出 JSON。"))?;
    println!();
    Ok(())
}

fn issue_list(errors: &[Issue]) {
    if !errors.is_empty() {
        eprintln!("部分课程未能查询到（{} 项）：", errors.len());
        for i in errors {
            eprintln!(
                "  {}（{}）：{}",
                terminal(&i.course_title),
                terminal(&i.course_id),
                terminal(&i.error)
            );
        }
    }
}

fn row(item: &Item, index: usize, show_course: bool) {
    let mut meta = Vec::new();
    if show_course {
        meta.push(terminal(&item.course_title));
    }
    if item.is_folder {
        meta.push("目录".into());
    }
    if let Some(status) = item.status {
        meta.push(status.into());
    }
    if item.opens_at.is_some() {
        meta.push(format!("开始 {}", date(item.opens_at)));
    }
    if item.due_at.is_some() {
        meta.push(format!("截止 {}", date(item.due_at)));
    }
    if item.published_at.is_some() {
        meta.push(format!("发布 {}", date(item.published_at)));
    }
    if let Some(grade) = &item.grade {
        meta.push(format!("得分 {}", terminal(grade)));
    }
    if let Some(max) = &item.max_grade {
        meta.push(format!("满分 {}", terminal(max)));
    }
    let extra = if meta.is_empty() {
        String::new()
    } else {
        format!(" · {}", meta.join(" · "))
    };
    println!(
        "  {index:>2}. {} [{}]{}",
        terminal(&item.title),
        terminal(&item.id),
        extra
    );
    for a in &item.attachments {
        println!("      附件：{}", terminal(&a.name));
    }
}

fn list(label: &str, data: &Listing<Item>, json: bool, show_course: bool) -> Result<u8> {
    if json {
        dump(data)?;
    } else {
        if data.items.is_empty() && !data.errors.is_empty() {
            println!("{label}：没有查到记录，请先检查下方失败的课程。");
        } else {
            println!("{label}（{} 条）", data.items.len());
        }
        for (i, item) in data.items.iter().enumerate() {
            row(item, i + 1, show_course);
        }
        issue_list(&data.errors);
    }
    Ok(if data.errors.is_empty() { 0 } else { 3 })
}

fn board(data: &Board, json: bool) -> Result<u8> {
    if json {
        dump(data)?;
    } else {
        println!("作业截止看板（北京时间）");
        for (name, items) in [
            ("即将截止", &data.upcoming),
            ("已过期", &data.overdue),
            ("无截止时间", &data.undated),
            ("提交状态未知，请到网页核对", &data.unknown),
        ] {
            if !items.is_empty() {
                println!("\n{name}（{}）", items.len());
                for (i, item) in items.iter().enumerate() {
                    row(item, i + 1, true);
                }
            }
        }
        if data.upcoming.is_empty()
            && data.overdue.is_empty()
            && data.undated.is_empty()
            && data.unknown.is_empty()
        {
            if data.errors.is_empty() {
                println!("没有待办作业。");
            } else {
                println!("没有查到待办；结果不完整。");
            }
        }
        issue_list(&data.errors);
    }
    Ok(if data.errors.is_empty() { 0 } else { 3 })
}

fn export_file(path: &Path, content: &str) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| err("无法创建日历文件，请确认输出目录已经存在。"))?;
    file.write_all(content.as_bytes())
        .map_err(|_| err("写入日历文件失败。"))?;
    file.persist_noclobber(path).map_err(|e| {
        if e.error.kind() == std::io::ErrorKind::AlreadyExists {
            err("输出文件已存在；未覆盖。请另选文件名。")
        } else {
            err("无法保存日历文件。")
        }
    })?;
    Ok(())
}

async fn run(cli: Cli) -> Result<u8> {
    if matches!(cli.command, Command::Mcp) {
        server::run(cli.demo).await?;
        return Ok(0);
    }
    if cli.demo && !cli.json {
        eprintln!("离线演示 · 以下均为示例数据");
    }
    let scope = Scope {
        semester: cli.semester,
        term: cli.term,
    };
    if let Command::Login { save } = cli.command {
        if cli.demo {
            return Err(err("演示模式不需要登录。"));
        }
        if !std::io::stdin().is_terminal() {
            return Err(err(
                "请在交互式终端运行 ktp login，非交互使用 KETANGPAI_TOKEN。",
            ));
        }
        let token = rpassword::prompt_password("粘贴本人课堂派网页 Token（输入不可见）：")
            .map_err(|_| err("读取 Token 失败。"))?;
        let token = token.trim();
        let profile = Service(Client::live(token)?).profile().await?;
        if save {
            save_token(token)?;
        }
        if cli.json {
            dump(&serde_json::json!({"verified":true, "saved":save, "profile":profile}))?;
        } else {
            println!(
                "{}",
                if save {
                    "验证通过，Token 已保存在本机。"
                } else {
                    "验证通过，未保存 Token。"
                }
            );
        }
        return Ok(0);
    }
    let mut api = Service(if cli.demo {
        Client::Demo
    } else {
        Client::live(&read_token()?)?
    });
    match cli.command {
        Command::Whoami => {
            let p = api.profile().await?;
            if cli.json {
                dump(&p)?;
            } else {
                for (key, label) in [("name", "姓名"), ("id", "用户 ID"), ("school", "学校")]
                {
                    println!("{label}：{}", terminal(p[key].as_str().unwrap_or("—")));
                }
            }
        }
        Command::Semesters => {
            let terms = api.semesters().await?;
            if cli.json {
                dump(&terms)?;
            } else if terms.is_empty() {
                println!("没有可选学期。");
            } else {
                for t in terms {
                    println!("{} · 学期 {}", terminal(&t.semester), terminal(&t.term));
                }
            }
        }
        Command::Courses => {
            let data = api.courses(&scope).await?;
            if cli.json {
                dump(&data)?;
            } else {
                if let Some(s) = &data.semester {
                    println!("{} · 学期 {}", terminal(&s.semester), terminal(&s.term));
                }
                if data.items.is_empty() {
                    println!("没有课程。");
                }
                for c in &data.items {
                    let fields = [&c.class_name, &c.teacher]
                        .into_iter()
                        .filter_map(|v| v.as_deref())
                        .map(terminal)
                        .collect::<Vec<_>>()
                        .join(" · ");
                    println!(
                        "  {} [{}]{}",
                        terminal(&c.title),
                        terminal(&c.id),
                        if fields.is_empty() {
                            String::new()
                        } else {
                            format!(" · {fields}")
                        }
                    );
                }
            }
        }
        Command::Hw { course, days } => {
            return board(
                &api.deadlines(&scope, course.as_deref(), days).await?,
                cli.json,
            );
        }
        Command::HwLs { course } => {
            return list(
                "作业",
                &api.list(&scope, Kind::Homework, Some(&course), None, "0")
                    .await?,
                cli.json,
                false,
            );
        }
        Command::HwInfo { course, homework } => {
            let item = api.homework_detail(&scope, &course, &homework).await?;
            if cli.json {
                dump(&item)?;
            } else {
                row(&item, 1, true);
                println!("  网页：{}", item.url);
            }
        }
        Command::Exam { course } => {
            return list(
                "课程测试",
                &api.list(&scope, Kind::Test, course.as_deref(), None, "0")
                    .await?,
                cli.json,
                true,
            );
        }
        Command::Notice { course, keyword } => {
            return list(
                "课程公告",
                &api.list(
                    &scope,
                    Kind::Notice,
                    course.as_deref(),
                    keyword.as_deref(),
                    "0",
                )
                .await?,
                cli.json,
                true,
            );
        }
        Command::Materials {
            course,
            directory,
            keyword,
        } => {
            return list(
                "课程资料",
                &api.list(
                    &scope,
                    Kind::Material,
                    Some(&course),
                    keyword.as_deref(),
                    &directory,
                )
                .await?,
                cli.json,
                false,
            );
        }
        Command::Chapters { course, directory } => {
            return list(
                "章节内容",
                &api.list(&scope, Kind::Chapter, Some(&course), None, &directory)
                    .await?,
                cli.json,
                false,
            );
        }
        Command::Ics {
            output,
            course,
            days,
        } => {
            let data = api.deadlines(&scope, course.as_deref(), days).await?;
            if !data.errors.is_empty() {
                issue_list(&data.errors);
                return Err(err("截止数据不完整，未生成日历。"));
            }
            if data.upcoming.is_empty() {
                return Err(err("没有可导出的待交作业截止时间。"));
            }
            export_file(&output, &calendar::export(&data.upcoming))?;
            if cli.json {
                dump(&serde_json::json!({"output":output,"count":data.upcoming.len()}))?;
            } else {
                println!(
                    "已导出 {} 项作业截止时间到 {}。",
                    data.upcoming.len(),
                    terminal(&output.to_string_lossy())
                );
            }
        }
        Command::Watch { course, interval } => {
            if cli.json {
                return Err(err("watch 不支持 --json，请用 hw --json。"));
            }
            let mut last = String::new();
            loop {
                match api.deadlines(&scope, course.as_deref(), Some(7)).await {
                    Ok(data) => {
                        let current = serde_json::to_string(&data)
                            .map_err(|_| err("截止结果序列化失败。"))?;
                        if current != last {
                            println!(
                                "\n{} · 截止看板更新",
                                chrono::Utc::now()
                                    .with_timezone(&service::shanghai())
                                    .format("%Y-%m-%d %H:%M")
                            );
                            board(&data, false)?;
                            last = current;
                        }
                    }
                    Err(e @ Error::Auth(_)) => return Err(e),
                    Err(e) => eprintln!("本轮查询失败：{e}"),
                }
                tokio::time::sleep(std::time::Duration::from_secs(u64::from(interval) * 60)).await;
            }
        }
        Command::Mcp | Command::Login { .. } => unreachable!(),
    }
    Ok(0)
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("{}", terminal(&e.to_string()));
            ExitCode::FAILURE
        }
    }
}
