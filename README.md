# 课堂派 CLI（Rust）

`ktp` 是课堂派网页**学生端只读**客户端。命令名参考 `xuexitong-cli`：用终端或 MCP 查课程、作业、测试、公告和资料；还可以把作业截止时间导入日历。程序不提交作业、不进入测试、不签到、不标记已读。

## Windows 快速开始

解压 Windows x64 包后，在解压目录打开 PowerShell。账号密码登录从 `0.2.0` 开始支持；已发布版本见 [Releases](https://github.com/E7G/ketangpai-cli/releases)。项目只提供命令行与 MCP，不包含 Qt 或桌面程序。

```powershell
.\ktp.exe --demo courses                    # 演示：无需登录，不联网
.\ktp.exe --demo hw                         # 演示：截止看板
.\ktp.exe --demo hw-info 数字电路 1
.\ktp.exe --help
```

### 账号密码登录（与学习通版相同的配置方式）

不配置任何文件也可以：执行 `login`，按提示输入本人的手机号（或邮箱）和密码。输入在终端中不可见。

```powershell
.\ktp.exe login
.\ktp.exe courses
.\ktp.exe hw --days 7
```

账号密码来源，优先级从高到低：

1. **MCP `login` 显式参数** `phone` 与 `password`（通常建议工具传空参数 `{}`，改用本机配置）。
2. **环境变量** `KETANGPAI_PHONE` 和 `KETANGPAI_PASSWORD`，两个变量一起设置。
3. **`config.json`**：将 `config.example.json` 复制为 `config.json`，填写 `phone`（手机号或邮箱）和 `password`，放在可执行文件旁。找不到时再查当前工作目录；也可用 `KETANGPAI_CONFIG` 指定路径。

同一个来源必须成对提供账号和密码，不会跨来源拼接。下面是配置文件格式（均为示例值），也兼容将这两个字段放在 `ketangpai` 节点里、使用 `email` 作为账号字段的配置。

```json
{
  "phone": "13800000000",
  "password": "your-password-here"
}
```

`ktp login` 默认保存会话 Token，`ktp login --no-save` 只验证。CLI 与 MCP 共用会话；配置了账号密码时也可直接运行 `ktp courses`：没有会话会自动登录，本地会话过期时会重登并重试当前查询一次。程序不会另行保存账号或密码；自行填写的 `config.json` 是明文文件，已由 `.gitignore` 排除。

会话默认存在 `%LOCALAPPDATA%\ketangpai-cli\session.token`；Linux/macOS 默认 `~/.config/ketangpai-cli/session.token`（尊重 `XDG_CONFIG_HOME`）。可用 `KETANGPAI_TOKEN_FILE` 指定文件。显式设置的 `KETANGPAI_TOKEN` 优先于存盘会话，过期时不会自动改用其他账号。

如果课堂派要求二次验证或扫码，请在[课堂派网页](https://www.ketangpai.com/#/main)完成登录。可从开发者工具 **网络 / Network** 中的 `semesterList` 或 `semesterCourseList` 请求复制 `Token` 请求头的值，然后运行 `ktp login --token` 导入。

## 常用命令

所有查询命令支持 `--json`。不指定学期时跟随课堂派网页，取接口返回的首个可选学期；查过去的学期先用 `semesters`，然后成对指定 `--semester` 与 `--term`。

| 命令 | 用途 |
| --- | --- |
| `ktp whoami` / `ktp semesters` | 查看基本账号信息 / 可选学期 |
| `ktp status --probe` | 查看会话和账号配置是否可用；去掉 `--probe` 不联网 |
| `ktp courses --semester 2025-2026 --term 2` | 查指定学期的课程 |
| `ktp courses --all` | 列出全部可选学期中的课程 |
| `ktp hw --days 7` | 看未来七天、已逾期、无截止和状态未知的作业；已提交/批改的不计入待办 |
| `ktp hw-ls 数字电路` | 查看该课作业及提交状态，支持课程 ID 或唯一关键词 |
| `ktp hw-info 数字电路 1` | 按作业 ID 或从 1 开始的列表序号查看详情 |
| `ktp notice --course 数字电路 --keyword 复习` | 搜课程公告标题，不读取通知中心私信 |
| `ktp notifications --unread --full` | 读取课堂派「通知中心」；按标题、正文或课程名筛选，不标记已读 |
| `ktp exam --course 数据结构` | 查看课程内测验，不是教务处统一考试 |
| `ktp chapters 数字电路` | 查看一层章节目录；用 `--directory ID` 下钻 |
| `ktp chapters 数字电路 --tree --todo` | 按章节展示活动状态和汇总 |
| `ktp progress` | 全学期课程进度总览，未完成多的排前 |
| `ktp progress 数字电路 --todo` | 查看章节活动进度；递归扫描，未知状态单列 |
| `ktp materials 数字电路 --directory ID --keyword 报告 --json` | 查资料名称、附件和网页链接 |
| `ktp materials 数字电路 --find 模板 --depth 3` | 递归搜索资料和附件名，显示所在目录 |
| `ktp download 数字电路 --keyword 报告 --dry-run` | 预览课程里老师明确允许下载的文件 |
| `ktp download 数字电路 --keyword 报告 --dir downloads` | 批量下载课程资料；已存在的同名文件不覆盖 |
| `ktp schedule --file academic.json` | 查看从学校教务导入的个人课表 |
| `ktp today` / `ktp next` / `ktp week 3 --grid` | 今天、下一节和第 3 教学周课表 |
| `ktp schedule-ics -o 课表.ics` | 导出教务课表，含上课前 30 分钟提醒 |
| `ktp school-exams --file academic.json` | 查看从学校教务导入的考试安排，与 `ktp exam` 区分 |
| `ktp pan --root D:/MyCloud` | 列出本机同步的云盘目录；传返回的目录 ID 下钻 |
| `ktp ics --days 14 -o 作业截止.ics` | 导出将到期的作业，含提前一小时的日历提醒；不覆盖文件 |
| `ktp watch --interval 30` | 前台终端每 30 分钟查询一次，七天截止看板变化才输出；`Ctrl+C` 退出 |
| `ktp watch --all-events --once --json` | 一次性输出截止、作业、测验、根目录资料和通知；去掉 `--once` 则报告轮询变化 |

时间按北京时间显示，日历使用 UTC。`--days` 仅限制未来分组。部分课程失败时，CLI 返回退出码 `3`，JSON 有 `errors`；导出作业日历时遇到课程查询失败则不生成不完整的文件。`chapters` 和 `materials` 只展示当前层；`progress` 与 `download` 支持递归。作业或活动缺少状态时，不猜为已完成或未完成。

通知正文只在课堂派返回时显示，默认最多 160 字；`--full` 上限 4,000 字，并在最多 20 条学校通知上补查正文。某些教学活动只有标题，详情仍需进入网页；工具不会标记已读。

### 章节与资料

- `progress` 汇总课堂派网页返回的互动课件、资料、作业和课程测验状态，不把该汇总等同于学校的成绩或官方必修任务点统计。没有章节的课程会尝试从普通活动标签汇总。可指定 `--directory ID`、`--depth 0–8` 与 `--todo`；全课程模式可用 `--max-courses` 控制范围。
- `download` 默认扫描两层子目录、最多尝试 20 个附件；可用 `--depth 0–8`、`--max-files 1–100` 或 `--material ID` 控制范围。单个附件上限 256 MiB。外链、未授权或不支持的下载地址会跳过，失败条目单列。`--demo download` 仅允许 `--dry-run`，不会写文件。

### 教务课表与学校考试

个人课表和学校考试**不直连学校教务系统**，需把教务导出或公布的安排整理成 `academic.example.json` 或 `academic.example.csv`（UTF-8）格式。`kind` 使用 `class` 或 `exam`，`start`、`end` 用 `YYYY-MM-DD HH:MM`（北京时间）或带偏移的 RFC 3339；`location`、`note` 可选。考试可以填写 `status` 与 `score`，未填写状态时仅按起止时间推算，不猜成绩。

选填 `semester_start` 后可查看第 N 教学周（以其所在周的周一为第一周）。重复课程把起止时间写成第一周对应星期的日期，再填 `repeat_weeks`：JSON 例如 `[1,3,5]`，CSV 例如 `1-8|10|12-16`。不用重复或周次筛选时，可以直接写每次课的真实日期。

```powershell
.\ktp.exe --demo schedule
.\ktp.exe week 1 --file academic.example.csv --grid
.\ktp.exe schedule-ics --file academic.json -o 课表.ics
.\ktp.exe school-exams --file academic.json -o 考试.ics
```

`schedule` 和 `school-exams` 都支持 `--from`、`--to`（含边界当日）、`--keyword`、`--json`、`-o` 日历导出。文件默认是当前目录的 `academic.json`，也可用 `KETANGPAI_ACADEMIC_FILE` 指定。不需要课堂派登录。不同学校的非标准导出需要先整理为模板，不会用课程测验替代统考。

### 个人云盘替代方案与无 GUI 巡检

课堂派学生端未查到可验证的个人云盘列表接口，教师备课区也不等同于学生个人文件。`pan` 仅读取通过 `--root` 或 `KETANGPAI_PAN_DIR` 配置的**本地同步目录**，返回名称、大小、更新时间和子目录 ID；不上传、不读取文件正文、不跟随目录内的符号链接或联接点。MCP 仅使用启动时配置的 `KETANGPAI_PAN_DIR`。

`watch --all-events` 在终端运行，不依赖 GUI。首轮建立基线，之后报告观察范围内的新增与状态变化；`--once --json` 可供本地调度器调用。课程资料巡检只扫根目录，不遍历整个资料库，也不会打开学习活动。

## MCP（stdio）

在 MCP 客户端中把本机 `ktp.exe` 注册为 stdio command，参数 `mcp`。通用 JSON 示例（替换绝对路径）：

```json
{
  "mcpServers": {
    "ketangpai": {
      "command": "C:/Tools/ketangpai/ktp.exe",
      "args": ["mcp"]
    }
  }
}
```

不需要在 MCP 配置里粘贴 Token。可以先执行同一系统账号下的 `ktp login`，也可以配置环境变量或 `config.json` 后直接查询。将参数换成 `["mcp", "--demo"]` 可离线测试。

| 能力 | MCP 工具 |
|---|---|
| 登录及课程 | `login`、`get_profile`、`list_semesters`、`list_courses` |
| 教务课表及考试 | `get_schedule`、`export_schedule_ics`、`list_school_exams` |
| 作业、测验及进度 | `list_homework`、`get_homework_detail`、`get_homework_deadlines`、`list_exams`（课程测验）、`list_chapters`、`get_chapter_progress`、`get_progress_overview` |
| 通知 | `list_notifications`（通知中心）、`list_notices`（课程公告） |
| 资料与文件 | `list_materials`、`find_materials`、`plan_material_download`、`download_material`、`list_pan_files`（本地同步目录） |

教务查询建议给 MCP 进程设定 `KETANGPAI_ACADEMIC_FILE`。`download_material` 和 `export_schedule_ics` 默认不写文件，只有本机操作者预设了**已存在的** `KETANGPAI_MCP_OUTPUT_DIR` 才允许保存；工具调用者不能传任意目录，导出文件不得覆盖同名文件。

## 从源码构建

本次构建使用 Rust 1.98.0。Windows 使用 MSVC 与 Windows SDK；Linux 使用系统 OpenSSL（如 Debian/Ubuntu 的 `libssl-dev` 和 `pkg-config`）。不需要 Python 或 Node。

```text
cargo build --release --locked
cargo test --locked
cargo run --locked -- --demo hw --json
```

或 `cargo install --path . --locked` 安装 `ktp` 命令。Lockfile 随源码提供。

## 来源和范围

- [DanMo661/xuexitong-cli](https://github.com/DanMo661/xuexitong-cli/tree/74b843ad4a3439a6ee5ba33f6a3288d9be6ed990)：参考其只读定位、CLI 命令和 MCP 工具设计；不移植其 Python 代码。
- [men-shen/auto-homework-collector](https://github.com/men-shen/auto-homework-collector/blob/7c266cb7f242c1b9f89a81a417539e7aa21c26a7/crawler/crawler.py)：交叉核对课堂派课程与作业查询；请求形状与内容类型还参考了[课堂派公开网页前端](https://www.ketangpai.com/)。这些是网页使用的接口，不等同于有兼容性保证的开放 API。
- MCP 使用 [官方 Rust SDK](https://github.com/modelcontextprotocol/rust-sdk)。

机构迁移版、个性化字段和网页接口变化可能需要适配。与学习通参考项目不同，课堂派版的教务课表与统考来自本地导入，云盘来自本地同步目录，没有把教师备课区当成学生云盘。项目不包含图形悬浮窗、系统托盘或 Qt 依赖。
