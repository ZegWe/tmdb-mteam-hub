---
status: accepted
owner: tmdb-mteam-hub
last_verified: 2026-07-11
---

# 故障排查

## 首先收集信息

```bash
docker compose ps
docker compose logs --tail=200 tmdb-mteam-hub
docker compose config
```

记录当前镜像 tag、`PUID`/`PGID`、`MEDIA_ROOT`，但不要粘贴密钥或 Cookie。

## 页面无法访问

- Docker 中的 `config.toml` 必须使用 `listen_ip = "0.0.0.0"`，并配置至少 24 字符的
  `management.admin_token`；缺少 token 时服务会拒绝非 loopback 启动。
- 确认 Compose 映射 `8787:8787`，NAS 防火墙允许可信来源。
- 反向代理同机访问时，可以把宿主绑定限制为 `127.0.0.1:8787`。
- 如果首页 404，确认镜像由当前多阶段 Dockerfile 构建且 `/app/static/index.html` 存在。

先区分进程、状态存储和前端资源问题：

```bash
curl -i http://127.0.0.1:8787/healthz
curl -i http://127.0.0.1:8787/readyz
curl -i http://127.0.0.1:8787/
```

- healthz 失败或容器为 `unhealthy`：查看进程日志、监听地址和端口；镜像健康检查不依赖管理
  token。
- healthz 成功但 readyz 返回 503：探针在状态写入进行中会快速失败而不排队，先隔几秒重试；
  若持续失败，保留原始 `state/`，检查权限、残留 SQLite journal/WAL、数据库损坏或未来
  schema。探针只读且不会恢复、修复、转换或替换数据库。
- 两者成功但首页失败：检查镜像内 `/app/static/index.html` 和反向代理静态路由。

## API 返回 401

- 浏览器先向 `/api/auth/login` 提交管理 token；成功后应收到
  `tmdb_mteam_admin_session` Cookie。
- CLI 请求添加 `Authorization: Bearer <admin_token>`，不要把 token 放在 query 参数中。
- token 为空时只有直接 peer 为 loopback 的请求可以 bootstrap。经反向代理访问时应配置 token，
  不能依赖代理到后端的 loopback 连接绕过登录。
- 修改 `admin_token` 后旧 Cookie 会立即失效，需要重新登录。
- 若 HTTPS 部署已开启 `secure_cookie`，确认浏览器确实通过 HTTPS 访问；`Secure` Cookie 不会在
  HTTP 请求中回传。

可用以下请求确认当前状态，响应不会回显 token：

```bash
curl -i http://127.0.0.1:8787/api/auth/status
```

## 浏览器跨 origin 请求失败

- 同源部署应保持 `management.allowed_origins = []`；没有 CORS 响应头是预期行为。
- 跨 origin 时填写完整且精确的 origin，包括 scheme、host 和非默认端口，但不包含路径。
- 不支持 `*`、`null` 或 `https://*.example.com`。
- 修改 allowlist 后重启服务。Cookie 使用 `SameSite=Strict`，真正的跨站管理页面应优先改为
  同源反向代理，而不是放宽 Cookie 或 CORS。

## 配置解析失败

错误会包含配置路径及 TOML line/column。修复原文件，不要删除它让服务生成空默认配置。检查最近的
`config.toml.bak.*`，恢复前先保留当前失败文件。

## Permission denied

- 核对 `.env` 的 `PUID`/`PGID`。
- 检查 `config/`、`state/`、缓存和 `MEDIA_ROOT` 的属主。
- `config.toml` 应为应用用户所有且权限 `0600`。
- qB 与应用使用不同用户时，通过共享组授予最小权限。

## 硬链接出现 EXDEV

源和目标不在同一文件系统。确保只把共同媒体根挂载到 `/srv/media`，并把
`download_dir`、`link_target_dir` 都设置为该根下的子目录。两个独立 NAS volume 无法硬链接。

## 找不到 qB 下载文件

- 检查 qB 实际 content path。
- 检查分类的 `qb_save_dir_name`。
- 确认应用的 `download_dir` 是容器内 `/srv/media/...` 路径，而不是宿主路径或 qB 的另一套
  容器路径。

## 订阅状态为空或回退失败

- 确认 `SUBSCRIPTION_STATE_DIR=/data/state`。
- 不要把 `state/` 当缓存清理。
- 若升级后 schema 错误，按[备份与恢复](backup-restore.md)恢复匹配版本的数据和镜像。

## 构建问题

- 前端要求 Node.js 22.18 及兼容 npm。
- `npm run build` 应生成 `static/index.html`。
- Dockerfile 会自行运行 `npm ci` 和前端构建，不应复制宿主 `static/`。
- 离线构建需要预先缓存 Node、Rust、Debian 基础镜像和依赖。
