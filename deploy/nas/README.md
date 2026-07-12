# NAS Docker Compose 部署

本文以群晖常见的 `/volume1` 路径为例。应用监听端口为 `8787`；Compose 默认只把它绑定到
宿主 `127.0.0.1`。不要直接将该端口暴露到互联网，远程访问应通过受控的反向代理或 VPN。

## 目录规划

部署目录保存配置、订阅状态和可重建缓存：

```text
/volume1/docker/tmdb-mteam-hub/
  docker-compose.yml
  .env
  config/
    config.toml
  state/
  cache/
    tmdb/
    douban/
```

媒体使用一个共享根目录挂入容器：

```text
/volume1/media/
  downloads/
    movies/
    tv/
  library/
    movies/
    tv/
```

创建目录：

```bash
mkdir -p /volume1/docker/tmdb-mteam-hub/config
mkdir -p /volume1/docker/tmdb-mteam-hub/state
mkdir -p /volume1/docker/tmdb-mteam-hub/cache/tmdb
mkdir -p /volume1/docker/tmdb-mteam-hub/cache/douban
mkdir -p /volume1/media/downloads /volume1/media/library
```

将仓库中的 `deploy/nas/docker-compose.yml` 放到部署目录，并将 `config.example.toml` 复制为
`config/config.toml` 后填写实际配置。Docker 容器内必须设置 `listen_ip = "0.0.0.0"`，否则
服务只监听容器自身的 loopback，宿主端口映射无法访问：

```toml
listen_ip = "0.0.0.0"
listen_port = 8787
```

非 loopback 监听还必须配置至少 24 字符的管理 token，否则服务会拒绝启动。在所有顶层 API
key 和 Cookie 字段之后添加：

```toml
[management]
admin_token = "请替换为至少24字符的高熵随机值"
allowed_origins = []
secure_cookie = false
```

不要再把顶层 `tmdb_api_key`、`mteam_api_key` 或 `douban_cookie` 写在 `[management]` 后面；TOML
会把它们解释为 management 字段。

可以使用密码管理器生成随机值，或在可信终端运行 `openssl rand -hex 24`。不要把 token 写进
Compose、`.env`、命令行参数或部署截图；它应只保存在权限为 `0600` 的 `config.toml` 中。

## 管理访问

默认 Compose 映射为 `127.0.0.1:8787:8787`，适合 NAS 本机反向代理。若确认 NAS 防火墙、管理
token 和可信网段策略均已就绪，并且确实需要直接通过 LAN 访问，可在 `.env` 显式设置：

```dotenv
HOST_BIND_IP=0.0.0.0
```

这会扩大宿主监听面，不会替代应用鉴权。不要为了方便而清空 `admin_token`；容器内部的
`listen_ip = "0.0.0.0"` 本身也要求有效 token。

- 通过反向代理地址或显式开放后的 `http://NAS_IP:8787/` 访问时，浏览器通过
  `/api/auth/login` 登录并获得 host-only、
  `HttpOnly`、`SameSite=Strict`、`Path=/api` 的会话 Cookie。
- CLI 使用 `Authorization: Bearer <admin_token>`。
- SPA 与 API 同源时保持 `allowed_origins = []`。只有管理页面确实来自另一 origin 时才填写精确
  `http(s)` origin，不使用通配符。
- 通过 HTTPS 反向代理访问时，把 `secure_cookie` 改为 `true` 并重启服务；直接 HTTP 保持
  `false`。
- 即使反向代理只通过 loopback 连接后端，也必须配置 token，因为无 token bootstrap 只适合
  直接本机开发，不适合代理后的共享访问。

## 镜像版本与运行用户

Compose 会自动读取同目录的 `.env`。建议固定发布版本，不要长期依赖可变的 `latest`：

```dotenv
TMDB_MTEAM_HUB_TAG=v0.1.0
MEDIA_ROOT=/volume1/media
PUID=1026
PGID=100
LOG_MAX_SIZE=10m
LOG_MAX_FILE=3
HEALTHCHECK_URL=http://127.0.0.1:8787/healthz
HOST_BIND_IP=127.0.0.1
CACHE_CLEANUP_INTERVAL_SECS=21600
OPERATION_LOG_RETENTION_DAYS=90
OPERATION_LOG_MAX_ROWS_PER_ACCOUNT=10000
```

- `TMDB_MTEAM_HUB_TAG` 可以使用发布版本或 `sha-...` 镜像标签；未设置时才使用 `latest`。
- `MEDIA_ROOT` 是下载目录和媒体库所在的共同宿主目录。
- `PUID`、`PGID` 是容器进程使用的用户和组；未设置时保持现有的 `0:0` 行为。
- `LOG_MAX_SIZE`、`LOG_MAX_FILE` 控制 Docker `json-file` 日志轮转。
- `HEALTHCHECK_URL` 默认匹配容器内的 `127.0.0.1:8787`。若显式修改应用监听端口或只监听
  IPv6，必须同步修改该 URL 与 Compose 端口映射，否则服务虽然启动，容器状态仍会是
  `unhealthy`。
- `CACHE_CLEANUP_INTERVAL_SECS` 控制过期 JSON 缓存清理周期；`0` 表示只在读取和启动检查时
  清理。
- `OPERATION_LOG_RETENTION_DAYS` 和 `OPERATION_LOG_MAX_ROWS_PER_ACCOUNT` 分别限制每个账号的
  操作日志年龄和条数；对应值为 `0` 时关闭该项限制。

使用非 root UID/GID 时，该用户必须能够读写 `config`、`state`、缓存目录和共享媒体目录。例如：

```bash
cd /volume1/docker/tmdb-mteam-hub
chown -R 1026:100 config state cache
chmod 700 config state
chmod 600 config/config.toml
```

qBittorrent 与本应用最好使用同一个组，并确保该组可以读取下载文件、在媒体库目录创建目录和硬链接。

## 硬链接路径

Compose 将整个 `MEDIA_ROOT` 挂载为容器内的 `/srv/media`。应用中的订阅分类应使用容器路径，例如：

```toml
download_dir = "/srv/media/downloads/movies"
link_target_dir = "/srv/media/library/movies"
```

硬链接要求源文件与目标目录位于同一个文件系统。不要把下载目录和媒体库分别从不同 NAS volume 挂入容器；即使两个目录在容器中都可见，跨文件系统仍会返回 `EXDEV`。使用一个共同根目录挂载还能避免多个 bind mount 造成路径和设备边界不一致。

qBittorrent 可以使用不同的容器内路径，但应用配置的 `download_dir` 必须指向本容器实际可见的下载目录，并与 qB 的保存目录结构对应。

## 数据持久性

- `config/`：持久配置和密钥，必须备份并限制读取权限。
- `state/subscriptions.sqlite`：当前订阅状态和操作日志的唯一权威库，必须备份。遗留
  `state/wanted.sqlite` 不会被当前版本读取或转换。
- `cache/tmdb/`、`cache/douban/`：可重建缓存，可以不纳入备份。
- `MEDIA_ROOT`：下载与媒体文件，由现有 NAS 媒体备份策略负责。

升级前建议停止容器，并只备份当前版本恢复所需的 `config.toml` 与
`subscriptions.sqlite`：

```bash
cd /volume1/docker/tmdb-mteam-hub
docker compose stop
backup_dir="backups/$(date +%Y%m%d-%H%M%S)"
umask 077
mkdir -p "$backup_dir/config" "$backup_dir/state"
test ! -e state/subscriptions.sqlite-wal
test ! -e state/subscriptions.sqlite-shm
test ! -e state/subscriptions.sqlite-journal
cp -p config/config.toml "$backup_dir/config/config.toml"
cp -p state/subscriptions.sqlite "$backup_dir/state/subscriptions.sqlite"
docker compose start
```

旧 `wanted.sqlite`、`wanted_*.json` 和可重建缓存不进入当前版本的恢复备份。完整步骤和 sidecar
检查失败时的处理见[备份与恢复](../../docs/operations/backup-restore.md)。

回滚到旧镜像时，数据库 schema 也可能需要恢复到该版本升级前的备份。

## 首次启用自动化

新安装保持：

```toml
[subscription_watcher]
enabled = false
dry_run = true
```

先完成登录、qB 连接测试、订阅分类和 `/srv/media` 路径检查，再在设置页启用 watcher。第一次
从关闭切换为启用会要求确认。先保留 dry-run，检查候选和硬链接计划；确认不会选错种子或目录后
再关闭 dry-run。dry-run 允许只读上游访问并写入审计/调度计划，但不会添加 qB torrent，也不会
创建目录或硬链接。

## 启动与更新

首次启动：

```bash
cd /volume1/docker/tmdb-mteam-hub
docker compose pull
docker compose up -d
docker compose ps
docker compose logs -f --tail=100
```

`docker compose ps` 应在启动后显示容器为 `healthy`。镜像健康检查只请求无鉴权的
`/healthz`，不会读取配置密钥或修改订阅数据库；需要单独确认状态存储可读时可请求
`/readyz`。如果 readyz 返回 503，先保留原始 `state/`，再检查 SQLite 损坏或 schema 版本不兼容，
不要通过删除数据库来让探针变绿。

更新时先按上面的停机流程备份两个权威文件，再修改 `.env` 中的 `TMDB_MTEAM_HUB_TAG`：

```bash
docker compose pull
docker compose up -d
```

通过配置的反向代理地址访问；只有显式设置 `HOST_BIND_IP=0.0.0.0` 后才直接访问
`http://NAS_IP:8787/`。使用管理 token 登录。若容器因权限问题退出，先查看
`docker compose logs`，再核对 `.env` 中的 UID/GID 与宿主目录属主。

完整的安全 bootstrap、token 轮换和忘记 token 恢复流程见
[安全 Bootstrap](../../docs/operations/security-bootstrap.md)。
