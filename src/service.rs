use crate::client::{Client, Endpoint, Result, WEB, err};
use chrono::{DateTime, Days, FixedOffset, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashSet;

const PAGE_SIZE: usize = 50;
const MAX_PAGES: usize = 20;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Homework,
    Courseware,
    Material,
    Test,
    Notice,
    Chapter,
}

impl Kind {
    fn filter(self) -> Value {
        match self {
            Self::Homework => json!(4),
            Self::Courseware => json!(1),
            Self::Material => json!(["2", "8"]),
            Self::Test => json!(6),
            Self::Notice => json!(7),
            Self::Chapter => json!(0),
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Homework => "作业",
            Self::Courseware => "互动课件",
            Self::Material => "资料",
            Self::Test => "测试",
            Self::Notice => "公告",
            Self::Chapter => "章节",
        }
    }
}

#[derive(Clone, Default)]
pub struct Scope {
    pub semester: Option<String>,
    pub term: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Semester {
    pub semester: String,
    pub term: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Course {
    pub id: String,
    pub title: String,
    pub class_name: Option<String>,
    pub teacher: Option<String>,
    pub url: String,
}

#[derive(Serialize)]
pub struct TermCourse {
    #[serde(flatten)]
    pub semester: Semester,
    #[serde(flatten)]
    pub course: Course,
}

#[derive(Serialize)]
pub struct CourseCatalog {
    pub items: Vec<TermCourse>,
    pub errors: Vec<String>,
    pub limited: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Attachment {
    pub name: String,
    pub size: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    pub course_id: String,
    pub course_title: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<i64>,
    #[serde(skip_serializing_if = "is_false")]
    pub is_folder: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_code: Option<i64>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_time"
    )]
    pub opens_at: Option<DateTime<FixedOffset>>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_time"
    )]
    pub due_at: Option<DateTime<FixedOffset>>,
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_time"
    )]
    pub published_at: Option<DateTime<FixedOffset>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grade: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_grade: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub question_count: Option<i64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
}

fn is_false(value: &bool) -> bool {
    !value
}

fn serialize_time<S: serde::Serializer>(
    value: &Option<DateTime<FixedOffset>>,
    s: S,
) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_str(
        &value
            .map(|d| d.to_rfc3339_opts(SecondsFormat::Secs, false))
            .unwrap_or_default(),
    )
}

#[derive(Debug, Clone, Serialize)]
pub struct Issue {
    pub course_id: String,
    pub course_title: String,
    pub error: String,
}

#[derive(Debug, Serialize)]
pub struct Listing<T> {
    #[serde(flatten)]
    pub semester: Option<Semester>,
    pub items: Vec<T>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<Issue>,
}

#[derive(Debug, Serialize)]
pub struct Board {
    #[serde(flatten)]
    pub semester: Option<Semester>,
    pub upcoming: Vec<Item>,
    pub overdue: Vec<Item>,
    pub undated: Vec<Item>,
    pub unknown: Vec<Item>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<Issue>,
}

pub fn integer(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_str()?.parse().ok())
}

pub(crate) fn text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

pub(crate) fn first(v: &Value, fields: &[&str]) -> Option<String> {
    fields.iter().find_map(|&f| text(&v[f]))
}

pub(crate) fn id(v: &Value, fields: &[&str]) -> Result<String> {
    first(v, fields).ok_or_else(|| err("课堂派响应中缺少项目编号，接口字段可能已变化。"))
}

pub(crate) fn rows<'a>(v: &'a Value, label: &str) -> Result<&'a [Value]> {
    v.as_array()
        .filter(|a| a.iter().all(Value::is_object))
        .map(Vec::as_slice)
        .ok_or_else(|| err(format!("课堂派「{label}」响应不是预期的列表。")))
}

pub fn shanghai() -> FixedOffset {
    FixedOffset::east_opt(8 * 3600).expect("valid offset")
}

pub(crate) fn time(v: &Value) -> Option<DateTime<FixedOffset>> {
    let tz = shanghai();
    if let Some(n) = integer(v) {
        if n <= 0 {
            return None;
        }
        let dt = if n >= 1_000_000_000_000 {
            DateTime::from_timestamp_millis(n)
        } else {
            DateTime::from_timestamp(n, 0)
        };
        return dt
            .filter(|d| (1900..=9999).contains(&chrono::Datelike::year(d)))
            .map(|d| d.with_timezone(&tz));
    }
    let s = v.as_str()?;
    if let Ok(parsed) = DateTime::parse_from_rfc3339(s) {
        return Some(parsed.with_timezone(&tz));
    }
    for format in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M"] {
        if let Ok(parsed) = NaiveDateTime::parse_from_str(s, format) {
            return tz.from_local_datetime(&parsed).single();
        }
    }
    None
}

fn first_time(v: &Value, fields: &[&str]) -> Option<DateTime<FixedOffset>> {
    fields.iter().find_map(|&f| time(&v[f]))
}

fn link(path: &str, params: &[(&str, &str)]) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(params)
        .finish();
    format!("{WEB}/#/{path}?{query}")
}

fn attachments(v: &Value) -> Vec<Attachment> {
    v["attachment"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            Some(Attachment {
                name: text(&a["name"])?,
                size: text(&a["size"]),
            })
        })
        .collect()
}

pub(crate) fn folder(raw: &Value) -> bool {
    integer(&raw["item_type"]) == Some(2)
}

fn homework_status(code: Option<i64>) -> &'static str {
    match code {
        Some(0) => "未提交",
        Some(1) => "已提交",
        Some(2) => "已批改",
        Some(3) => "被退回",
        Some(4) => "已批改（分数未公布）",
        _ => "未知",
    }
}

fn test_status(code: Option<i64>) -> &'static str {
    match code {
        Some(1) => "未参与",
        Some(2) => "未提交",
        Some(3) => "被退回",
        Some(4 | 5) => "已提交",
        Some(v) if v > 5 => "已批改",
        _ => "未知",
    }
}

pub(crate) fn choose<'a>(items: &'a [Course], selector: &str) -> Result<&'a Course> {
    let selector = selector.trim();
    if selector.is_empty() {
        return Err(err("请输入课程名或课程 ID。"));
    }
    if let Some(c) = items.iter().find(|c| c.id == selector) {
        return Ok(c);
    }
    let needle = selector.to_lowercase();
    let exact: Vec<_> = items
        .iter()
        .filter(|c| c.title.to_lowercase() == needle)
        .collect();
    let found: Vec<_> = if exact.is_empty() {
        items
            .iter()
            .filter(|c| c.title.to_lowercase().contains(&needle))
            .collect()
    } else {
        exact
    };
    match found.as_slice() {
        [course] => Ok(course),
        [] => Err(err(
            "未找到课程。请先运行 ktp courses，或指定对应的学年学期。",
        )),
        _ => Err(err(format!(
            "课程名不唯一，请改用 ID：{}",
            found
                .into_iter()
                .take(10)
                .map(|c| format!("{}（{}）", c.title, c.id))
                .collect::<Vec<_>>()
                .join("、")
        ))),
    }
}

pub struct Service(pub Client);

pub fn normalize_profile(p: &Value) -> Result<Value> {
    if !p.is_object() {
        return Err(err("课堂派个人资料响应格式不符。"));
    }
    Ok(
        json!({"id": first(p, &["uid", "userid", "id"]), "name": first(p, &["realname", "username", "name"]),
              "school": first(p, &["schoolname", "schoolName", "school"])}),
    )
}

impl Service {
    pub async fn all_courses(&mut self) -> Result<CourseCatalog> {
        let terms = self.semesters().await?;
        let mut out = CourseCatalog {
            items: vec![],
            errors: vec![],
            limited: terms.len() > 40,
        };
        for s in terms.into_iter().take(40) {
            let scope = Scope {
                semester: Some(s.semester.clone()),
                term: Some(s.term.clone()),
            };
            match self.courses(&scope).await {
                Ok(list) => out
                    .items
                    .extend(list.items.into_iter().map(|course| TermCourse {
                        semester: s.clone(),
                        course,
                    })),
                Err(e) if e.is_auth() => return Err(e),
                Err(e) => out
                    .errors
                    .push(format!("{} / {}：{}", s.semester, s.term, e)),
            }
        }
        Ok(out)
    }

    pub async fn profile(&mut self) -> Result<Value> {
        let p = self.0.query(Endpoint::Profile, json!({})).await?;
        normalize_profile(&p)
    }

    pub async fn semesters(&mut self) -> Result<Vec<Semester>> {
        let value = self
            .0
            .query(Endpoint::Semesters, json!({"isstudy":"1", "search":""}))
            .await?;
        let mut semesters = Vec::new();
        for row in rows(&value["semester"], "学期")? {
            let s = Semester {
                semester: id(row, &["semester"])?,
                term: id(row, &["term"])?,
            };
            if !semesters.contains(&s) {
                semesters.push(s);
            }
        }
        Ok(semesters)
    }

    pub async fn courses(&mut self, scope: &Scope) -> Result<Listing<Course>> {
        let semester = match (&scope.semester, &scope.term) {
            (None, None) => self.semesters().await?.into_iter().next(),
            (Some(semester), Some(term))
                if !semester.trim().is_empty() && !term.trim().is_empty() =>
            {
                Some(Semester {
                    semester: semester.trim().into(),
                    term: term.trim().into(),
                })
            }
            _ => {
                return Err(err(
                    "请同时指定 --semester 和 --term，例如 --semester 2026-2027 --term 1。",
                ));
            }
        };
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        if let Some(s) = &semester {
            let value = self
                .0
                .query(
                    Endpoint::Courses,
                    json!({"isstudy":"1", "search":"", "semester":s.semester, "term":s.term}),
                )
                .await?;
            for row in rows(&value, "课程")? {
                let course_id = id(row, &["id", "courseid"])?;
                if !seen.insert(course_id.clone()) {
                    continue;
                }
                items.push(Course {
                    url: link(
                        "main/classDetail",
                        &[("courseid", &course_id), ("courserole", "0")],
                    ),
                    id: course_id,
                    title: text(&row["coursename"]).unwrap_or_else(|| "（未命名课程）".into()),
                    class_name: text(&row["classname"]),
                    teacher: first(row, &["username", "teachername"]),
                });
            }
        }
        Ok(Listing {
            semester,
            items,
            errors: vec![],
        })
    }

    pub(crate) async fn content(
        &mut self,
        course: &Course,
        kind: Kind,
        directory: &str,
    ) -> Result<Vec<Value>> {
        let mut records = Vec::new();
        let mut page_seen = HashSet::new();
        let mut ids = HashSet::new();
        for page in 1..=MAX_PAGES {
            let data = self.0.query(Endpoint::CourseContent, json!({
                "courseid": course.id, "courserole": 0, "contenttype": kind.filter(), "dirid": directory,
                "lessonlink": [], "sort": [], "page": page, "limit": PAGE_SIZE, "vtr_type": "",
                "desc": if kind == Kind::Chapter { 2 } else { 3 },
            })).await?;
            let list = rows(&data["list"], kind.label())?;
            let last = integer(&data["pageTotal"]).and_then(|n| usize::try_from(n).ok());
            if list.is_empty() {
                if last.is_some_and(|last| page < last) {
                    return Err(err("课堂派提前返回了空页。"));
                }
                return Ok(records);
            }
            let signature = list
                .iter()
                .map(|v| {
                    Ok((
                        id(v, &["id", "homeworkid"])?,
                        folder(v),
                        integer(&v["contenttype"]),
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            if !page_seen.insert(signature.clone()) {
                return Err(err("课堂派分页重复，本课程结果未采用。"));
            }
            for (row, key) in list.iter().zip(signature) {
                if ids.insert(key) {
                    records.push(row.clone());
                }
            }
            let size = integer(&data["pageSize"])
                .and_then(|n| usize::try_from(n).ok())
                .filter(|&n| n > 0)
                .unwrap_or(PAGE_SIZE);
            if last.map_or(list.len() < size, |last| page >= last) {
                return Ok(records);
            }
        }
        Err(err(format!(
            "「{}」超过 {MAX_PAGES} 页，本课程结果未采用。",
            kind.label()
        )))
    }

    pub async fn list(
        &mut self,
        scope: &Scope,
        kind: Kind,
        course: Option<&str>,
        keyword: Option<&str>,
        directory: &str,
    ) -> Result<Listing<Item>> {
        if matches!(kind, Kind::Material | Kind::Chapter) && course.is_none() {
            return Err(err("请指定一门课程。"));
        }
        let listing = self.courses(scope).await?;
        let mut courses = listing.items.iter().collect::<Vec<_>>();
        if let Some(selector) = course {
            courses = vec![choose(&listing.items, selector)?];
        }
        let one_course = courses.len() == 1;
        let keyword = keyword.map(str::to_lowercase);
        let mut out = Listing {
            semester: listing.semester,
            items: vec![],
            errors: vec![],
        };
        for course in courses {
            let result = self
                .content(course, kind, directory)
                .await
                .and_then(|raws| {
                    raws.iter()
                        .filter(|x| matches!(kind, Kind::Material | Kind::Chapter) || !folder(x))
                        .map(|x| normalize(x, kind, course))
                        .collect::<Result<Vec<_>>>()
                });
            match result {
                Ok(items) => out.items.extend(items.into_iter().filter(|item| {
                    keyword.as_ref().is_none_or(|kw| {
                        item.title.to_lowercase().contains(kw)
                            || item
                                .attachments
                                .iter()
                                .any(|a| a.name.to_lowercase().contains(kw))
                    })
                })),
                Err(e) if one_course || e.is_auth() => return Err(e),
                Err(e) => out.errors.push(Issue {
                    course_id: course.id.clone(),
                    course_title: course.title.clone(),
                    error: e.to_string(),
                }),
            }
        }
        Ok(out)
    }

    pub async fn homework_detail(
        &mut self,
        scope: &Scope,
        course: &str,
        homework: &str,
    ) -> Result<Item> {
        let out = self
            .list(scope, Kind::Homework, Some(course), None, "0")
            .await?;
        let mut entry = out
            .items
            .iter()
            .find(|x| x.id == homework)
            .or_else(|| {
                homework
                    .parse::<usize>()
                    .ok()?
                    .checked_sub(1)
                    .and_then(|i| out.items.get(i))
            })
            .cloned()
            .ok_or_else(|| err("找不到作业，请先运行 ktp hw-ls，再使用作业 ID 或序号。"))?;
        let data = self
            .0
            .query(Endpoint::Homework, json!({"homeworkid":entry.id}))
            .await?;
        if !data.is_object() {
            return Err(err("作业详情响应格式不符。"));
        }
        if let Some(t) = first_time(&data, &["releasetime", "begintime"]) {
            entry.opens_at = Some(t);
        }
        if let Some(t) = time(&data["endtime"]) {
            entry.due_at = Some(t);
        }
        entry.max_grade = first(&data, &["fullscore", "total_score", "totalScore"]);
        entry.question_count = ["question_count", "subject_count", "subjectTotal"]
            .into_iter()
            .find_map(|k| integer(&data[k]).filter(|n| *n >= 0))
            .or(entry.question_count);
        entry.attachments = attachments(&data);
        Ok(entry)
    }

    pub async fn deadlines(
        &mut self,
        scope: &Scope,
        course: Option<&str>,
        days: Option<u16>,
    ) -> Result<Board> {
        if days.is_some_and(|d| !(1..=365).contains(&d)) {
            return Err(err("days 必须在 1–365 之间。"));
        }
        let result = self.list(scope, Kind::Homework, course, None, "0").await?;
        Ok(build_board(result, days, Utc::now()))
    }
}

pub(crate) fn normalize(raw: &Value, kind: Kind, c: &Course) -> Result<Item> {
    let is_folder = folder(raw);
    let key = if kind == Kind::Homework {
        &["homeworkid", "id"][..]
    } else {
        &["id"]
    };
    let item_id = id(raw, key)?;
    let mut i = Item {
        id: item_id,
        title: first(raw, &["title", "name"]).unwrap_or_else(|| "（未命名）".into()),
        course_id: c.id.clone(),
        course_title: c.title.clone(),
        url: c.url.clone(),
        content_type: integer(&raw["contenttype"]),
        is_folder,
        status: None,
        status_code: None,
        opens_at: None,
        due_at: None,
        published_at: None,
        grade: None,
        max_grade: None,
        duration_seconds: None,
        question_count: None,
        attachments: vec![],
    };
    if is_folder {
        return Ok(i);
    }
    let common = [("courseid", c.id.as_str()), ("courserole", "0")];
    match kind {
        Kind::Homework => {
            i.status_code = integer(&raw["mstatus"]);
            i.status = Some(homework_status(i.status_code));
            i.opens_at = first_time(raw, &["releasetime", "begintime"]);
            i.due_at = time(&raw["endtime"]);
            if i.status_code == Some(2) {
                i.grade = text(&raw["grade"]);
            }
            let state = text(&raw["timestate"]).unwrap_or_else(|| "2".into());
            i.url = link(
                "homework",
                &[
                    common[0],
                    common[1],
                    ("homeworkId", &i.id),
                    ("type", &state),
                ],
            );
        }
        Kind::Test => {
            i.status_code = integer(&raw["submit_state"]);
            i.status = Some(test_status(i.status_code));
            i.opens_at = first_time(raw, &["begintime", "releasetime"]);
            i.due_at = time(&raw["endtime"]);
            i.duration_seconds = integer(&raw["timelength"]).filter(|n| *n > 0);
            i.question_count = integer(&raw["subjectTotal"]).filter(|n| *n >= 0);
            // Mirror the student card's publication rules, including unpublished grades.
            if matches!(
                (integer(&raw["over"]), i.status_code),
                (Some(0), Some(6 | 7)) | (Some(1), Some(8 | 10))
            ) {
                i.grade = text(&raw["score"]);
                i.max_grade = text(&raw["total_score"]);
            }
        }
        Kind::Notice => {
            i.published_at = time(&raw["releasetime"]);
            i.url = link("billboard_detail", &[common[0], common[1], ("id", &i.id)]);
        }
        Kind::Material => {
            i.attachments = attachments(raw);
            let ty = if i.content_type == Some(8) { "8" } else { "2" };
            i.url = link(
                "resource_detail",
                &[common[0], common[1], ("id", &i.id), ("type", ty)],
            );
        }
        Kind::Chapter | Kind::Courseware => {}
    }
    Ok(i)
}

fn build_board(result: Listing<Item>, days: Option<u16>, now: DateTime<Utc>) -> Board {
    let until = days.and_then(|days| now.checked_add_days(Days::new(days.into())));
    let mut b = Board {
        semester: result.semester,
        errors: result.errors,
        upcoming: vec![],
        overdue: vec![],
        undated: vec![],
        unknown: vec![],
    };
    for i in result.items {
        match i.status_code {
            Some(1 | 2 | 4) => {}
            Some(0 | 3) => match i.due_at.map(|due| due.to_utc()) {
                None => b.undated.push(i),
                Some(due) if due < now => b.overdue.push(i),
                Some(due) if until.is_none_or(|u| due <= u) => b.upcoming.push(i),
                _ => {}
            },
            _ => b.unknown.push(i),
        }
    }
    b.upcoming.sort_by_key(|i| i.due_at);
    b.overdue.sort_by_key(|i| std::cmp::Reverse(i.due_at));
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_seconds_milliseconds_and_china_local_time() {
        let a = time(&json!(1795201200)).unwrap();
        assert_eq!(a, time(&json!(1795201200000_i64)).unwrap());
        assert_eq!(a.offset().local_minus_utc(), 28800);
        assert_eq!(
            time(&json!("2026-11-21 12:00:00")).unwrap().to_utc(),
            time(&json!("2026-11-21T04:00:00Z")).unwrap().to_utc()
        );
        assert_eq!(time(&json!("0")), None);
    }

    #[tokio::test]
    async fn demo_exercises_pagination_and_status_handling() {
        let mut api = Service(Client::Demo);
        let result = api
            .deadlines(&Scope::default(), None, Some(10))
            .await
            .unwrap();
        assert_eq!(result.upcoming.len(), 2);
        assert_eq!(result.overdue.len(), 1);
        assert_eq!(result.undated.len(), 1);
        assert_eq!(result.unknown.len(), 1);
        assert!(result.upcoming[0].due_at < result.upcoming[1].due_at);
        let detail = api
            .homework_detail(&Scope::default(), "数字", "1")
            .await
            .unwrap();
        assert_eq!(detail.id, "hw-1");
        assert_eq!(detail.max_grade.as_deref(), Some("100"));
    }
}
