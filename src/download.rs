use crate::client::{Client, Endpoint, Result, WEB, err};
use crate::service::{Kind, Scope, Service, choose, first, id, integer, text};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
use url::Url;

const MAX_BYTES: u64 = 256 * 1024 * 1024;

pub struct Options<'a> {
    pub course: &'a str,
    pub directory: &'a str,
    pub depth: u8,
    pub keyword: Option<&'a str>,
    pub material: Option<&'a str>,
    pub max_files: usize,
    pub output: &'a Path,
    pub dry_run: bool,
}

#[derive(Serialize)]
pub struct FileResult {
    pub material: String,
    pub name: Option<String>,
    pub status: &'static str,
    pub path: Option<PathBuf>,
    pub reason: Option<String>,
}

#[derive(Serialize)]
pub struct Batch {
    pub items: Vec<FileResult>,
    pub limited: bool,
}

fn allowed(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return false;
    };
    // Only the platform's known file service and observed first-party OSS buckets.
    let host_ok = host == "downloadv5.ketangpai.com"
        || host == "download.ketangpai.com"
        || host == "ketangpai-test.oss.aliyuncs.com"
        || (host.starts_with("ketangpai-live.")
            && host.ends_with(".aliyuncs.com")
            && host
                .split('.')
                .nth(1)
                .is_some_and(|label| label.starts_with("oss-"))
            && host.split('.').count() == 4);
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && host_ok
}

fn attachment_url(raw: &str) -> Option<Url> {
    let mut url = if raw.starts_with("//") {
        Url::parse(&format!("https:{raw}")).ok()?
    } else {
        Url::parse(raw).ok()?
    };
    if url.scheme() == "http" && url.port().is_none() {
        url.set_scheme("https").ok()?;
    }
    if !allowed(&url) {
        return None;
    }
    if url
        .host_str()
        .is_some_and(|h| h.ends_with(".ketangpai.com"))
    {
        let keys: Vec<_> = url.query_pairs().map(|(k, _)| k.into_owned()).collect();
        let mut q = url.query_pairs_mut();
        if !keys.iter().any(|k| k == "useCdn") {
            q.append_pair("useCdn", "true");
        }
        if !keys.iter().any(|k| k == "ossmode") {
            q.append_pair("ossmode", "1");
        }
    }
    Some(url)
}

pub(crate) fn filename(raw: &str) -> String {
    let leaf = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let clean: String = leaf.chars().map(|c| if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') { '_' } else { c }).collect();
    let clean = clean.trim_matches([' ', '.']);
    let stem = clean
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end()
        .to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&stem.as_str())
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|n| {
                matches!(
                    n,
                    "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        });
    let name: String = clean
        .chars()
        .scan(0usize, |n, c| {
            *n += c.len_utf8();
            (*n <= 180).then_some(c)
        })
        .collect();
    let name = name.trim_end_matches([' ', '.']);
    if name.is_empty() {
        "attachment.bin".into()
    } else if reserved {
        format!("_{name}")
    } else {
        name.into()
    }
}

fn create_http() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|_| err("无法初始化资料下载客户端。"))
}

async fn transfer(
    http: &reqwest::Client,
    mut u: Url,
    directory: &Path,
    name: &str,
) -> Result<bool> {
    let dest = directory.join(name);
    if dest.symlink_metadata().is_ok() {
        return Ok(false);
    }
    let mut count = 0;
    let mut response = loop {
        let res = http
            .get(u.clone())
            .header(reqwest::header::REFERER, format!("{WEB}/"))
            .send()
            .await
            .map_err(|_| err("连接资料下载服务失败或超时。"))?;
        if !res.status().is_redirection() {
            break res;
        }
        count += 1;
        if count > 5 {
            return Err(err("资料下载重定向过多。"));
        }
        u = res
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|s| s.to_str().ok())
            .and_then(|s| u.join(s).ok())
            .filter(allowed)
            .ok_or_else(|| err("资料下载跳转到了未支持的地址，请在网页下载。"))?;
    };
    if response.status() != reqwest::StatusCode::OK {
        return Err(err(format!(
            "资料下载返回 HTTP {}。",
            response.status().as_u16()
        )));
    }
    // reqwest reports the remaining body size; capture it before consuming the stream.
    let expected_bytes = response.content_length();
    if expected_bytes.is_some_and(|n| n > MAX_BYTES) {
        return Err(err("文件超过 256 MiB，请到网页下载。"));
    }
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|s| s.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let is_attachment = response
        .headers()
        .get(reqwest::header::CONTENT_DISPOSITION)
        .and_then(|s| s.to_str().ok())
        .is_some_and(|s| s.to_ascii_lowercase().starts_with("attachment"));
    if !is_attachment
        && (mime.is_empty()
            || mime.contains("html")
            || mime.contains("json")
            || mime.contains("xml"))
    {
        return Err(err("下载服务未返回可确认的附件，请到网页下载。"));
    }
    let mut temp =
        tempfile::NamedTempFile::new_in(directory).map_err(|_| err("无法创建下载临时文件。"))?;
    let mut bytes = 0u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| err("下载中断，未保留临时文件。"))?
    {
        bytes += chunk.len() as u64;
        if bytes > MAX_BYTES {
            return Err(err("文件超过 256 MiB，请到网页下载。"));
        }
        temp.write_all(&chunk)
            .map_err(|_| err("无法写入本地文件。"))?;
    }
    if bytes == 0 || expected_bytes.is_some_and(|n| n != bytes) {
        return Err(err("下载回包为空或不完整。"));
    }
    match temp.persist_noclobber(&dest) {
        Ok(_) => Ok(true),
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(_) => Err(err("无法保存下载文件。")),
    }
}

fn result(
    material: &str,
    name: Option<String>,
    status: &'static str,
    path: Option<PathBuf>,
    reason: Option<&str>,
) -> FileResult {
    FileResult {
        material: material.into(),
        name,
        status,
        path,
        reason: reason.map(str::to_owned),
    }
}

fn matches_keyword(row: &Value, keyword: &str) -> bool {
    text(&row["title"]).is_some_and(|s| s.to_lowercase().contains(keyword))
        || row["attachment"].as_array().is_some_and(|a| {
            a.iter()
                .any(|f| text(&f["name"]).is_some_and(|n| n.to_lowercase().contains(keyword)))
        })
}

pub async fn batch(api: &mut Service, scope: &Scope, options: Options<'_>) -> Result<Batch> {
    if !(1..=100).contains(&options.max_files) {
        return Err(err("单批下载数量必须在 1–100 之间。"));
    }
    let is_demo = matches!(api.0, Client::Demo);
    if is_demo && !options.dry_run {
        return Err(err("离线演示只支持 download --dry-run，不会创建文件。"));
    }
    let course_list = api.courses(scope).await?;
    let course = choose(&course_list.items, options.course)?;
    let (rows, limited) = crate::student::walk(
        api,
        course,
        Kind::Material,
        options.directory,
        options.depth,
    )
    .await?;
    let mut batch = Batch {
        items: vec![],
        limited,
    };
    let mut ids = std::collections::HashSet::new();
    let keyword = options.keyword.map(str::to_lowercase);
    let mut counted = 0;
    let mut http = None;
    for item in rows {
        let row = item.row;
        let item_id = id(&row, &["id"])?;
        if options.material.is_some_and(|m| m != item_id)
            || keyword.as_ref().is_some_and(|k| !matches_keyword(&row, k))
            || !ids.insert(item_id.clone())
        {
            continue;
        }
        let label = first(&row, &["title", "name"]).unwrap_or_else(|| item_id.clone());
        if integer(&row["contenttype"]) == Some(8) || integer(&row["candownload"]) == Some(0) {
            batch.items.push(result(
                &label,
                None,
                "skipped",
                None,
                Some("外链或课堂派不允许下载。"),
            ));
            continue;
        }
        if counted == options.max_files {
            batch.limited = true;
            break;
        }
        let data = match api
            .0
            .query(
                Endpoint::MaterialDetail,
                json!({"courseid":course.id,"id":item_id,"contenttype":"2"}),
            )
            .await
        {
            Ok(value) => value,
            Err(e) if e.is_auth() => return Err(e),
            Err(e) => {
                batch
                    .items
                    .push(result(&label, None, "failed", None, Some(&e.to_string())));
                continue;
            }
        };
        if integer(&data["candownload"]) != Some(1) {
            batch.items.push(result(
                &label,
                None,
                "skipped",
                None,
                Some("课堂派未明确允许下载。"),
            ));
            continue;
        }
        let Some(attachments) = data["attachment"].as_array().filter(|a| !a.is_empty()) else {
            batch
                .items
                .push(result(&label, None, "skipped", None, Some("没有附件。")));
            continue;
        };
        for f in attachments {
            if counted >= options.max_files {
                batch.limited = true;
                return Ok(batch);
            }
            let name = filename(&text(&f["name"]).unwrap_or_else(|| "attachment.bin".into()));
            let Some(url) = f["url"].as_str().and_then(attachment_url) else {
                batch.items.push(result(
                    &label,
                    Some(name),
                    "skipped",
                    None,
                    Some("没有受支持的 HTTPS 下载链接。"),
                ));
                continue;
            };
            counted += 1;
            if options.dry_run {
                batch
                    .items
                    .push(result(&label, Some(name), "planned", None, None));
                continue;
            }
            std::fs::create_dir_all(options.output).map_err(|_| err("无法创建下载目录。"))?;
            if http.is_none() {
                http = Some(create_http()?);
            }
            match transfer(http.as_ref().unwrap(), url, options.output, &name).await {
                Ok(true) => batch.items.push(result(
                    &label,
                    Some(name.clone()),
                    "saved",
                    Some(options.output.join(name)),
                    None,
                )),
                Ok(false) => batch.items.push(result(
                    &label,
                    Some(name),
                    "skipped",
                    None,
                    Some("同名文件已存在，未覆盖。"),
                )),
                Err(e) => batch.items.push(result(
                    &label,
                    Some(name),
                    "failed",
                    None,
                    Some(&e.to_string()),
                )),
            }
        }
    }
    Ok(batch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn streams_complete_response_and_does_not_overwrite() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let serving = std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(&mut stream);
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
            }
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 7\r\nConnection: close\r\n\r\nfixture").unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let http = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        let u = Url::parse(&format!("http://{address}/sample")).unwrap();
        // Production passes only URLs that have already cleared attachment_url.
        assert!(
            transfer(&http, u.clone(), dir.path(), "test.pdf")
                .await
                .unwrap()
        );
        assert!(!transfer(&http, u, dir.path(), "test.pdf").await.unwrap());
        assert_eq!(
            std::fs::read(dir.path().join("test.pdf")).unwrap(),
            b"fixture"
        );
        serving.join().unwrap();
    }
    #[test]
    fn rejects_non_platform_hosts_and_userinfo() {
        assert!(attachment_url("https://downloadv5.ketangpai.com/path").is_some());
        assert!(attachment_url("https://ketangpai-test.oss.aliyuncs.com/path").is_some());
        assert!(
            attachment_url("https://ketangpai-live.oss-cn-beijing.aliyuncs.com/file").is_some()
        );
        for bad in [
            "https://127.0.0.1/a",
            "http://[::1]/a",
            "https://downloadv5.ketangpai.com.evil.test/a",
            "https://me@downloadv5.ketangpai.com/a",
            "https://downloadv5.ketangpai.com:8443/a",
            "file:///etc/passwd",
        ] {
            assert!(attachment_url(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn cleans_platform_paths_and_windows_devices() {
        assert_eq!(filename("../../CON.txt"), "_CON.txt");
        assert_eq!(filename("dir\\test:stream.pdf"), "test_stream.pdf");
        assert_eq!(filename(".."), "attachment.bin");
    }
}
