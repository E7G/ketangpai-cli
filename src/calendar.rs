use crate::service::Item;
use chrono::Utc;
use sha2::{Digest, Sha256};

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\\n")
        .chars()
        .filter(|c| !c.is_control())
        .collect()
}

fn fold(line: &str, out: &mut String) {
    let mut width = 0;
    for c in line.chars() {
        if width + c.len_utf8() > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += c.len_utf8();
    }
    out.push_str("\r\n");
}

pub fn export(items: &[Item]) -> String {
    let stamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    let mut out = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//ketangpai-cli//Homework deadlines//ZH\r\nCALSCALE:GREGORIAN\r\n".to_string();
    for item in items {
        let Some(due) = item.due_at else {
            continue;
        };
        let uid = Sha256::digest(format!("{}\0{}", item.course_id, item.id));
        let title = escape(&format!("{}：{} [截止]", item.course_title, item.title));
        let due = due.to_utc().format("%Y%m%dT%H%M%SZ");
        for line in [
            "BEGIN:VEVENT".into(),
            format!("UID:{uid:x}@ketangpai-cli"),
            format!("DTSTAMP:{stamp}"),
            format!("DTSTART:{due}"),
            format!("SUMMARY:{title}"),
            format!("URL:{}", item.url),
            "BEGIN:VALARM".into(),
            "TRIGGER:-PT1H".into(),
            "ACTION:DISPLAY".into(),
            format!("DESCRIPTION:{title}"),
            "END:VALARM".into(),
            "END:VEVENT".into(),
        ] {
            fold(&line, &mut out);
        }
    }
    out.push_str("END:VCALENDAR\r\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_chinese_without_splitting_utf8_or_exceeding_octet_limit() {
        let mut out = String::new();
        let line = format!("SUMMARY:{}", "这是用于测试的中文标题".repeat(15));
        fold(&line, &mut out);
        assert!(out.split("\r\n").all(|s| s.len() <= 75));
        assert_eq!(out.trim_end().replace("\r\n ", ""), line);
        assert_eq!(escape("a,b;c\\d\r\n下一行"), "a\\,b\\;c\\\\d\\n下一行");
    }
}
