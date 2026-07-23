---
status: accepted
owner: tmdb-mteam-hub
last_verified: 2026-07-11
---

# 安全 Bootstrap

本 runbook 负责新安装、旧配置升级、容器暴露、管理 token 轮换和配置备份恢复。不要在命令、
截图、日志或工单中粘贴真实 token、Cookie、API key 或 qB 密码。

## 本机新安装

1. 将 `config.example.toml` 复制为 `config.toml`，权限设为 `0600`。
2. 保持 `listen_ip = "127.0.0.1"`、`management.allowed_origins = []`、
   `management.secure_cookie = false`。
3. 直接从同一台主机访问 `http://127.0.0.1:8787/`。空 token 只允许直接 TCP peer 为 loopback
   的一次性本地 bootstrap。
4. 在设置页生成并保存至少 24 字符的高熵管理 token。保存成功后浏览器立即用新 token 换取
   host-only、`HttpOnly`、`SameSite=Strict`、`Path=/api` Cookie；token 不写浏览器存储。
5. 保持 watcher 为 `enabled = false`、`dry_run = true`，完成 qB、分类和目录校验后再启用。

可以用密码管理器生成随机值。若使用可信终端生成，只把结果写进权限受限的配置文件，不要把
固定示例值复制为生产 token。

## 容器与 NAS

容器内部必须监听 `0.0.0.0` 才能接收端口映射，因此必须在启动前配置有效管理 token：

```toml
listen_ip = "0.0.0.0"

[management]
admin_token = "由密码管理器生成的至少24字符高熵值"
allowed_origins = []
secure_cookie = false
```

Compose 默认宿主映射是 `127.0.0.1:8787:8787`，供 NAS 本机反向代理使用。确需直接 LAN 访问
时，显式设置 `HOST_BIND_IP=0.0.0.0`，并先确认管理 token、NAS 防火墙和可信网段限制。不得把
清空 token 当作连通性修复。

反向代理负责 TLS。浏览器通过 HTTPS 访问时设置 `secure_cookie = true` 并重启；直接 HTTP
环境保持 `false`，否则浏览器不会回传 Cookie。

## 从旧 `0.0.0.0` 配置升级

升级前先停服务并复制整个配置和 state 目录。旧配置若显式监听 `0.0.0.0` 而没有有效 token，
新版本会 fail fast，不会静默改回 loopback：

1. 服务保持停止。
2. 先把 `config.toml` 复制到独立备份目录并验证备份非空。
3. 选择改回 `127.0.0.1`，或在继续非 loopback 监听前设置至少 24 字符高熵 token。
4. 确认文件权限为 `0600` 后启动。
5. 检查浏览器登录、CLI Bearer、`/healthz` 和 `/readyz`。

配置规范化写回会先创建 `config.toml.bak.<unix-seconds>`。这不替代升级前对整个 config/state
目录的独立备份。

## 登录与 CLI

浏览器只把 token POST 到 `/api/auth/login`，成功后依靠 HttpOnly Cookie。CLI 使用：

```text
Authorization: Bearer <admin_token>
```

不要把 token 放进 URL query、Compose、`.env`、shell history 或日志。除 auth 状态/登录/登出
以及最小 health/readiness 外，管理 API 都要求 Cookie 或 Bearer。

## Token 轮换与清除

在已认证的设置页输入新 token 并保存。响应成功后旧 Cookie 立即失效，页面会用新 token 重新
登录并清空输入框。其他浏览器和 CLI 必须改用新 token。

清除最后一个 token 只允许 loopback 监听；非 loopback 或容器监听会拒绝保存。即使是 loopback，
也只应在恢复过程中短暂使用 bootstrap，随后立即设置新 token。

## 忘记 Token

1. 停止服务，防止运行中配置协调器与手工编辑互相覆盖。
2. 保留当前 `config.toml` 和最近的 `config.toml.bak.*`，不要删除整个配置。
3. 若服务原来对 LAN 开放，先把 `listen_ip` 改为 `127.0.0.1` 或把 Compose 宿主绑定恢复为
   `127.0.0.1`。
4. 在 `config.toml` 中设置新的高熵 token，或仅在直接本机恢复时暂时清空。
5. 原子替换配置并恢复 `0600` 权限，再启动并重新登录。

不要启动服务后再覆盖运行中的配置文件，也不要同时运行两个会写同一配置的版本。

## CORS

SPA 与 API 同源时保持：

```toml
allowed_origins = []
```

此时没有 CORS allow header 是预期行为。只有明确的跨 origin 客户端才填写完整、精确的
`http://host[:port]` 或 `https://host[:port]`。不接受 `*`、`null`、userinfo、path、query、
fragment 或子域通配符。CORS 不替代鉴权、TLS 或 CSRF/反向代理隔离。

## Watcher dry-run 到实时模式

1. 新安装保持 `enabled = false`、`dry_run = true`。
2. 配好豆瓣、M-Team、已保存的 qB server ID、订阅分类和同文件系统媒体路径。
3. 首次启用 watcher 时确认提示，但继续保持 dry-run。
4. 检查轮询、候选和硬链接计划。dry-run 可以写审计与调度状态，但不得添加 qB torrent、创建
   目录或调用硬链接。
5. 只有计划和路径均正确后才关闭 dry-run。TV 自动订阅还要求有效 TMDB 凭据；其硬链接会写入
   `标题.年份/Season XX`，上线前需一并核对该目录布局。

紧急止损优先设置 `enabled = false`；dry-run 不是禁用的替代品。

## 恢复配置备份

1. 停止服务。
2. 选取内容和时间点已验证的 `config.toml.bak.*` 或独立部署备份。
3. 保留当前失败文件，使用同目录临时文件和原子 rename 替换 `config.toml`。
4. 把属主恢复为运行 UID/GID，并设置 `chmod 600 config.toml`。
5. 启动后检查配置 revision、登录、`/healthz` 和 `/readyz`。不要为让服务启动而删除 state。

SQLite 恢复和 schema 回滚必须同时遵循[备份与恢复](backup-restore.md)以及
[升级与回滚](upgrade-rollback.md)。

## 常见启动错误

- `management.admin_token` 太短：生成至少 24 字符高熵值。
- 非 loopback 且 token 为空：改为 loopback，或在开放监听前设置 token。
- unknown field：修正拼写或旧字段；解析器不会静默丢弃未知项。
- TOML line/column 错误：修复原文件，禁止删除后生成空配置。
- `secure_cookie = true` 但使用 HTTP：改用 HTTPS，或仅在可信本地 HTTP 环境关闭该选项。
- CORS origin 被拒绝：只填写不带 path 的精确 http(s) origin；同源保持空数组。
- 登录返回 `429`：同一直接 peer 的失败次数触发 15 分钟阻断；等待 `Retry-After`，确认 token
  后再尝试。反向代理后的多个用户可能共享同一 peer 预算。
- Cookie mutation 返回 `csrf_rejected`：确保 SPA 与 API 同源且请求由正常浏览器 fetch 发出；
  CLI 应使用 Bearer，而不是复制 Cookie 并伪造浏览器请求。
- 容器健康但页面从 NAS IP 不可达：默认宿主只绑定 loopback；优先配置本机反向代理，需要直接
  LAN 访问时才显式设置 `HOST_BIND_IP=0.0.0.0`。
