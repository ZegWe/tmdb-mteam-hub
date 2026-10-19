# TMDB M-Team Hub

TMDB M-Team Hub 是一个面向个人媒体库的单体应用：统一完成 TMDB/豆瓣检索、M-Team
种子匹配、qBittorrent 推送、订阅状态跟踪和同文件系统硬链接。仓库由一个 Rust/Axum
服务、一个 Vue SPA 和一个 SQLite 状态库组成。

项目正在进行模块边界、安全默认值、存储和前端契约的收敛。当前权威方向见
[架构总览](docs/architecture/overview.md)和
[架构收敛 PRD](docs/superpowers/specs/2026-07-11-project-architecture-convergence-prd.md)。

## 快速开始

支持的工具链固定为 Node.js 22.18.0、npm 11.17.0 和 Rust 1.96。权威版本分别记录在
`package.json`、`Cargo.toml` 和 CI workflow 中。

```bash
npm ci
cp config.example.toml config.toml
cargo run
```

另开一个终端启动前端开发服务器：

```bash
npm run dev
```

Vite 默认监听 `5173`，并把 `/api` 代理到 `127.0.0.1:8787`。`config.toml` 包含密钥和
Cookie，不要提交到版本库。

当前版本只使用 `cache/subscriptions/subscriptions.sqlite`（容器内为
`/data/state/subscriptions.sqlite`）。服务不会枚举、探测或打开旧 `wanted.sqlite` 和
`wanted_*.json`，也不会读取、转换、修改或删除它们；项目不提供旧状态迁移。

生产式本地运行需要先生成静态资源：

```bash
npm run build
cargo run --release
```

## 管理访问

进入影视详情不会自动搜索种子；选择检索方式或季数后，点击“搜索种子”才会发起请求。
订阅详情中的历史候选默认折叠，点击“查找其他种子”才会进行新搜索。候选标题可打开 M-Team
原站详情，并展示大小、做种人数、下载中人数和发布时间；未提供的数据以“—”显示。
已选中、下载中和已完成的任务，也可以在“下载”区域点击种子标题打开原站详情。

电影和 TV 订阅仅对豆瓣 ID 或 IMDb ID 搜索匹配的种子自动下载。名字搜索的候选进入
“待确认种子”状态，需要在订阅详情核对标题、年份和季集数，点击“选中并下载”后才会
安排下载。确认只授权所选种子，不授权同名的其他结果或后续集数；“重跑任务”不会替代确认。

即使已通过豆瓣/IMDb ID 自动选中种子，也可以在订阅详情选择“名称”“豆瓣 ID”或“IMDb ID”，
点击“查找其他种子”补充候选。手动 ID 搜索结果同样需要确认，搜索保留当前下载；选择其他候选并确认“清理旧任务并换种”后，后台
先记录旧文件清单，再删除本订阅的旧 qB 任务及下载内容、已验证的硬链接和空链接目录，最后
清空旧下载/链接记录并开始新种子。下载中、链接中和已完成的订阅均支持换种；TV 会清空本订阅
全部旧进度，新种子必须覆盖目标起始集。清理失败会保留清理记录，可重跑继续；任务归属冲突、
文件路径越界或链接已被替换时会停止，清理完成前不会推送新种子。需要启用非 dry-run 的自动调度。

TV 的手动选择仍校验当前待下载集的季集覆盖；候选列表已更新或任务正在执行时，应刷新
详情后再选。已有下载和硬链接不会因这项规则自动删除或重置。

搜索海报由浏览器直接请求图片源，按图片源的 HTTP 响应头缓存。订阅在元数据阶段将海报
保存到 `SUBSCRIPTION_STATE_DIR/posters/`，下载失败按元数据失败重试；成功后不设过期时间，
后续展示、服务重启和列表同步均复用本地文件，不再下载。图片读取接口缺失时返回 404，
不会回源。已有订阅在下次执行元数据阶段时建立缓存，不会在升级时批量补抓。

除 `/api/auth/*` 外的管理 API 都需要鉴权。默认监听 `127.0.0.1` 且
`management.admin_token` 为空时，仅请求的直接 TCP peer 为 loopback 地址才能使用本地
bootstrap；经反向代理、容器端口映射或局域网访问时必须配置至少 24 字符的管理 token。

浏览器向 `/api/auth/login` 提交 token 后会获得 host-only、`HttpOnly`、`SameSite=Strict`、
`Path=/api` 的会话 Cookie。CLI 使用 Bearer token：

```bash
curl -H "Authorization: Bearer $TMDB_MTEAM_ADMIN_TOKEN" http://127.0.0.1:8787/api/config
```

同源部署默认不启用 CORS。只有确需跨 origin 调用 API 时，才在
`management.allowed_origins` 中填写精确的 `http(s)` origin。HTTPS 部署应同时设置
`management.secure_cookie = true`。完整配置见[配置说明](docs/operations/configuration.md)。
新安装、容器、token 轮换和忘记 token 的步骤见
[安全 Bootstrap](docs/operations/security-bootstrap.md)。

## 验证

仓库当前可用的验证命令是：

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
npm run verify:frontend
```

`.github/workflows/quality.yml` 在 pull request 直接运行；`main`/tag 的镜像发布工作流会先复用
同一组门禁，通过后才构建并发布镜像，避免 `main` 同时触发两套重复质量任务。手动发布也不能
跳过测试。

## Docker 与 NAS

Dockerfile 会在镜像内分别构建前端和 Rust 服务，不依赖宿主机已有的 `static/`：

```bash
docker build -t tmdb-mteam-hub:local .
```

NAS 部署、共享媒体挂载、UID/GID 和镜像版本配置见
[NAS Docker Compose 指南](deploy/nas/README.md)。Docker 容器内的 `config.toml` 必须监听
`0.0.0.0`，并配置至少 24 字符的 `management.admin_token`；宿主暴露范围仍应由 Compose
端口绑定、防火墙、VPN 或反向代理控制。仓库内 Compose 默认只绑定宿主 `127.0.0.1`；直接
LAN 暴露必须通过 `HOST_BIND_IP` 显式选择。

## 文档入口

- [架构总览](docs/architecture/overview.md)
- [数据与持久化](docs/architecture/data-storage.md)
- [配置说明](docs/operations/configuration.md)
- [安全 Bootstrap](docs/operations/security-bootstrap.md)
- [Retention 与日常维护](docs/operations/housekeeping.md)
- [备份与恢复](docs/operations/backup-restore.md)
- [升级与回滚](docs/operations/upgrade-rollback.md)
- [安全边界](docs/operations/security.md)
- [故障排查](docs/operations/troubleshooting.md)
- [ADR 0001：独立详情路由](docs/adr/0001-standalone-detail-routes.md)
- [ADR 0002：订阅状态收敛](docs/adr/0002-subscription-state-convergence.md)
- [ADR 0003：订阅状态只支持最新 Schema](docs/adr/0003-latest-subscription-storage-only.md)
- [历史文档归档（不可执行）](docs/archive/README.md)

## 当前边界

这是单用户、自托管应用，不应直接暴露到互联网。管理面已经支持 token/Cookie 鉴权和精确
CORS allowlist，但这些机制不能替代 TLS、NAS 防火墙、VPN 或受控反向代理。
