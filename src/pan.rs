use crate::client::{Result, err};
use serde::Serialize;
use std::{
    env, fs,
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

const LIMIT: usize = 1000;

#[derive(Serialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub is_folder: bool,
    pub bytes: Option<u64>,
    pub modified_at: Option<chrono::DateTime<chrono::FixedOffset>>,
}

#[derive(Serialize)]
pub struct Listing {
    pub source: &'static str,
    pub folder: String,
    pub items: Vec<Entry>,
    pub limited: bool,
}

fn link_or_reparse(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    meta.file_type().is_symlink()
}

fn folder_components(value: Option<&str>) -> Result<Vec<&str>> {
    let value = value.unwrap_or_default();
    if value.is_empty() {
        return Ok(vec![]);
    }
    let parts: Vec<_> = value.split('/').collect();
    if parts.len() > 32
        || parts.iter().any(|p| {
            p.ends_with(['.', ' '])
                || p.contains(['\\', ':'])
                || p.chars().any(char::is_control)
                || !matches!(
                    Path::new(p).components().collect::<Vec<_>>().as_slice(),
                    [Component::Normal(_)]
                )
        })
    {
        return Err(err(
            "云盘目录 ID 必须是列表返回的相对目录，不能跨出配置的目录。",
        ));
    }
    Ok(parts)
}

fn demo_entries(folder: &str) -> Vec<Entry> {
    let entry = |id: &str, name: &str, is_folder| Entry {
        id: id.into(),
        name: name.into(),
        is_folder,
        bytes: (!is_folder).then_some(3456),
        modified_at: None,
    };
    match folder {
        "" => vec![entry("示例资料", "示例资料", true)],
        "示例资料" => vec![entry("示例资料/实验笔记.pdf", "实验笔记.pdf", false)],
        _ => vec![],
    }
}

pub fn list(root: Option<&Path>, folder: Option<&str>, demo: bool) -> Result<Listing> {
    let parts = folder_components(folder)?;
    let folder = parts.join("/");
    if demo {
        return Ok(Listing {
            source: "demo",
            items: demo_entries(&folder),
            folder,
            limited: false,
        });
    }
    let root = root.map(Path::to_path_buf).or_else(|| env::var_os("KETANGPAI_PAN_DIR").filter(|p| !p.is_empty()).map(PathBuf::from))
        .ok_or_else(|| err("未配置本地云盘目录。课堂派学生端没有已验证的个人云盘接口；请用 --root 或 KETANGPAI_PAN_DIR 指定你自己的同步目录。"))?;
    let base = root
        .canonicalize()
        .map_err(|_| err("无法打开配置的云盘目录。"))?;
    let mut sub = base.clone();
    for part in parts {
        sub.push(part);
        let meta = sub
            .symlink_metadata()
            .map_err(|_| err("云盘目录不存在或无法读取。"))?;
        if !meta.is_dir() || link_or_reparse(&meta) {
            return Err(err("云盘不跟随符号链接、目录联接或非目录条目。"));
        }
    }
    let sub = sub.canonicalize().map_err(|_| err("云盘目录无法读取。"))?;
    if !sub.starts_with(&base) {
        return Err(err("云盘目录超出配置范围。"));
    }
    let children = fs::read_dir(sub).map_err(|_| err("无法列出云盘目录。"))?;
    let mut items = vec![];
    let mut limited = false;
    for (n, child) in children.enumerate() {
        if n >= LIMIT {
            limited = true;
            break;
        }
        let child = child.map_err(|_| err("读取云盘目录时发生错误。"))?;
        let meta =
            fs::symlink_metadata(child.path()).map_err(|_| err("读取云盘条目信息时发生错误。"))?;
        if link_or_reparse(&meta) || (!meta.is_dir() && !meta.is_file()) {
            continue;
        }
        let Ok(name) = child.file_name().into_string() else {
            continue;
        };
        // Do not emit names that cannot round-trip as a portable folder ID.
        if folder_components(Some(&name)).is_err() || name.contains('/') {
            continue;
        }
        let id = if folder.is_empty() {
            name.clone()
        } else {
            format!("{folder}/{name}")
        };
        let modified_at = meta
            .modified()
            .ok()
            .filter(|t| t.duration_since(SystemTime::UNIX_EPOCH).is_ok())
            .map(|time| {
                chrono::DateTime::<chrono::Utc>::from(time)
                    .with_timezone(&crate::service::shanghai())
            });
        items.push(Entry {
            id,
            name,
            is_folder: meta.is_dir(),
            bytes: meta.is_file().then_some(meta.len()),
            modified_at,
        });
    }
    items.sort_by(|a, b| {
        (!a.is_folder, a.name.to_lowercase(), &a.name).cmp(&(
            !b.is_folder,
            b.name.to_lowercase(),
            &b.name,
        ))
    });
    Ok(Listing {
        source: "local_sync_directory",
        folder,
        items,
        limited,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lists_configured_root_and_rejects_traversal() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("课程")).unwrap();
        fs::write(root.path().join("课程/notes.pdf"), b"data").unwrap();
        let listing = list(Some(root.path()), Some("课程"), false).unwrap();
        assert_eq!(listing.items[0].id, "课程/notes.pdf");
        assert_eq!(listing.items[0].bytes, Some(4));
        for bad in [
            "../",
            "课程/../",
            "/课程",
            "C:relative",
            "课程\\..",
            "课程/.. ",
            "课程//x",
        ] {
            assert!(list(Some(root.path()), Some(bad), false).is_err(), "{bad}");
        }
    }
}
