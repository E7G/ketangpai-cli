use reqwest::header::{ACCEPT, HeaderMap, HeaderValue, ORIGIN, REFERER};
use serde_json::{Value, json};
use std::{env, fs, io::Write, path::PathBuf, time::Duration};
use thiserror::Error;
use tokio::time::{Instant, sleep_until};

pub const WEB: &str = "https://www.ketangpai.com";
const API: &str = "https://openapiv5.ketangpai.com";

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Auth(String),
    #[error("{0}")]
    Message(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn err(message: impl Into<String>) -> Error {
    Error::Message(message.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endpoint {
    Semesters,
    Courses,
    CourseContent,
    Homework,
    Profile,
}

impl Endpoint {
    fn path(self) -> &'static str {
        match self {
            Self::Semesters => "/CourseApi/semesterList",
            Self::Courses => "/CourseApi/semesterCourseList",
            Self::CourseContent => "/FutureV2/CourseMeans/getCourseContent",
            Self::Homework => "/Futurev2/Homework/getHomework",
            Self::Profile => "/UserApi/getUserBasinInfo",
        }
    }
}

pub fn token_path() -> Result<PathBuf> {
    if let Some(path) = env::var_os("KETANGPAI_TOKEN_FILE").filter(|x| !x.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let base = if cfg!(windows) {
        env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
    };
    Ok(base
        .ok_or_else(|| err("无法定位本机配置目录，请设置 KETANGPAI_TOKEN_FILE。"))?
        .join("ketangpai-cli")
        .join("session.token"))
}

pub fn read_token() -> Result<String> {
    if let Ok(token) = env::var("KETANGPAI_TOKEN") {
        if !token.trim().is_empty() {
            return Ok(token.trim().to_owned());
        }
    }
    match fs::read_to_string(token_path()?) {
        Ok(token) => Ok(token.trim_start_matches('\u{feff}').trim().to_owned()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(_) => Err(err("无法读取本机 Token 文件。")),
    }
}

pub fn save_token(token: &str) -> Result<()> {
    let path = token_path()?;
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    let parent = parent.unwrap_or_else(|| std::path::Path::new("."));
    fs::create_dir_all(parent).map_err(|_| err("无法创建本机 Token 目录。"))?;
    let mut file =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| err("无法创建本机 Token 文件。"))?;
    writeln!(file, "{token}").map_err(|_| err("无法写入本机 Token 文件。"))?;
    file.persist(path)
        .map_err(|_| err("无法保存本机 Token 文件。"))?;
    Ok(())
}

pub enum Client {
    Live {
        http: reqwest::Client,
        next: Instant,
    },
    Demo,
}

impl Client {
    pub fn live(token: &str) -> Result<Self> {
        if token.is_empty() {
            return Err(Error::Auth(
                "未配置 Token。请运行 ktp login --save，或设置 KETANGPAI_TOKEN。".into(),
            ));
        }
        if !token.bytes().all(|b| (33..=126).contains(&b)) {
            return Err(Error::Auth(
                "Token 格式无效，请只复制 Token 请求头的值。".into(),
            ));
        }
        let mut headers = HeaderMap::new();
        let mut secret = HeaderValue::from_str(token).map_err(|_| err("Token 格式无效。"))?;
        secret.set_sensitive(true);
        headers.insert("token", secret);
        headers.insert(ORIGIN, HeaderValue::from_static(WEB));
        headers.insert(
            REFERER,
            HeaderValue::from_static("https://www.ketangpai.com/"),
        );
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(6))
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|_| err("无法初始化 HTTPS 客户端。"))?;
        Ok(Self::Live {
            http,
            next: Instant::now(),
        })
    }

    pub async fn query(&mut self, endpoint: Endpoint, mut payload: Value) -> Result<Value> {
        payload["reqtimestamp"] = json!(chrono::Utc::now().timestamp_millis());
        match self {
            Self::Demo => unwrap_response(endpoint, crate::demo::answer(endpoint, &payload)),
            Self::Live { http, next } => {
                sleep_until(*next).await;
                *next = Instant::now() + Duration::from_millis(250);
                let response = http
                    .post(format!("{API}{}", endpoint.path()))
                    .json(&payload)
                    .send()
                    .await
                    .map_err(|_| err("连接课堂派失败或超时，请检查网络。"))?;
                let status = response.status();
                match status.as_u16() {
                    401 | 403 => {
                        return Err(Error::Auth(
                            "课堂派拒绝访问。请更新 Token 或检查账号权限。".into(),
                        ));
                    }
                    429 => return Err(err("课堂派请求过于频繁，请稍后重试。")),
                    _ if !status.is_success() => {
                        return Err(err(format!(
                            "课堂派返回 HTTP {}（{endpoint:?}）。",
                            status.as_u16()
                        )));
                    }
                    _ => {}
                }
                let envelope = response
                    .json()
                    .await
                    .map_err(|_| err(format!("课堂派未返回有效 JSON（{endpoint:?}）。")))?;
                unwrap_response(endpoint, envelope)
            }
        }
    }
}

fn unwrap_response(endpoint: Endpoint, mut envelope: Value) -> Result<Value> {
    if envelope["status"] != 1 && envelope["status"] != "1" {
        let message = envelope["message"].as_str().unwrap_or("");
        let code = crate::service::integer(&envelope["code"]);
        if matches!(code, Some(20003 | 20100 | 20101))
            || [
                "未登录",
                "登录失效",
                "重新登录",
                "请登录",
                "重新登陆",
                "登陆已过期",
            ]
            .iter()
            .any(|s| message.contains(s))
        {
            return Err(Error::Auth(
                "登录失效或账号平台已迁移。请在课堂派网页重新登录并更新 Token。".into(),
            ));
        }
        return Err(err(format!(
            "课堂派查询未成功（{endpoint:?}），接口可能变化或内容不可访问。"
        )));
    }
    let data = envelope
        .as_object_mut()
        .and_then(|x| x.remove("data"))
        .ok_or_else(|| err(format!("课堂派响应缺少 data（{endpoint:?}）。")))?;
    // getUserBasinInfo returns a successful guest envelope for an invalid token.
    if endpoint == Endpoint::Profile && data["uid"] == "MDAwMDAwMDAwMA" {
        return Err(Error::Auth(
            "课堂派只返回了访客资料，请重新登录并更新 Token。".into(),
        ));
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_are_classified_without_copying_private_body() {
        let e = unwrap_response(
            Endpoint::Courses,
            json!({"status":0,"code":"20003","message":"secret"}),
        )
        .unwrap_err();
        assert!(matches!(e, Error::Auth(_)));
        assert!(!e.to_string().contains("secret"));
        assert!(unwrap_response(Endpoint::Courses, json!({"status":1})).is_err());
        assert_eq!(
            unwrap_response(Endpoint::Courses, json!({"status":"1","data":[]})).unwrap(),
            json!([])
        );
    }

    #[test]
    fn successful_guest_profile_is_not_a_valid_session() {
        let e = unwrap_response(
            Endpoint::Profile,
            json!({"status":1,"code":10000,"data":{"uid":"MDAwMDAwMDAwMA"}}),
        )
        .unwrap_err();
        assert!(matches!(e, Error::Auth(_)));
    }
}
