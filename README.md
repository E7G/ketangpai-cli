# 课堂派 CLI（Rust）

`ktp` 是课堂派网页**学生端只读**客户端。命令名参考 `xuexitong-cli`：用终端或 MCP 查课程、作业、测试、公告和资料；还可以把作业截止时间导入日历。程序不提交作业、不进入测试、不签到、不标记已读。

## Windows 快速开始

从 [Releases](https://github.com/E7G/ketangpai-cli/releases/latest) 下载 Windows x64 包，解压后在该目录打开 PowerShell：

```powershell
.\ktp.exe --demo courses                    # 演示：无需登录，不联网
.\ktp.exe --demo hw                         # 演示：截止看板
.\ktp.exe --demo hw-info 数字电路 1
.\ktp.exe --help
```

### 查询本人账号

1. 在 [课堂派网页](https://www.ketangpai.com/#/main) 正常登录本人账号。
2. 打开开发者工具 `F12` → **网络 / Network**，刷新「我的课堂」。选中 `semesterList` 或 `semesterCourseList` 请求，在 **Request Headers / 请求标头** 中复制 `Token` 的值。不要复制整行，也不要把它发到聊天或提交到 Git。
3. 在 PowerShell 执行下面的命令，将 Token 粘贴到**不可见输入提示**，验证成功后保存到本机。

```powershell
.\ktp.exe login --save
.\ktp.exe courses
.\ktp.exe hw --days 7
```

不带 `--save` 时只验证，不保存。默认文件是 `%LOCALAPPDATA%\ketangpai-cli\session.token`（当前用户目录，明文，删除即可清除）；Linux/macOS 默认 `~/.config/ketangpai-cli/session.token`，也尊重 `XDG_CONFIG_HOME`。程序优先读非空环境变量 `KETANGPAI_TOKEN`，否则读该文件；可用 `KETANGPAI_TOKEN_FILE` 指定自己的文件。环境变量有旧值时，先清除它再使用新保存的 Token。Token 过期后在网页重新登录再录入。

## 常用命令

所有查询命令支持 `--json`。不指定学期时跟随课堂派网页，取接口返回的首个可选学期；查过去的学期先用 `semesters`，然后成对指定 `--semester` 与 `--term`。

| 命令 | 用途 |
| --- | --- |
| `ktp whoami` / `ktp semesters` | 查看基本账号信息 / 可选学期 |
| `ktp courses --semester 2025-2026 --term 2` | 查指定学期的课程 |
| `ktp hw --days 7` | 看未来七天、已逾期、无截止和状态未知的作业；已提交/批改的不计入待办 |
| `ktp hw-ls 数字电路` | 查看该课作业及提交状态，支持课程 ID 或唯一关键词 |
| `ktp hw-info 数字电路 1` | 按作业 ID 或从 1 开始的列表序号查看详情 |
| `ktp notice --course 数字电路 --keyword 复习` | 搜课程公告标题，不读取通知中心私信 |
| `ktp exam --course 数据结构` | 查看课程内测验，不是教务处统一考试 |
| `ktp chapters 数字电路` | 查看一层章节目录；用 `--directory ID` 下钻 |
| `ktp materials 数字电路 --directory ID --keyword 报告 --json` | 查资料名称、附件和网页链接 |
| `ktp ics --days 14 -o 作业截止.ics` | 导出将到期的作业，含提前一小时的日历提醒；不覆盖文件 |
| `ktp watch --interval 30` | 前台终端每 30 分钟查询一次，七天截止看板变化才输出；`Ctrl+C` 退出 |

时间按北京时间显示，日历使用 UTC。`--days` 仅限制未来分组。部分课程失败时，CLI 返回退出码 `3`，JSON 有 `errors`；导出日历时遇到课程查询失败则不生成不完整的文件。文件夹功能只查询当前层，不递归。作业缺少状态时不会当作已提交或未交。资料下载请从返回的网页链接进入。

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

不需要在 MCP 配置里粘贴 Token。先执行同一系统账号下的 `ktp login --save`。也可以先把参数换成 `["mcp", "--demo"]` 测试。服务器提供 `get_profile`、`list_semesters`、`list_courses`、`list_homework`、`get_homework_detail`、`get_homework_deadlines`、`list_notices`、`list_exams`、`list_chapters`、`list_materials`。示例：让客户端「查未来 7 天要交的作业，按截止时间排序」。

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

已离线验证 CLI、示例响应解析、截止分组、日历导出和 MCP stdio 调用。真实学生账号验证了基础资料、学期与课程、作业列表与详情、截止看板和资料列表；公告、测验和章节请求成功，但本次样本没有返回记录。机构迁移版、个性化字段和网页接口变化可能需要适配。此版不包含图形悬浮窗、教务课表、任务点进度、个人云盘或文件下载。
