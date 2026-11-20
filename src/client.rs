use crate::auth::{self, CredentialsInput};
use reqwest::header::{ACCEPT, HeaderMap, HeaderValue, ORIGIN, REFERER};
use serde_json::{Value, json};
use std::{env, fs, io::Write, path::PathBuf, time::Duration};
use thiserror::Error;
use tokio::time::{Instant, sleep_until};
use zeroize::Zeroizing;

pub const WEB: &str = "https://www.ketangpai.com";
pub const API: &str = "https://openapiv5.ketangpai.com";

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Auth(String),
    #[error("{0}")]
    SessionExpired(String),
    #[error("{0}")]
    Message(String),
}

impl Error {
    pub fn is_auth(&self) -> bool {
        matches!(self, Self::Auth(_) | Self::SessionExpired(_))
    }
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
    Notifications,
    NotificationBody,
    MaterialDetail,
}

impl Endpoint {
    fn path(self) -> &'static str {
        match self {
            Self::Semesters => "/CourseApi/semesterList",
            Self::Courses => "/CourseApi/semesterCourseList",
            Self::CourseContent => "/FutureV2/CourseMeans/getCourseContent",
            Self::Homework => "/Futurev2/Homework/getHomework",
            Self::Profile => "/UserApi/getUserBasinInfo",
            Self::Notifications => "/MessageApi/lists",
            Self::NotificationBody => "/MessageApi/getAgencyMsg",
            Self::MaterialDetail => "/FutureV2/Courseware/query",
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

pub(crate) fn cached_token() -> Result<Option<Zeroizing<String>>> {
    match fs::read_to_string(token_path()?) {
        Ok(token) => {
            let token = Zeroizing::new(token);
            let clean = token.trim_start_matches('\u{feff}').trim();
            Ok((!clean.is_empty()).then(|| Zeroizing::new(clean.to_owned())))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
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

pub fn web_client(token: Option<&str>) -> Result<reqwest::Client> {
    let mut headers = HeaderMap::new();
    if let Some(token) = token {
        let mut secret = HeaderValue::from_str(token).map_err(|_| err("Token 格式无效。"))?;
        secret.set_sensitive(true);
        headers.insert("token", secret);
    }
    headers.insert(ORIGIN, HeaderValue::from_static(WEB));
    headers.insert(
        REFERER,
        HeaderValue::from_static("https://www.ketangpai.com/"),
    );
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    reqwest::Client::builder()
        .default_headers(headers)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(6))
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| err("无法初始化 HTTPS 客户端。"))
}

pub enum Client {
    Live {
        http: reqwest::Client,
        next: Instant,
        can_relogin: bool,
    },
    Demo,
}

impl Client {
    fn live(http: reqwest::Client, can_relogin: bool) -> Self {
        Self::Live {
            http,
            next: Instant::now(),
            can_relogin,
        }
    }

    pub async fn configured() -> Result<Self> {
        if let Ok(token) = env::var("KETANGPAI_TOKEN") {
            let token = Zeroizing::new(token);
            if !token.trim().is_empty() {
                return Ok(Self::live(client_for_token(token.trim())?, false));
            }
        }
        if let Some(token) = cached_token()? {
            return Ok(Self::live(client_for_token(&token)?, true));
        }
        let creds =
            auth::resolve(CredentialsInput::default())?.ok_or_else(auth::missing_credentials)?;
        let (http, _) = auth::login(creds, true).await?;
        Ok(Self::live(http, true))
    }

    pub async fn query(&mut self, endpoint: Endpoint, payload: Value) -> Result<Value> {
        match self {
            Self::Demo => unwrap_response(endpoint, crate::demo::answer(endpoint, &payload)),
            Self::Live {
                http,
                next,
                can_relogin,
            } => {
                sleep_until(*next).await;
                *next = Instant::now() + Duration::from_millis(250);
                let first = query_http(http, endpoint, payload.clone()).await;
                if !*can_relogin || !matches!(first, Err(Error::SessionExpired(_))) {
                    return first;
                }
                let Some(creds) = auth::resolve(CredentialsInput::default())? else {
                    return first;
                };
                sleep_until(*next).await;
                let (renewed, _) = auth::login(creds, true).await?;
                *http = renewed;
                sleep_until(*next).await;
                *next = Instant::now() + Duration::from_millis(250);
                query_http(http, endpoint, payload).await
            }
        }
    }
}

fn client_for_token(token: &str) -> Result<reqwest::Client> {
    if token.is_empty() || !token.bytes().all(|b| (33..=126).contains(&b)) {
        return Err(Error::Auth(
            "Token 格式无效，请重新登录或只复制 Token 请求头的值。".into(),
        ));
    }
    web_client(Some(token))
}

pub async fn verify_token(token: &str) -> Result<(reqwest::Client, Value)> {
    let http = client_for_token(token)?;
    let data = query_http(&http, Endpoint::Profile, json!({})).await?;
    let profile = crate::service::normalize_profile(&data)?;
    if profile["id"].as_str().is_none() {
        return Err(Error::Auth("未能确认课堂派登录会话。".into()));
    }
    Ok((http, profile))
}

async fn query_http(
    http: &reqwest::Client,
    endpoint: Endpoint,
    mut payload: Value,
) -> Result<Value> {
    payload["reqtimestamp"] = json!(chrono::Utc::now().timestamp_millis());
    let response = http
        .post(format!("{API}{}", endpoint.path()))
        .json(&payload)
        .send()
        .await
        .map_err(|_| err("连接课堂派失败或超时，请检查网络。"))?;
    let status = response.status();
    match status.as_u16() {
        401 => return Err(expired()),
        403 => return Err(Error::Auth("课堂派拒绝访问，请检查账号权限。".into())),
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

fn expired() -> Error {
    Error::SessionExpired(
        "课堂派会话已过期。请运行 ktp login，或用 ktp login --token 导入新的网页 Token。".into(),
    )
}

fn unwrap_response(endpoint: Endpoint, mut envelope: Value) -> Result<Value> {
    if envelope["status"] != 1 && envelope["status"] != "1" {
        let message = envelope["message"].as_str().unwrap_or("");
        let code = crate::service::integer(&envelope["code"]);
        if matches!(code, Some(20100 | 20101)) {
            return Err(Error::Auth(
                "账号平台已迁移，请在所属课堂派网页重新登录。".into(),
            ));
        }
        if code == Some(20003)
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
            return Err(expired());
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
        return Err(expired());
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
        assert!(matches!(e, Error::SessionExpired(_)));
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
        assert!(matches!(e, Error::SessionExpired(_)));
    }
}
