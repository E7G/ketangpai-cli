use crate::client::{Endpoint, Result, WEB, err};
use crate::service::{
    Course, Issue, Item, Kind, Scope, Semester, Service, choose, first, folder, id, integer,
    normalize, rows, text, time,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{HashSet, VecDeque};

#[derive(Debug, Serialize)]
pub struct Notification {
    pub id: String,
    pub title: String,
    pub category: &'static str,
    pub course: Option<String>,
    pub unread: Option<bool>,
    pub published_at: Option<chrono::DateTime<chrono::FixedOffset>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
}

// Notification HTML is untrusted; expose the visible text, never markup or attributes.
fn notice_text(input: &str) -> String {
    let mut text = String::new();
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '<'
            && chars
                .peek()
                .is_some_and(|c| c.is_ascii_alphabetic() || matches!(c, '/' | '!' | '?'))
        {
            let mut quote = None;
            for x in chars.by_ref() {
                match (quote, x) {
                    (Some(q), c) if q == c => quote = None,
                    (None, q @ ('\'' | '"')) => quote = Some(q),
                    (None, '>') => break,
                    _ => {}
                }
            }
            text.push(' ');
        } else {
            text.push(c);
        }
    }
    html_escape::decode_html_entities(&text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn excerpt(s: &str, length: usize) -> String {
    if let Some((end, _)) = s.char_indices().nth(length) {
        format!("{}…", &s[..end])
    } else {
        s.to_owned()
    }
}

#[derive(Debug, Serialize)]
pub struct Inbox {
    pub items: Vec<Notification>,
    pub has_more: bool,
    pub url: String,
    pub body_incomplete: bool,
}

pub async fn notifications(
    api: &mut Service,
    unread: bool,
    keyword: Option<&str>,
    limit: usize,
    full: bool,
) -> Result<Inbox> {
    const SIZE: usize = 20;
    if !(1..=200).contains(&limit) {
        return Err(err("通知数量必须在 1–200 之间。"));
    }
    let keyword = keyword.map(str::to_lowercase);
    let mut result = Inbox {
        items: vec![],
        has_more: false,
        url: format!("{WEB}/#/main/notification"),
        body_incomplete: false,
    };
    let mut seen = HashSet::new();
    let mut detail_reads = 0;
    for page in 1..=25 {
        let response = api
            .0
            .query(
                Endpoint::Notifications,
                json!({"pageSize":SIZE,"currentPage":page,"type":"100"}),
            )
            .await?;
        let entries = rows(&response["list"], "通知中心")?;
        let more = integer(&response["total"])
            .map_or(entries.len() == SIZE, |total| total > (page * SIZE) as i64);
        let before = seen.len();
        for (index, raw) in entries.iter().enumerate() {
            let key = id(raw, &["id"])?;
            if !seen.insert(key.clone()) {
                continue;
            }
            let is_unread = match integer(&raw["isread"]) {
                Some(0) => Some(true),
                Some(1) => Some(false),
                _ => None,
            };
            let title = text(&raw["title"]).unwrap_or_else(|| "（无标题）".into());
            let course = text(&raw["coursename"]);
            let mut content = raw["content"]
                .as_str()
                .map(notice_text)
                .filter(|s| !s.is_empty());
            if unread && is_unread != Some(true) {
                continue;
            }
            // The web client fetches school-issued bodies separately from marking as read.
            if (full || keyword.is_some())
                && content.is_none()
                && let Some(coid) = text(&raw["coid"]).filter(|v| v != "0")
            {
                if detail_reads >= 20 {
                    result.body_incomplete = true;
                } else {
                    detail_reads += 1;
                    match api
                        .0
                        .query(Endpoint::NotificationBody, json!({"id":key, "coid":coid}))
                        .await
                    {
                        Ok(body) => {
                            content = body["content"]
                                .as_str()
                                .map(notice_text)
                                .filter(|s| !s.is_empty())
                        }
                        Err(e @ crate::client::Error::SessionExpired(_)) => return Err(e),
                        Err(_) => result.body_incomplete = true,
                    }
                }
            }
            if keyword.as_ref().is_some_and(|k| {
                !title.to_lowercase().contains(k)
                    && !course
                        .as_ref()
                        .is_some_and(|s| s.to_lowercase().contains(k))
                    && !content
                        .as_ref()
                        .is_some_and(|s| s.to_lowercase().contains(k))
            }) {
                continue;
            }
            let category = match integer(&raw["type"]) {
                Some(0) => "教学活动",
                Some(1) => "教务通知",
                Some(2) => "系统通知",
                Some(3) => "申诉申请",
                _ => "其他",
            };
            result.items.push(Notification {
                id: key,
                title,
                category,
                course,
                unread: is_unread,
                published_at: time(&raw["createtime"]),
                content: content.map(|s| excerpt(&s, if full { 4000 } else { 160 })),
            });
            if result.items.len() == limit {
                result.has_more = more || index + 1 < entries.len();
                return Ok(result);
            }
        }
        if !more {
            return Ok(result);
        }
        if seen.len() == before {
            return Err(err("课堂派通知分页重复或提前结束。"));
        }
    }
    result.has_more = true;
    Ok(result)
}

#[derive(Debug)]
pub struct Located {
    pub path: Vec<String>,
    pub row: Value,
}

pub async fn walk(
    api: &mut Service,
    course: &Course,
    kind: Kind,
    root: &str,
    depth: u8,
) -> Result<(Vec<Located>, bool)> {
    if depth > 8 {
        return Err(err("目录深度不能超过 8 层。"));
    }
    let mut queue = VecDeque::from([(root.to_owned(), vec![])]);
    let mut visited = HashSet::from([root.to_owned()]);
    let mut results = vec![];
    let mut limited = false;
    let mut fetched = 0;
    while let Some((directory, path)) = queue.pop_front() {
        if fetched == 40 {
            limited = true;
            break;
        }
        fetched += 1;
        for row in api.content(course, kind, &directory).await? {
            if folder(&row) {
                let dir = id(&row, &["id"])?;
                if !visited.insert(dir.clone()) {
                    continue;
                }
                if path.len() < usize::from(depth) {
                    let mut child = path.clone();
                    child.push(first(&row, &["title", "name"]).unwrap_or(dir.clone()));
                    queue.push_back((dir, child));
                } else {
                    limited = true;
                }
            } else {
                results.push(Located {
                    path: path.clone(),
                    row,
                });
                if results.len() >= 2000 {
                    return Ok((results, true));
                }
            }
        }
    }
    Ok((results, limited))
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Complete,
    Pending,
    InProgress,
    Unknown,
}

impl State {
    pub fn label(self) -> &'static str {
        match self {
            Self::Complete => "已完成",
            Self::Pending => "未完成",
            Self::InProgress => "进行中",
            Self::Unknown => "未知",
        }
    }
}

#[derive(Serialize)]
pub struct Activity {
    pub id: String,
    pub chapter: String,
    pub title: String,
    pub kind: &'static str,
    pub state: State,
}

#[derive(Default, Serialize)]
pub struct Counts {
    pub complete: usize,
    pub pending: usize,
    pub in_progress: usize,
    pub unknown: usize,
}

impl Counts {
    fn add(&mut self, state: State) {
        match state {
            State::Complete => self.complete += 1,
            State::Pending => self.pending += 1,
            State::InProgress => self.in_progress += 1,
            State::Unknown => self.unknown += 1,
        }
    }
}

#[derive(Serialize)]
pub struct Progress {
    pub course_id: String,
    pub course: String,
    pub counts: Counts,
    pub items: Vec<Activity>,
    pub limited: bool,
    pub chapters: std::collections::BTreeMap<String, Counts>,
}

#[derive(Serialize)]
pub struct ProgressSummary {
    pub course_id: String,
    pub course: String,
    pub counts: Counts,
    pub limited: bool,
}

#[derive(Serialize)]
pub struct ProgressOverview {
    pub semester: Option<Semester>,
    pub items: Vec<ProgressSummary>,
    pub errors: Vec<Issue>,
    pub limited: bool,
}

pub async fn progress_overview(
    api: &mut Service,
    scope: &Scope,
    depth: u8,
    max_courses: usize,
) -> Result<ProgressOverview> {
    if depth > 8 || !(1..=40).contains(&max_courses) {
        return Err(err("课程上限必须在 1–40，目录深度必须在 0–8。"));
    }
    let list = api.courses(scope).await?;
    let mut out = ProgressOverview {
        semester: list.semester,
        items: vec![],
        errors: vec![],
        limited: list.items.len() > max_courses,
    };
    for course in list.items.into_iter().take(max_courses) {
        match progress_in_course(api, &course, "0", depth, false).await {
            Ok(p) => out.items.push(ProgressSummary {
                course_id: p.course_id,
                course: p.course,
                counts: p.counts,
                limited: p.limited,
            }),
            Err(e) if e.is_auth() => return Err(e),
            Err(e) => out.errors.push(Issue {
                course_id: course.id,
                course_title: course.title,
                error: e.to_string(),
            }),
        }
    }
    out.items.sort_by_key(|x| {
        (
            std::cmp::Reverse(x.counts.pending + x.counts.in_progress),
            x.course.clone(),
        )
    });
    Ok(out)
}

#[derive(Serialize)]
pub struct MaterialMatch {
    pub chapter: Vec<String>,
    #[serde(flatten)]
    pub material: Item,
}

#[derive(Serialize)]
pub struct MaterialSearch {
    pub items: Vec<MaterialMatch>,
    pub limited: bool,
}

pub async fn find_materials(
    api: &mut Service,
    scope: &Scope,
    selector: &str,
    directory: &str,
    depth: u8,
    keyword: &str,
) -> Result<MaterialSearch> {
    let keyword = keyword.trim().to_lowercase();
    if keyword.is_empty() {
        return Err(err("资料搜索关键词不能为空。"));
    }
    let list = api.courses(scope).await?;
    let course = choose(&list.items, selector)?;
    let (entries, limited) = walk(api, course, Kind::Material, directory, depth).await?;
    let mut items = vec![];
    for entry in entries {
        let material = normalize(&entry.row, Kind::Material, course)?;
        if material.title.to_lowercase().contains(&keyword)
            || material
                .attachments
                .iter()
                .any(|a| a.name.to_lowercase().contains(&keyword))
        {
            items.push(MaterialMatch {
                chapter: entry.path,
                material,
            });
        }
    }
    Ok(MaterialSearch { items, limited })
}

fn activity(raw: &Value, section: String) -> Result<Option<Activity>> {
    let (kind, state) = match integer(&raw["contenttype"]) {
        Some(1) => {
            let learning = raw["learning"]
                .as_f64()
                .or_else(|| raw["learning"].as_str()?.parse().ok());
            (
                "互动课件",
                match learning {
                    Some(v) if v.is_finite() && v >= 1.0 => State::Complete,
                    Some(v) if v > 0.0 && v < 1.0 => State::InProgress,
                    Some(0.0) => State::Pending,
                    _ => State::Unknown,
                },
            )
        }
        Some(2 | 8) => (
            "资料",
            match integer(&raw["state"]) {
                Some(1) => State::Complete,
                Some(0) => State::InProgress,
                Some(-1) => State::Pending,
                _ => State::Unknown,
            },
        ),
        Some(4) => (
            "作业",
            match integer(&raw["mstatus"]) {
                Some(1 | 2 | 4) => State::Complete,
                Some(0 | 3) => State::Pending,
                _ => State::Unknown,
            },
        ),
        Some(6) => (
            "测试",
            match integer(&raw["submit_state"]) {
                Some(x) if x >= 4 => State::Complete,
                Some(1 | 3) => State::Pending,
                Some(2) => State::InProgress,
                _ => State::Unknown,
            },
        ),
        _ => return Ok(None),
    };
    Ok(Some(Activity {
        id: id(raw, &["id", "homeworkid"])?,
        chapter: section,
        title: first(raw, &["title", "name"]).unwrap_or_else(|| "（无标题）".into()),
        kind,
        state,
    }))
}

pub async fn progress(
    api: &mut Service,
    scope: &Scope,
    selector: &str,
    directory: &str,
    depth: u8,
    todo: bool,
) -> Result<Progress> {
    let listing = api.courses(scope).await?;
    let course = choose(&listing.items, selector)?;
    progress_in_course(api, course, directory, depth, todo).await
}

async fn progress_in_course(
    api: &mut Service,
    course: &Course,
    directory: &str,
    depth: u8,
    todo: bool,
) -> Result<Progress> {
    let (mut entries, limited) = walk(api, course, Kind::Chapter, directory, depth).await?;
    // Some classrooms use only the flat activity tabs and have no chapter tree.
    if entries.is_empty() && !limited && directory == "0" {
        for (kind, type_id) in [
            (Kind::Courseware, 1),
            (Kind::Material, 2),
            (Kind::Homework, 4),
            (Kind::Test, 6),
        ] {
            for mut row in api
                .content(course, kind, directory)
                .await?
                .into_iter()
                .filter(|x| !folder(x))
            {
                if row["contenttype"].is_null() {
                    row["contenttype"] = json!(type_id);
                }
                entries.push(Located { path: vec![], row });
            }
        }
    }
    let mut result = Progress {
        course_id: course.id.clone(),
        course: course.title.clone(),
        counts: Counts::default(),
        items: vec![],
        limited,
        chapters: std::collections::BTreeMap::new(),
    };
    let mut seen = HashSet::new();
    for item in entries {
        let section = if item.path.is_empty() {
            if directory == "0" {
                "课程根目录".into()
            } else {
                directory.to_owned()
            }
        } else {
            item.path.join(" / ")
        };
        if let Some(activity) = activity(&item.row, section)? {
            if !seen.insert((activity.kind, activity.id.clone(), activity.chapter.clone())) {
                continue;
            }
            result.counts.add(activity.state);
            result
                .chapters
                .entry(activity.chapter.clone())
                .or_default()
                .add(activity.state);
            if !todo || activity.state != State::Complete {
                result.items.push(activity);
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn does_not_count_missing_states_as_complete() {
        let a = activity(&json!({"id":"x","contenttype":2}), "一".into())
            .unwrap()
            .unwrap();
        assert_eq!(a.state, State::Unknown);
        let a = activity(
            &json!({"id":"x","contenttype":"1","learning":"0.25"}),
            "一".into(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(a.state, State::InProgress);
    }
}
