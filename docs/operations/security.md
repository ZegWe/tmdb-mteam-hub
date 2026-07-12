---
status: accepted
owner: tmdb-mteam-hub
last_verified: 2026-07-11
---

# 安全边界

## 当前部署假设

TMDB M-Team Hub 是单用户、自托管管理应用。它能读取第三方密钥、控制 qBittorrent，并在媒体
目录创建硬链接。当前管理 API 已受 token/Cookie 鉴权保护，CORS 也采用同源默认和精确
allowlist；应用仍不应直接暴露到互联网。

## 管理鉴权

- 除 `/api/auth/status`、`/api/auth/login` 和 `/api/auth/logout` 外，管理 API 均要求鉴权。
- `management.admin_token` 非空时至少 24 个字符；任何非 loopback 监听都必须配置。
- token 为空时，仅直接 TCP peer 为 loopback 的请求可使用本地 bootstrap。不要把这一模式用于
  同机反向代理后的共享访问，因为后端看到的代理 peer 可能仍是 loopback。
- 浏览器向 `/api/auth/login` 提交 token 后获得 host-only、`HttpOnly`、`SameSite=Strict`、
  `Path=/api` 的 Cookie；Cookie 中不保存明文 token。
- CLI 使用 `Authorization: Bearer <admin_token>`。不要把 token 放入 URL、shell history、日志
  或截图。
- 修改 token 会使现有登录 Cookie 失效。
- 登录失败按直接 peer 独立计数：5 分钟窗口内达到 5 次后阻断 15 分钟，并返回不含 token 的
  `429`/`Retry-After`。成功登录会清除该 peer 的失败预算。反向代理仍应提供入口级速率限制；
  若所有客户端共享一个代理 peer，应用内 limiter 也会按该共享地址聚合。24 字符是长度下限，
  不替代高熵随机生成。

## 网络

- 本地进程优先监听 `127.0.0.1`。
- Docker 容器内部监听 `0.0.0.0` 时必须配置管理 token，宿主端口仍应限制到可信网络。
- 同机反向代理可以把 Compose 端口改为 `127.0.0.1:8787:8787`。
- 远程访问优先使用 VPN，或使用 TLS 反向代理；HTTPS 部署设置
  `management.secure_cookie = true`。
- 同源部署保持 `management.allowed_origins = []`，后端不会发送 CORS 头。跨 origin 只允许配置
  中列出的精确 origin，不支持通配符或 `null`。
- CORS 不是访问控制替代品；即使 origin 获准，请求仍必须通过 Cookie 或 Bearer 鉴权。
- Cookie 鉴权的 POST/PUT 等 mutation 必须带浏览器生成的 `Sec-Fetch-Site: same-origin`；
  `same-site`、`cross-site` 或缺失值都会 fail closed。Bearer CLI 不依赖该浏览器 header。
  loopback bootstrap 也拒绝显式 `same-site`/`cross-site` mutation；缺少 Fetch Metadata 但带有
  `Origin` 的 mutation 同样拒绝，降低恶意网页访问 localhost 的风险。同一站点下仍不应让不
  可信子域与本应用共享可达入口；反向代理继续隔离 host 并拒绝非预期 Origin。CORS 本身不会
  阻止浏览器发送所有简单请求。

## 配置与秘密

- `config.toml`、临时文件和备份使用 `0600`。
- 不把管理 token、TMDB/M-Team key、豆瓣 Cookie、qB 密码写入日志、Issue 或截图。
- NAS 备份若离开设备，应加密。
- qB server 只配置受信地址，不接受来自普通请求的任意 URL 和密码。

## 容器权限

- 通过 `PUID`/`PGID` 使用具备最小目录权限的用户。
- 配置和 state 目录只允许应用用户访问。
- 媒体共享组只授予下载源读取与目标目录写入所需权限。
- 不要为了绕过权限错误给媒体根目录设置全局可写。

## 上线检查

- 端口没有映射到公网接口或路由器端口转发。
- 非 loopback 或代理部署已配置至少 24 字符的管理 token。
- 浏览器登录、CLI Bearer、反向代理/VPN 访问控制经过验证。
- HTTPS 部署已开启 `secure_cookie`；CORS allowlist 仅包含实际需要的精确 origin。
- `config.toml` 权限正确且不在版本库。
- 已完成配置与 state 备份。
- qB、下载目录和链接目标使用预期用户与同一文件系统。

目标安全模型见
[架构收敛 PRD](../superpowers/specs/2026-07-11-project-architecture-convergence-prd.md)和
[安全与配置计划](../superpowers/plans/2026-07-11-safety-configuration-automation.md)。
