//! Local, date-specific records exported or transcribed from the student's registrar.
use crate::client::{Result, err};
use crate::service::shanghai;
use chrono::{DateTime, Datelike, Days, FixedOffset, NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    env,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Class,
    Exam,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    source: String,
    semester_start: Option<String>,
    #[serde(default)]
    events: Vec<InputEvent>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputEvent {
    kind: Kind,
    title: String,
    start: String,
    end: String,
    location: Option<String>,
    note: Option<String>,
    status: Option<String>,
    score: Option<String>,
    #[serde(default)]
    repeat_weeks: Vec<u16>,
}

#[derive(Serialize)]
pub struct Event {
    pub kind: Kind,
    pub title: String,
    pub start: DateTime<FixedOffset>,
    pub end: DateTime<FixedOffset>,
    pub location: Option<String>,
    pub note: Option<String>,
    pub duration_minutes: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<String>,
}

#[derive(Serialize)]
pub struct Agenda {
    pub source: String,
    pub semester_start: Option<NaiveDate>,
    pub items: Vec<Event>,
}

#[derive(Default)]
pub struct Filter {
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    pub keyword: Option<String>,
    pub week: Option<u16>,
}

pub fn week_bounds(anchor: NaiveDate, week: u16) -> Result<(NaiveDate, NaiveDate)> {
    if !(1..=60).contains(&week) {
        return Err(err("教学周必须在 1–60 之间。"));
    }
    let monday = anchor.checked_sub_days(Days::new(anchor.weekday().num_days_from_monday().into()));
    let start = monday
        .and_then(|d| d.checked_add_days(Days::new(u64::from(week - 1) * 7)))
        .ok_or_else(|| err("教学周日期超出范围。"))?;
    let end = start
        .checked_add_days(Days::new(6))
        .ok_or_else(|| err("教学周日期超出范围。"))?;
    Ok((start, end))
}

pub fn parse_date(raw: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(raw, "%Y-%m-%d")
        .map_err(|_| err("请使用 YYYY-MM-DD 日期，例如 2026-11-21。"))
}

fn datetime(raw: &str) -> Option<DateTime<FixedOffset>> {
    let tz = shanghai();
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|d| d.with_timezone(&tz))
        .or_else(|| {
            ["%Y-%m-%d %H:%M", "%Y-%m-%d %H:%M:%S"]
                .into_iter()
                .find_map(|f| {
                    let d = NaiveDateTime::parse_from_str(raw, f).ok()?;
                    tz.from_local_datetime(&d).single()
                })
        })
}

fn csv_input(bytes: &[u8]) -> Result<Input> {
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .from_reader(bytes);
    let headers = reader
        .headers()
        .map_err(|_| err("无法读取教务 CSV 表头；请使用 UTF-8。"))?
        .clone();
    let col = |names: &[&str]| {
        headers
            .iter()
            .position(|h| names.iter().any(|n| h.eq_ignore_ascii_case(n)))
    };
    let required =
        |names: &[&str]| col(names).ok_or_else(|| err(format!("教务 CSV 缺少 {} 列。", names[0])));
    let kind = required(&["kind", "类型"])?;
    let title = required(&["title", "名称", "课程名称", "考试名称"])?;
    let start = required(&["start", "开始时间"])?;
    let end = required(&["end", "结束时间"])?;
    let anchor = col(&["semester_start", "学期起始日"]);
    let location = col(&["location", "地点", "教室", "考场"]);
    let note = col(&["note", "备注"]);
    let status = col(&["status", "状态"]);
    let score = col(&["score", "成绩"]);
    let repeats = col(&["repeat_weeks", "重复周次"]);
    let mut input = Input {
        source: "本地教务 CSV".into(),
        semester_start: None,
        events: vec![],
    };
    for (i, row) in reader.records().enumerate() {
        let invalid = |reason: &str| err(format!("教务 CSV 第 {} 条数据：{reason}。", i + 1));
        let row = row.map_err(|_| invalid("列数或 UTF-8 格式不正确"))?;
        let value = |idx: Option<usize>| {
            idx.and_then(|n| row.get(n))
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let k = match row[kind].to_lowercase().as_str() {
            "class" | "课程" | "上课" => Kind::Class,
            "exam" | "考试" => Kind::Exam,
            _ => return Err(invalid("kind 必须是 class 或 exam")),
        };
        if let Some(date) = value(anchor) {
            if input.semester_start.as_ref().is_some_and(|d| d != &date) {
                return Err(invalid("学期起始日不一致"));
            }
            input.semester_start = Some(date);
        }
        let mut repeat_weeks = vec![];
        if let Some(weeks) = value(repeats) {
            let parse = |n: &str| {
                n.trim()
                    .parse::<u16>()
                    .ok()
                    .filter(|v| (1..=60).contains(v))
            };
            for segment in weeks.split(['|', ',', '、']) {
                let (a, b) = segment.split_once('-').unwrap_or((segment, segment));
                let (a, b) = (parse(a), parse(b));
                if let (Some(a), Some(b)) = (a, b)
                    && a <= b
                {
                    for n in a..=b {
                        if !repeat_weeks.contains(&n) {
                            repeat_weeks.push(n);
                        }
                    }
                } else {
                    return Err(invalid("重复周次应类似 1-8|10|12-16"));
                }
            }
        }
        input.events.push(InputEvent {
            kind: k,
            title: row[title].into(),
            start: row[start].into(),
            end: row[end].into(),
            location: value(location),
            note: value(note),
            status: value(status),
            score: value(score),
            repeat_weeks,
        });
    }
    Ok(input)
}

pub fn load(path: Option<&Path>, demo: bool, kind: Kind, filter: &Filter) -> Result<Agenda> {
    let (input, is_csv) = match (path, demo) {
        (None, true) => (include_bytes!("../academic.example.json").to_vec(), false),
        _ => {
            const MAX: u64 = 4 * 1024 * 1024;
            let path = path
                .map(Path::to_path_buf)
                .or_else(|| {
                    env::var_os("KETANGPAI_ACADEMIC_FILE")
                        .filter(|p| !p.is_empty())
                        .map(PathBuf::from)
                })
                .unwrap_or_else(|| PathBuf::from("academic.json"));
            let is_csv = path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("csv"));
            let file = File::open(path).map_err(|_| {
                err("无法打开教务日程文件。请用 --file 指定本地 JSON 或 UTF-8 CSV（包内附模板）。")
            })?;
            let mut bytes = vec![];
            file.take(MAX + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| err("无法读取教务日程文件。"))?;
            if bytes.len() as u64 > MAX {
                return Err(err("教务日程文件不能超过 4 MiB。"));
            }
            (bytes, is_csv)
        }
    };
    let bytes = input.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&input);
    let input: Input = if is_csv {
        csv_input(bytes)?
    } else {
        serde_json::from_slice(bytes).map_err(|e| {
            err(format!(
                "教务 JSON 格式不正确（第 {} 行），请对照模板。",
                e.line()
            ))
        })?
    };
    if input.source.trim().is_empty() {
        return Err(err("教务文件必须注明数据来源 source。"));
    }
    let semester_start = input
        .semester_start
        .as_deref()
        .map(parse_date)
        .transpose()?;
    let (from, to) = if let Some(week) = filter.week {
        if filter.from.is_some() || filter.to.is_some() {
            return Err(err("--week 不能和日期范围同时使用。"));
        }
        let anchor = semester_start
            .ok_or_else(|| err("使用教学周需要在教务 JSON 中填写 semester_start。"))?;
        let (a, b) = week_bounds(anchor, week)?;
        (Some(a), Some(b))
    } else {
        (filter.from, filter.to)
    };
    if matches!((from, to), (Some(a), Some(b)) if a > b) {
        return Err(err("起始日期不能晚于结束日期。"));
    }
    let to_exclusive = to
        .map(|d| {
            d.checked_add_days(Days::new(1))
                .ok_or_else(|| err("结束日期超出范围。"))
        })
        .transpose()?;
    let kw = filter.keyword.as_ref().map(|k| k.to_lowercase());
    let mut agenda = Agenda {
        source: input.source,
        semester_start,
        items: vec![],
    };
    for (i, raw) in input.events.into_iter().enumerate() {
        let invalid = || {
            err(format!(
                "第 {} 项教务安排缺少标题，或起止时间不正确。",
                i + 1
            ))
        };
        let start = datetime(&raw.start).ok_or_else(invalid)?;
        let end = datetime(&raw.end).ok_or_else(invalid)?;
        if raw.title.trim().is_empty() || end <= start {
            return Err(invalid());
        }
        if !raw.repeat_weeks.is_empty() && (raw.kind != Kind::Class || semester_start.is_none()) {
            return Err(err(format!(
                "第 {} 项安排：只有课表可以设置 repeat_weeks，且需填写 semester_start。",
                i + 1
            )));
        }
        let mut offsets = Vec::new();
        if let Some(anchor) = semester_start.filter(|_| !raw.repeat_weeks.is_empty()) {
            let (week_one, _) = week_bounds(anchor, 1)?;
            let start_day = start.date_naive();
            let weekday = u64::from(start_day.weekday().num_days_from_monday());
            if start_day
                != week_one
                    .checked_add_days(Days::new(weekday))
                    .ok_or_else(invalid)?
            {
                return Err(err(format!(
                    "第 {} 项重复课程的 start 必须写教学第一周相应星期的日期。",
                    i + 1
                )));
            }
            for week in raw.repeat_weeks {
                if !(1..=60).contains(&week) {
                    return Err(err(format!("第 {} 项 repeat_weeks 超出 1–60。", i + 1)));
                }
                let days = u64::from(week - 1) * 7;
                if !offsets.contains(&days) {
                    offsets.push(days);
                }
            }
        } else {
            offsets.push(0);
        }
        if raw.kind != kind
            || kw
                .as_ref()
                .is_some_and(|k| !raw.title.to_lowercase().contains(k))
        {
            continue;
        }
        for offset in offsets {
            let start = start
                .checked_add_days(Days::new(offset))
                .ok_or_else(invalid)?;
            let end = end
                .checked_add_days(Days::new(offset))
                .ok_or_else(invalid)?;
            if from.is_some_and(|f| {
                end.date_naive() < f
                    || (end.date_naive() == f && end.time() == chrono::NaiveTime::MIN)
            }) || to_exclusive.is_some_and(|t| start.date_naive() >= t)
            {
                continue;
            }
            if agenda.items.len() >= 20_000 {
                return Err(err("展开的教务安排超过 20,000 项，请缩短查询范围。"));
            }
            let status = if kind == Kind::Exam {
                raw.status.clone().or_else(|| {
                    let now = Utc::now();
                    Some(
                        if now < start {
                            "未开始"
                        } else if now >= end {
                            "已结束"
                        } else {
                            "进行中"
                        }
                        .into(),
                    )
                })
            } else {
                None
            };
            agenda.items.push(Event {
                kind,
                title: raw.title.clone(),
                start,
                end,
                location: raw.location.clone(),
                note: raw.note.clone(),
                duration_minutes: (end - start).num_minutes(),
                status,
                score: raw.score.clone(),
            });
        }
    }
    agenda
        .items
        .sort_by(|a, b| (a.start, &a.title).cmp(&(b.start, &b.title)));
    Ok(agenda)
}

pub fn export(agenda: &Agenda) -> String {
    use crate::calendar::{escape, fold};
    let mut out = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//ketangpai-cli//Registrar Agenda//ZH\r\nCALSCALE:GREGORIAN\r\n".to_owned();
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    for item in &agenda.items {
        let start = item.start.to_utc().format("%Y%m%dT%H%M%SZ");
        let end = item.end.to_utc().format("%Y%m%dT%H%M%SZ");
        let uid = Sha256::digest(format!(
            "{:?}\0{}\0{start}\0{end}\0{}",
            item.kind,
            item.title,
            item.location.as_deref().unwrap_or_default()
        ));
        for line in [
            "BEGIN:VEVENT".into(),
            format!("UID:{uid:x}@ktp-academic"),
            format!("DTSTAMP:{stamp}"),
            format!("DTSTART:{start}"),
            format!("DTEND:{end}"),
            format!("SUMMARY:{}", escape(&item.title)),
        ] {
            fold(&line, &mut out);
        }
        if let Some(location) = &item.location {
            fold(&format!("LOCATION:{}", escape(location)), &mut out);
        }
        if let Some(note) = &item.note {
            fold(&format!("DESCRIPTION:{}", escape(note)), &mut out);
        }
        if item.kind == Kind::Class {
            out.push_str("BEGIN:VALARM\r\nTRIGGER:-PT30M\r\nACTION:DISPLAY\r\n");
            fold(&format!("DESCRIPTION:{}", escape(&item.title)), &mut out);
            out.push_str("END:VALARM\r\n");
        }
        out.push_str("END:VEVENT\r\n");
    }
    out.push_str("END:VCALENDAR\r\n");
    out
}
