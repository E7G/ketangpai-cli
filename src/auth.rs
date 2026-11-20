use crate::client::{API, Error, Result, err, save_token, verify_token, web_client};
use aes::Aes128;
use base64::{Engine, engine::general_purpose::STANDARD};
use cbc::cipher::{BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use serde::{Deserialize, Deserializer};
use serde_json::{Value, json};
use std::{
    env,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;
// This is the encoding used by the official web sign-in form; HTTPS protects the request.
const WEB_FORM_KEY: &[u8; 16] = b"ktp4567890123456";

pub struct Secret(Zeroizing<String>);

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        String::deserialize(deserializer).map(|s| Self(Zeroizing::new(s)))
    }
}

#[derive(Deserialize, Default)]
pub struct CredentialsInput {
    #[serde(alias = "email", alias = "account")]
    pub phone: Option<String>,
    pub password: Option<Secret>,
}

pub struct Credentials {
    account: String,
    password: Secret,
}

impl Credentials {
    fn new(account: String, password: Secret) -> Result<Self> {
        let account = account.trim();
        if account.is_empty() || account.len() > 512 || account.chars().any(char::is_control) {
            return Err(err("课堂派账号格式不正确，请输入手机号或邮箱。"));
        }
        if password.0.is_empty() || password.0.len() > 4096 {
            return Err(err("课堂派密码为空或超出支持的长度。"));
        }
        Ok(Self {
            account: account.to_owned(),
            password,
        })
    }

    pub fn prompt() -> Result<Self> {
        let account = rpassword::prompt_password("课堂派手机号或邮箱（输入不可见）：")
            .map_err(|_| err("无法读取账号。"))?;
        let secret = rpassword::prompt_password("课堂派密码（输入不可见）：")
            .map_err(|_| err("无法读取密码。"))?;
        Self::new(account, Secret(Zeroizing::new(secret)))
    }
}

impl CredentialsInput {
    fn pair(self, origin: &str) -> Result<Option<Credentials>> {
        match (self.phone, self.password) {
            (None, None) => Ok(None),
            (Some(a), Some(p)) => Credentials::new(a, p).map(Some),
            _ => Err(err(format!("{origin}中的账号和密码必须成对配置。"))),
        }
    }
}

#[derive(Deserialize)]
struct Nested {
    enabled: Option<bool>,
    #[serde(flatten)]
    credentials: CredentialsInput,
}

#[derive(Deserialize)]
struct Config {
    ketangpai: Option<Nested>,
    #[serde(flatten)]
    direct: CredentialsInput,
}

impl Config {
    fn parse(bytes: &[u8]) -> Result<Option<Credentials>> {
        let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
        let parsed: Self = serde_json::from_slice(bytes)
            .map_err(|_| err("无法解析课堂派 config.json；请检查 JSON 格式和字段类型。"))?;
        match parsed.ketangpai {
            Some(nested) if nested.enabled == Some(false) => Ok(None),
            Some(nested) => nested.credentials.pair("config.json 的 ketangpai 节点"),
            None => parsed.direct.pair("config.json"),
        }
    }
}

fn read_config(path: &Path, required: bool) -> Result<Option<Credentials>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && !required => return Ok(None),
        Err(_) => return Err(err("无法打开课堂派配置文件，请检查路径和读取权限。")),
    };
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(MAX_CONFIG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| err("无法读取课堂派配置文件。"))?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err(err("课堂派配置文件不能超过 1 MiB。"));
    }
    Config::parse(&bytes)
}

pub fn resolve(explicit: CredentialsInput) -> Result<Option<Credentials>> {
    if let Some(credentials) = explicit.pair("登录参数")? {
        return Ok(Some(credentials));
    }
    let account = env::var("KETANGPAI_PHONE").ok().filter(|s| !s.is_empty());
    let password = env::var("KETANGPAI_PASSWORD")
        .ok()
        .filter(|s| !s.is_empty())
        .map(|s| Secret(Zeroizing::new(s)));
    if let Some(credentials) = (CredentialsInput {
        phone: account,
        password,
    })
    .pair("环境变量")?
    {
        return Ok(Some(credentials));
    }
    if let Some(path) = env::var_os("KETANGPAI_CONFIG").filter(|p| !p.is_empty()) {
        return read_config(&PathBuf::from(path), true);
    }
    if let Some(path) = env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("config.json")))
    {
        if path.is_file() {
            return read_config(&path, true);
        }
    }
    read_config(Path::new("config.json"), false)
}

pub fn missing_credentials() -> Error {
    Error::Auth("未配置课堂派账号密码。请运行 ktp login，或配置 KETANGPAI_PHONE / KETANGPAI_PASSWORD 或 config.json。".into())
}

fn encode_password(password: &str) -> Result<String> {
    let mut out = vec![0; (password.len() / 16 + 1) * 16];
    let cipher = cbc::Encryptor::<Aes128>::new(WEB_FORM_KEY.into(), WEB_FORM_KEY.into());
    let encoded = cipher
        .encrypt_padded_b2b_mut::<Pkcs7>(password.as_bytes(), &mut out)
        .map_err(|_| err("无法编码课堂派登录请求。"))?;
    Ok(STANDARD.encode(encoded))
}

fn decode_login(value: &Value) -> Result<Zeroizing<String>> {
    let body = &value["data"];
    if body["safe"] == 1 || body["safe"] == "1" {
        return Err(Error::Auth("课堂派要求通过网页更新密码后再登录。".into()));
    }
    let msg = value["message"].as_str().unwrap_or_default();
    if ["验证码", "二次验证", "两步验证", "安全验证", "扫码", "短信"]
        .iter()
        .any(|s| msg.contains(s))
    {
        return Err(Error::Auth(
            "课堂派要求额外验证。请在网页完成登录，再用 ktp login --token 导入网页 Token。".into(),
        ));
    }
    if value["status"] != 1 && value["status"] != "1" {
        let code = crate::service::integer(&value["code"]);
        if matches!(code, Some(20100 | 20101)) {
            return Err(Error::Auth(
                "账号平台已迁移，请在所属课堂派网页登录。".into(),
            ));
        }
        return Err(Error::Auth(match code {
            Some(code) => format!(
                "课堂派未接受账号密码登录（代码 {code}）。请检查账号密码，或在网页登录确认是否有额外验证。"
            ),
            None => {
                "课堂派未接受账号密码登录。请检查账号密码，或在网页登录确认是否有额外验证。".into()
            }
        }));
    }
    let token = body["token"]
        .as_str()
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            Error::Auth("课堂派未返回登录会话，请在网页登录后重试或导入 Token。".into())
        })?;
    Ok(Zeroizing::new(token.to_owned()))
}

pub async fn login(credentials: Credentials, save: bool) -> Result<(reqwest::Client, Value)> {
    let form = json!({
        "email": credentials.account, "password": encode_password(&credentials.password.0)?,
        "remember": "0", "code": "", "mobile": "", "type": "login", "encryption": 1,
        "reqtimestamp": chrono::Utc::now().timestamp_millis(),
    });
    drop(credentials);
    let response = web_client(None)?
        .post(format!("{API}/UserApi/login"))
        .json(&form)
        .send()
        .await
        .map_err(|_| err("连接课堂派登录服务失败或超时，请检查网络。"))?;
    let status = response.status();
    if status.as_u16() == 429 {
        return Err(err("登录请求过于频繁，请稍后再试。"));
    }
    if !status.is_success() {
        return Err(Error::Auth(format!(
            "课堂派登录返回 HTTP {}。请在网页登录确认账号状态。",
            status.as_u16()
        )));
    }
    let envelope: Value = response
        .json()
        .await
        .map_err(|_| err("课堂派登录未返回有效 JSON。"))?;
    let token = decode_login(&envelope)?;
    drop(envelope);
    let (http, profile) = verify_token(&token).await?;
    if save {
        save_token(&token)?;
    }
    Ok((http, profile))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_upstream_config_shape_and_prior_nested_shape() {
        let direct = Config::parse(br#"{"phone":"test@example.invalid","password":" fixture "}"#)
            .unwrap()
            .unwrap();
        assert_eq!(direct.account, "test@example.invalid");
        assert_eq!(direct.password.0.as_str(), " fixture ");
        let nested =
            Config::parse(br#"{"ketangpai":{"enabled":true,"email":"stub","password":"fixture"}}"#)
                .unwrap()
                .unwrap();
        assert_eq!(nested.account, "stub");
        assert!(
            Config::parse(br#"{"ketangpai":{"enabled":false}}"#)
                .unwrap()
                .is_none()
        );
        assert!(Config::parse(br#"{"phone":"stub"}"#).is_err());
    }

    #[test]
    fn matches_independent_aes_vector_for_web_form() {
        assert_eq!(
            encode_password("fixture-课堂-P@ss!").unwrap(),
            "J+O3vdelMeJ16zCo4p7IEIwZJ1PWWc47uUkwxHJ/i9A="
        );
    }

    #[test]
    fn never_reports_raw_login_messages_or_uses_challenge_sessions() {
        let good = json!({"status":1,"data":{"token":"fixture-secret"}});
        assert_eq!(decode_login(&good).unwrap().as_str(), "fixture-secret");
        let bad = json!({"status":0,"code":12345,"message":"fixture-secret 短信验证"});
        let e = decode_login(&bad).unwrap_err().to_string();
        assert!(e.contains("额外验证"));
        assert!(!e.contains("fixture-secret"));
        assert!(
            decode_login(&json!({"status":1,"data":{"token":"fixture-secret","safe":1}})).is_err()
        );
    }
}
