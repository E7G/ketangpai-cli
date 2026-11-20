use crate::client::Endpoint;
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use std::sync::OnceLock;

pub fn answer(endpoint: Endpoint, payload: &Value) -> Value {
    static NOW: OnceLock<chrono::DateTime<Utc>> = OnceLock::new();
    let now = *NOW.get_or_init(Utc::now);
    let hours = |offset: i64| (now + Duration::hours(offset)).timestamp();
    let page = payload["page"].as_u64().unwrap_or(1);
    let course = payload["courseid"].as_str().unwrap_or("");
    let dir = payload["dirid"].as_str().unwrap_or("0");
    let data = match endpoint {
        Endpoint::Profile => {
            json!({"uid":"demo-student", "username":"演示学生", "schoolname":"示例大学"})
        }
        Endpoint::Semesters => {
            json!({"semester":[{"semester":"2026-2027", "term":"1"}, {"semester":"2025-2026", "term":2}], "topcourses":[]})
        }
        Endpoint::Courses => match (payload["semester"].as_str(), payload["term"].as_str()) {
            (Some("2026-2027"), Some("1")) => json!([
                {"id":"demo-digital", "coursename":"数字电路", "classname":"01 班", "username":"李老师"},
                {"id":"demo-data", "coursename":"数据结构", "classname":"02 班", "username":"王老师"}
            ]),
            (Some("2025-2026"), Some("2")) => {
                json!([{"id":"demo-history", "coursename":"计算机导论", "username":"杨老师"}])
            }
            _ => json!([]),
        },
        Endpoint::Homework => json!({"title":"组合逻辑练习", "endtime":hours(15), "fullscore":100,
                                    "attachment":[{"name":"实验题目.pdf", "size":"96 KB"}]}),
        Endpoint::CourseContent => {
            let ty = &payload["contenttype"];
            let mut pages = 1;
            let list = if ty == 4 {
                match (course, page) {
                    ("demo-digital", 1) => {
                        pages = 2;
                        json!([
                            {"id":"row-1", "homeworkid":"hw-1", "title":"组合逻辑练习", "mstatus":"0", "endtime":hours(15), "timestate":"2"},
                            {"id":"row-2", "homeworkid":"hw-2", "title":"逻辑门实验", "mstatus":2, "grade":0, "endtime":hours(-72)},
                            {"id":"row-3", "homeworkid":"hw-3", "title":"卡诺图校正", "mstatus":3, "endtime":hours(-5), "timestate":"3"}
                        ])
                    }
                    ("demo-digital", 2) => {
                        pages = 2;
                        json!([{"id":"row-4", "homeworkid":"hw-4", "title":"时序电路预习", "mstatus":0, "endtime":0},
                               {"id":"row-5", "homeworkid":"hw-5", "title":"综合练习", "endtime":hours(4)}])
                    }
                    ("demo-data", 1) => {
                        json!([{"id":"row-6", "homeworkid":"hw-6", "title":"树与图练习", "mstatus":0, "endtime":hours(66) * 1000}])
                    }
                    _ => json!([]),
                }
            } else if ty == 6 && course == "demo-data" {
                json!([{"id":"test-1", "title":"第五章小测", "begintime":hours(48), "endtime":hours(50), "submit_state":1}])
            } else if ty == 7 && course == "demo-digital" {
                json!([{"id":"notice-1", "title":"期中复习安排", "releasetime":hours(-2)}])
            } else if course == "demo-digital" && (ty == 0 || ty == &json!(["2", "8"])) {
                match dir {
                    "0" => json!([{"id":"dir-1", "name":"第一章：逻辑基础", "item_type":"2"}]),
                    "dir-1" => json!([{"id":"mat-1", "title":"实验资料", "contenttype":"2",
                                       "attachment":[{"name":"逻辑门实验报告模板.pdf", "size":"512 KB"}]}]),
                    _ => json!([]),
                }
            } else {
                json!([])
            };
            json!({"list":list, "currentPage":page, "pageSize":50, "pageTotal":pages})
        }
    };
    json!({"status":1, "data":data})
}
