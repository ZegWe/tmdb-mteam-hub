---
status: accepted
owner: tmdb-mteam-hub
last_verified: 2026-07-12
---

# 配置说明

## 创建配置

本地开发：

```bash
cp config.example.toml config.toml
```

默认路径是启动进程时的当前目录，也可以通过 `CONFIG_PATH` 指定绝对路径。配置包含 API key、
豆瓣 Cookie 和 qBittorrent 密码，必须限制为仅运行用户可读。

## 监听地址

新本地配置默认监听 `127.0.0.1:8787`。Docker 端口映射要求容器内服务监听所有容器接口，
因此 NAS 的 `config/config.toml` 应显式设置 `0.0.0.0`：

```toml
listen_ip = "0.0.0.0"
listen_port = 8787
```

同时，在 `tmdb_api_key`、`mteam_api_key`、`douban_cookie` 等顶层字段之后添加管理表：

```toml
[management]
admin_token = "请替换为至少24字符的高熵随机值"
allowed_origins = []
secure_cookie = false
```

TOML 进入 `[management]` 后，后续普通 key 都属于该表，因此不要把顶层 API key 或 Cookie 写在
它后面。

非 loopback 监听而未配置 token 时，服务会拒绝启动。这只控制容器内部监听；宿主暴露范围仍应
通过 Compose 的 `ports`、NAS 防火墙、VPN 或反向代理控制。

## 管理面鉴权

`management.admin_token` 非空时至少需要 24 个字符。配置为空仅适用于直接本机开发：只有请求
的直接 TCP peer 是 loopback 地址时，后端才允许无 token bootstrap。经同机反向代理访问时，
后端看到的 peer 也可能是 loopback，因此代理后的共享访问同样必须配置 token。

浏览器登录：

```bash
curl -i http://127.0.0.1:8787/api/auth/login \
  -H 'Content-Type: application/json' \
  --data '{"token":"你的管理 token"}'
```

成功响应设置名为 `tmdb_mteam_admin_session` 的 host-only Cookie，不设置 `Domain`，并带有
`Path=/api`、`HttpOnly` 和 `SameSite=Strict`。修改 `admin_token` 会立即使旧 Cookie 失效。

CLI 和自动化脚本不使用 Cookie，直接发送：

```text
Authorization: Bearer <admin_token>
```

管理字段通过配置文件加载；修改 `admin_token`、`allowed_origins` 或 `secure_cookie` 后应重启
服务。

## CORS 与 HTTPS

`management.allowed_origins = []` 是推荐的同源默认值：后端不会发送 CORS 响应头。确需跨
origin 访问时，只能填写精确的完整 `http://` 或 `https://` origin，例如：

```toml
[management]
admin_token = "请替换为至少24字符的高熵随机值"
allowed_origins = ["https://media-admin.example:8443"]
secure_cookie = true
```

origin 不得包含 userinfo、路径、query、fragment、通配符或 `null`。allowlist 只控制浏览器
CORS，不替代鉴权和 TLS。通过 HTTPS 访问时必须开启 `secure_cookie`；直接使用本地 HTTP 时
保持 `false`，否则浏览器不会回传带 `Secure` 的 Cookie。

## 路径环境变量

| 环境变量                             | 用途                       |
| ------------------------------------ | -------------------------- |
| `CONFIG_PATH`                        | 配置文件                   |
| `TMDB_CACHE_DIR`                     | TMDB JSON 缓存             |
| `DOUBAN_CACHE_DIR`                   | 豆瓣 JSON 缓存             |
| `SUBSCRIPTION_STATE_DIR`             | 订阅 SQLite 与操作日志目录 |
| `TMDB_CACHE_TTL_SECS`                | TMDB 缓存 TTL              |
| `DOUBAN_CACHE_TTL_SECS`              | 豆瓣缓存 TTL               |
| `CACHE_CLEANUP_INTERVAL_SECS`        | 过期 JSON 缓存清理周期     |
| `OPERATION_LOG_RETENTION_DAYS`       | 每账号操作日志保留天数     |
| `OPERATION_LOG_MAX_ROWS_PER_ACCOUNT` | 每账号操作日志最大条数     |
| `EXECUTION_LEASE_TTL_SECS`           | Execution attempt lease    |
| `EXECUTION_BATCH_SIZE`               | 每批最多 claim 数量        |
| `EXECUTION_CONCURRENCY`              | 每批最大并发 effects       |
| `EXECUTION_IDLE_INTERVAL_SECS`       | 队列空闲检查间隔           |
| `EXECUTION_JITTER_SECS`              | worker 抖动上限            |
| `FILESYSTEM_EFFECT_CONCURRENCY`      | 文件系统 effect 并发上限   |

Docker Compose 已分别设置 `/data/config`、`/data/state` 和两个缓存目录。
`TMDB_CACHE_DIR`、`DOUBAN_CACHE_DIR` 与 `SUBSCRIPTION_STATE_DIR` 不得相同或互为父子目录；
启动时会解析真实路径并拒绝重叠配置，避免缓存清理进入权威订阅状态目录。

## TOML 校验与写回

- 格式错误、未知字段或非法监听地址应直接报告错误并停止启动。
- 不要在服务运行时同时从多个编辑器和设置页面修改同一文件。
- 配置规范化若需要写回，应保留时间戳备份。
- `config.toml`、临时文件和备份均应保持 `0600` 权限。

服务启动失败时先查看错误中的文件路径、line 和 column，不要删除原文件让服务生成空配置。

## qBittorrent 与订阅分类

每个 qB server 应有稳定 ID。每个订阅分类通过 `qb_server_id` 绑定目标服务器，并配置：

- `qb_category`
- `qb_save_dir_name`
- `download_dir`
- `link_target_dir`

Docker 中的 `download_dir` 和 `link_target_dir` 必须使用 `/srv/media` 下的容器路径。qB 可以
使用不同容器路径，但目录结构必须能映射到应用看到的共享媒体根。

## 自动化启用

新安装默认 `subscription_watcher.enabled = false`、`dry_run = true`。关闭状态不 Poll、不 claim；
从关闭切换为启用时，API 和设置页都要求显式确认。dry-run 只允许 Poll 刷新权威订阅记录，不会
claim Execution、请求 M-Team 下载 token、添加 torrent 或创建目录/硬链接。确认分类、qB 和媒体
路径配置后再显式关闭 dry-run 进入实时模式。

相关实施状态见
[安全与配置计划](../superpowers/plans/2026-07-11-safety-configuration-automation.md)。
