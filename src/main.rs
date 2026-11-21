mod auth;
mod calendar;
mod client;
mod demo;
mod download;
mod server;
mod service;
mod student;

use chrono::{DateTime, FixedOffset};
use clap::{Parser, Subcommand};
use client::{Client, Result, err, save_token, verify_token};
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
    /// 用课堂派账号密码登录并保存会话。
    Login {
        /// 改为手动导入网页 Token。
        #[arg(long)]
        token: bool,
        /// 只验证，不保存 Token。
        #[arg(long)]
        no_save: bool,
        #[arg(long = "save", hide = true, conflicts_with = "no_save")]
        legacy_save: bool,
    },
    /// 查询当前账号基础资料。
    Whoami,
    /// 查看本机登录配置；不联网、不显示凭据。
    Status {
        /// 额外查询课堂派，验证登录是否可用。
        #[arg(long)]
        probe: bool,
    },
    /// 查看可选学年学期。
    Semesters,
    /// 查看课程列表。
    Courses {
        /// 列出全部可选学期的课程。
        #[arg(long, conflicts_with_all = ["semester", "term"])]
        all: bool,
    },
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
        /// 递归搜索资料名称或附件名。
        #[arg(long, conflicts_with = "keyword")]
        find: Option<String>,
        #[arg(long, default_value_t = 3)]
        depth: u8,
    },
    /// 列出课程章节或指定目录下的内容。
    Chapters {
        course: String,
        #[arg(long, default_value = "0")]
        directory: String,
        #[arg(long)]
        tree: bool,
        #[arg(long)]
        todo: bool,
        #[arg(long, default_value_t = 3)]
        depth: u8,
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
        /// 运行一轮后退出，可搭配 --json 使用。
        #[arg(long)]
        once: bool,
        /// 同时查询通知、作业状态、课程资料和测验动态。
        #[arg(long)]
        all_events: bool,
    },
    /// 查看课堂派通知中心（不会标记已读）。
    Notifications {
        #[arg(long)]
        unread: bool,
        #[arg(long, short)]
        keyword: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// 展开通知正文（纯文本）。
        #[arg(long)]
        full: bool,
    },
    /// 查看课程章节中的学习活动完成情况。
    Progress {
        course: Option<String>,
        #[arg(long, default_value = "0")]
        directory: String,
        #[arg(long, default_value_t = 3)]
        depth: u8,
        #[arg(long)]
        todo: bool,
        #[arg(long, default_value_t = 20)]
        max_courses: usize,
    },
    /// 批量下载课堂派明确允许下载的课程附件。
    Download {
        course: String,
        #[arg(long, default_value = "0")]
        directory: String,
        #[arg(long, default_value_t = 2)]
        depth: u8,
        #[arg(short, long, visible_alias = "find")]
        keyword: Option<String>,
        #[arg(long, visible_alias = "id")]
        material: Option<String>,
        #[arg(long, default_value_t = 20)]
        max_files: usize,
        #[arg(long, default_value = "downloads")]
        dir: PathBuf,
        #[arg(long)]
        dry_run: bool,
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

fn print_progress(data: &student::Progress, tree: bool, json: bool) -> Result<u8> {
    if json {
        dump(data)?;
    } else {
        let summary = |c: &student::Counts| {
            format!(
                "已完成 {} / 未完成 {} / 进行中 {} / 未知 {}",
                c.complete, c.pending, c.in_progress, c.unknown
            )
        };
        println!("{} · {}", terminal(&data.course), summary(&data.counts));
        if tree {
            for (chapter, counts) in &data.chapters {
                println!("  {} · {}", terminal(chapter), summary(counts));
                for i in data.items.iter().filter(|x| x.chapter == *chapter) {
                    println!(
                        "    {} · {} · {}",
                        terminal(&i.title),
                        i.kind,
                        i.state.label()
                    );
                }
            }
        } else {
            for i in &data.items {
                println!(
                    "  {} / {} · {} · {}",
                    terminal(&i.chapter),
                    terminal(&i.title),
                    i.kind,
                    i.state.label()
                );
            }
        }
        if data.limited {
            println!("达到目录扫描限制，或有更深目录；结果不完整。");
        }
    }
    Ok(0)
}

async fn status(probe: bool, demo: bool, json: bool) -> Result<u8> {
    let mut data = if demo {
        serde_json::json!({"mode":"demo","environment_token":false,"cached_session":false,"password_configured":false})
    } else {
        let environment_token =
            std::env::var("KETANGPAI_TOKEN").is_ok_and(|s| !s.trim().is_empty());
        let cached_session = client::cached_token();
        let password = auth::resolve(auth::CredentialsInput::default());
        serde_json::json!({ "mode":"live", "environment_token":environment_token,
            "cached_session":cached_session.as_ref().is_ok_and(Option::is_some),
            "cache_error":cached_session.err().map(|e| e.to_string()),
            "password_configured":password.as_ref().is_ok_and(Option::is_some),
            "config_error":password.err().map(|e| e.to_string()) })
    };
    let mut exit = 0;
    if probe {
        let checked = async {
            Service(if demo {
                Client::Demo
            } else {
                Client::configured().await?
            })
            .profile()
            .await
        }
        .await;
        match checked {
            Ok(_) => data["probe"] = serde_json::json!({"ok":true,"network":!demo}),
            Err(e) => {
                data["probe"] =
                    serde_json::json!({"ok":false,"network":!demo,"error":e.to_string()});
                exit = 1;
            }
        }
    }
    if json {
        dump(&data)?;
    } else {
        for (key, label) in [
            ("environment_token", "环境变量 Token"),
            ("cached_session", "本机会话"),
            ("password_configured", "账号密码配置"),
        ] {
            println!("{label}：{}", if data[key] == true { "有" } else { "无" });
        }
        for key in ["cache_error", "config_error"] {
            if let Some(e) = data[key].as_str() {
                eprintln!("{}", terminal(e));
            }
        }
        if probe {
            println!(
                "会话探测：{}",
                if exit == 0 {
                    "通过".into()
                } else {
                    terminal(data["probe"]["error"].as_str().unwrap_or("失败"))
                }
            );
        }
    }
    Ok(exit)
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
    if let Some(seconds) = item.duration_seconds {
        meta.push(format!("限时 {} 分 {} 秒", seconds / 60, seconds % 60));
    }
    if let Some(n) = item.question_count {
        meta.push(format!("题量 {n}"));
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
    if let Command::Status { probe } = &cli.command {
        return status(*probe, cli.demo, cli.json).await;
    }
    let scope = Scope {
        semester: cli.semester,
        term: cli.term,
    };
    if let Command::Login {
        token,
        no_save,
        legacy_save: _,
    } = cli.command
    {
        if cli.demo {
            return Err(err("演示模式不需要登录。"));
        }
        let save = !no_save;
        let (profile, method) = if token {
            if !std::io::stdin().is_terminal() {
                return Err(err(
                    "请在交互式终端手动导入 Token；非交互可设置 KETANGPAI_TOKEN。",
                ));
            }
            let token = zeroize::Zeroizing::new(
                rpassword::prompt_password("粘贴本人课堂派网页 Token（输入不可见）：")
                    .map_err(|_| err("读取 Token 失败。"))?,
            );
            let (_, profile) = verify_token(token.trim()).await?;
            if save {
                save_token(token.trim())?;
            }
            (profile, "token")
        } else {
            let credentials = match auth::resolve(auth::CredentialsInput::default())? {
                Some(credentials) => credentials,
                None if std::io::stdin().is_terminal() => auth::Credentials::prompt()?,
                None => return Err(auth::missing_credentials()),
            };
            let (_, profile) = auth::login(credentials, save).await?;
            (profile, "password")
        };
        if cli.json {
            dump(
                &serde_json::json!({"verified":true,"method":method,"saved":save,"profile":profile}),
            )?;
        } else {
            println!(
                "{}",
                if save {
                    "登录成功，会话已保存在本机。"
                } else {
                    "登录验证通过，未保存会话。"
                }
            );
        }
        return Ok(0);
    }
    let mut api = Service(if cli.demo {
        Client::Demo
    } else {
        Client::configured().await?
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
        Command::Courses { all } => {
            if all {
                let data = api.all_courses().await?;
                if cli.json {
                    dump(&data)?;
                } else {
                    for entry in &data.items {
                        println!(
                            "  {}/{} · {} [{}]",
                            terminal(&entry.semester.semester),
                            terminal(&entry.semester.term),
                            terminal(&entry.course.title),
                            terminal(&entry.course.id)
                        );
                    }
                    if data.items.is_empty() {
                        println!("没有读到课程。");
                    }
                    if data.limited {
                        println!("已达学期扫描上限。");
                    }
                    for e in &data.errors {
                        eprintln!("{}", terminal(e));
                    }
                }
                return Ok(if data.errors.is_empty() { 0 } else { 3 });
            }
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
            find,
            depth,
        } => {
            if let Some(keyword) = find {
                let matches =
                    student::find_materials(&mut api, &scope, &course, &directory, depth, &keyword)
                        .await?;
                if cli.json {
                    dump(&matches)?;
                } else {
                    println!("匹配资料（{} 项）", matches.items.len());
                    for (idx, m) in matches.items.iter().enumerate() {
                        row(&m.material, idx + 1, false);
                        println!(
                            "      {} · {}",
                            if m.chapter.is_empty() {
                                "当前目录".into()
                            } else {
                                terminal(&m.chapter.join(" / "))
                            },
                            m.material.url
                        );
                    }
                    if matches.limited {
                        println!("达到扫描限制，搜索结果可能不完整。");
                    }
                }
                return Ok(0);
            }
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
        Command::Chapters {
            course,
            directory,
            tree,
            todo,
            depth,
        } => {
            if tree || todo {
                let data =
                    student::progress(&mut api, &scope, &course, &directory, depth, todo).await?;
                return print_progress(&data, true, cli.json);
            }
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
        Command::Watch {
            course,
            interval,
            once,
            all_events,
        } => {
            if once && !all_events {
                return board(
                    &api.deadlines(&scope, course.as_deref(), Some(7)).await?,
                    cli.json,
                );
            }
            if all_events {
                return watch_events(
                    &mut api,
                    &scope,
                    course.as_deref(),
                    interval,
                    once,
                    cli.json,
                )
                .await;
            }
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
                    Err(e) if e.is_auth() => return Err(e),
                    Err(e) => eprintln!("本轮查询失败：{e}"),
                }
                tokio::time::sleep(std::time::Duration::from_secs(u64::from(interval) * 60)).await;
            }
        }
        Command::Notifications {
            unread,
            keyword,
            limit,
            full,
        } => {
            let data =
                student::notifications(&mut api, unread, keyword.as_deref(), limit, full).await?;
            if cli.json {
                dump(&data)?;
            } else {
                println!("课堂派通知中心（{} 条）", data.items.len());
                for n in &data.items {
                    println!(
                        "  {} · {} · {} · {}{}",
                        date(n.published_at),
                        match n.unread {
                            Some(true) => "未读",
                            Some(false) => "已读",
                            None => "状态未知",
                        },
                        n.category,
                        terminal(&n.title),
                        n.course
                            .as_deref()
                            .map(|c| format!(" · {}", terminal(c)))
                            .unwrap_or_default()
                    );
                    if let Some(body) = &n.content {
                        println!("    {}", terminal(body));
                    }
                }
                if data.has_more {
                    println!("  当前输出不是完整通知列表。");
                }
                if data.body_incomplete {
                    println!("  部分学校通知正文未查到。");
                }
                println!("  网页：{}", data.url);
            }
        }
        Command::Progress {
            course,
            directory,
            depth,
            todo,
            max_courses,
        } => {
            if let Some(course) = course {
                let data =
                    student::progress(&mut api, &scope, &course, &directory, depth, todo).await?;
                return print_progress(&data, false, cli.json);
            }
            if directory != "0" {
                return Err(err("使用 --directory 需要指定课程。"));
            }
            let mut data = student::progress_overview(&mut api, &scope, depth, max_courses).await?;
            if todo {
                data.items
                    .retain(|p| p.counts.pending + p.counts.in_progress + p.counts.unknown > 0);
            }
            if cli.json {
                dump(&data)?;
            } else {
                println!("课程学习进度（{} 门课）", data.items.len());
                for p in &data.items {
                    println!(
                        "  {} · 已完成 {} / 未完成 {} / 进行中 {} / 未知 {}{}",
                        terminal(&p.course),
                        p.counts.complete,
                        p.counts.pending,
                        p.counts.in_progress,
                        p.counts.unknown,
                        if p.limited {
                            " · 部分目录未扫描"
                        } else {
                            ""
                        }
                    );
                }
                if data.limited {
                    println!("达到课程扫描上限；还有课程未查询。");
                }
                issue_list(&data.errors);
            }
            return Ok(if data.errors.is_empty() { 0 } else { 3 });
        }
        Command::Download {
            course,
            directory,
            depth,
            keyword,
            material,
            max_files,
            dir,
            dry_run,
        } => {
            let report = download::batch(
                &mut api,
                &scope,
                download::Options {
                    course: &course,
                    directory: &directory,
                    depth,
                    keyword: keyword.as_deref(),
                    material: material.as_deref(),
                    max_files,
                    output: &dir,
                    dry_run,
                },
            )
            .await?;
            if cli.json {
                dump(&report)?;
            } else {
                if report.items.is_empty() {
                    println!("没有匹配的附件。");
                }
                for i in &report.items {
                    println!(
                        "  {} · {}{}",
                        i.status,
                        terminal(i.name.as_deref().unwrap_or(&i.material)),
                        i.reason
                            .as_deref()
                            .map(|r| format!(" · {}", terminal(r)))
                            .unwrap_or_default()
                    );
                }
                if report.limited {
                    println!("  达到扫描或下载数量限制；结果可能不完整。");
                }
            }
            return Ok(if report.items.iter().any(|i| i.status == "failed") {
                3
            } else {
                0
            });
        }
        Command::Mcp | Command::Login { .. } | Command::Status { .. } => {
            unreachable!()
        }
    }
    Ok(0)
}

async fn watch_events(
    api: &mut Service,
    scope: &Scope,
    course: Option<&str>,
    interval: u16,
    once: bool,
    json: bool,
) -> Result<u8> {
    use std::collections::BTreeMap;
    if json && !once {
        return Err(err("watch 的 JSON 输出需要 --once。"));
    }
    let mut previous = BTreeMap::<String, String>::new();
    let mut previous_deadlines = String::new();
    let mut initialized = false;
    loop {
        let deadlines = api.deadlines(scope, course, Some(7)).await;
        let mut events = BTreeMap::<String, String>::new();
        let mut errors: Vec<String> = vec![];
        for (kind, label) in [
            (Kind::Homework, "作业"),
            (Kind::Material, "资料"),
            (Kind::Test, "测验"),
        ] {
            let result = if kind == Kind::Material && course.is_none() {
                // The general material command requires a single course; watch scans each
                // course's top level only and records individual failures.
                let courses = api.courses(scope).await?;
                let mut listing = Listing {
                    semester: courses.semester,
                    items: vec![],
                    errors: vec![],
                };
                for c in courses.items {
                    match api.content(&c, Kind::Material, "0").await.and_then(|rs| {
                        rs.iter()
                            .filter(|r| !service::folder(r))
                            .map(|r| service::normalize(r, Kind::Material, &c))
                            .collect::<Result<Vec<Item>>>()
                    }) {
                        Ok(items) => listing.items.extend(items),
                        Err(e) if e.is_auth() => return Err(e),
                        Err(e) => listing.errors.push(Issue {
                            course_id: c.id,
                            course_title: c.title,
                            error: e.to_string(),
                        }),
                    }
                }
                Ok(listing)
            } else {
                api.list(scope, kind, course, None, "0").await
            };
            match result {
                Ok(listing) => {
                    for i in listing.items.into_iter().filter(|i| !i.is_folder) {
                        let key = format!("{label}:{}:{}", i.course_id, i.id);
                        let line = format!(
                            "{label} · {} · {}{}{}{}",
                            i.course_title,
                            i.title,
                            i.status.map(|s| format!(" · {s}")).unwrap_or_default(),
                            i.due_at
                                .map(|d| format!(" · 截止 {}", date(Some(d))))
                                .unwrap_or_default(),
                            i.grade.map(|g| format!(" · 得分 {g}")).unwrap_or_default()
                        );
                        events.insert(key, line);
                    }
                    for i in listing.errors {
                        errors.push(format!("{label} · {}：{}", i.course_title, i.error));
                    }
                }
                Err(e) if e.is_auth() => return Err(e),
                Err(e) => errors.push(format!("{label}：{e}")),
            }
        }
        match student::notifications(api, false, None, 20, false).await {
            Ok(inbox) => {
                for i in inbox.items {
                    events.insert(
                        format!("通知:{}", i.id),
                        format!("通知 · {} · {}", i.category, i.title),
                    );
                }
            }
            Err(e) if e.is_auth() => return Err(e),
            Err(e) => errors.push(format!("通知：{e}")),
        }
        let deadlines = match deadlines {
            Ok(board) => {
                for i in &board.errors {
                    errors.push(format!("截止看板 · {}：{}", i.course_title, i.error));
                }
                Some(board)
            }
            Err(e) if e.is_auth() => return Err(e),
            Err(e) => {
                errors.push(format!("截止看板：{e}"));
                None
            }
        };
        if once {
            if json {
                dump(
                    &serde_json::json!({"events":events.values().collect::<Vec<_>>(),"deadlines":deadlines,"errors":errors}),
                )?;
            } else {
                for line in events.values() {
                    println!("{}", terminal(line));
                }
                if let Some(data) = deadlines {
                    board(&data, false)?;
                }
                for error in &errors {
                    eprintln!("{}", terminal(error));
                }
            }
            return Ok(if errors.is_empty() { 0 } else { 3 });
        }
        if !initialized {
            println!(
                "动态巡检基线：{} 条。后续报告新增或变化；Ctrl+C 退出。",
                events.len()
            );
            initialized = true;
        } else {
            for (key, line) in &events {
                if previous.get(key) != Some(line) {
                    println!("更新 · {}", terminal(line));
                }
            }
        }
        if let Some(current) = &deadlines {
            let snapshot =
                serde_json::to_string(current).map_err(|_| err("截止结果序列化失败。"))?;
            if snapshot != previous_deadlines {
                board(current, false)?;
                previous_deadlines = snapshot;
            }
        }
        // Preserve old items when an endpoint fails so recovery does not create false alerts.
        if errors.is_empty() {
            previous = events;
        } else {
            previous.extend(events);
        }
        for error in &errors {
            eprintln!("{}", terminal(error));
        }
        tokio::time::sleep(std::time::Duration::from_secs(u64::from(interval) * 60)).await;
    }
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
