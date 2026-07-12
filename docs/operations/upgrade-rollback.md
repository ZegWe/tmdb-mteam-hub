---
status: accepted
owner: tmdb-mteam-hub
last_verified: 2026-07-12
---

# 升级与回滚

## 升级前

1. 阅读目标版本变更，确认当前版本的配置和 `subscriptions.sqlite` 合同。
2. 固定当前 `TMDB_MTEAM_HUB_TAG`，记录可工作的镜像标签或 digest。
3. 按[备份与恢复](backup-restore.md)停机备份 `config/config.toml` 和
   `state/subscriptions.sqlite`。
4. 确认共享媒体根、UID/GID 和目录权限没有变化。

不要只依赖 `latest` 作为升级和回滚依据。

项目不提供旧 SQLite 迁移。当前服务不会枚举、探测、打开、读取、转换、修改或删除旧
`wanted.sqlite`；升级只使用 `subscriptions.sqlite`。

## 升级

修改 `.env`：

```dotenv
TMDB_MTEAM_HUB_TAG=v0.2.0
```

然后执行：

```bash
docker compose pull
docker compose up -d
docker compose ps
docker compose logs --tail=200 tmdb-mteam-hub
```

确认进程存活、状态存储可读，并验证静态首页：

```bash
curl --fail http://127.0.0.1:8787/healthz
curl --fail http://127.0.0.1:8787/readyz
curl --fail http://127.0.0.1:8787/
```

`/healthz` 只证明进程事件循环存活；`/readyz` 会只读检查已有订阅 SQLite，且在数据库尚不存在
时不会修复它。损坏或不符合当前 manifest 的数据库会让 readyz 返回 503，但不会被探针自动
备份、替换或转换。状态写入正在进行时探针会快速返回 503 而不排队，升级验证可隔几秒重试；
持续失败才按 SQLite/schema 故障处理。

同时检查配置没有被意外重写，订阅数量和日志可以读取。

## 回滚

如果新版本只发生无 schema 影响的代码问题，可以先恢复原镜像标签：

```bash
docker compose stop
# 修改 .env 中的 TMDB_MTEAM_HUB_TAG
docker compose pull
docker compose up -d
```

如果新版本写入了旧镜像不理解的当前状态，则必须同时恢复升级前匹配的 `config.toml` 和
`subscriptions.sqlite` 备份，不能只切换镜像，也不能尝试把 `subscriptions.sqlite` 改名成旧
`wanted.sqlite`。

回滚后重复首页、订阅数量、最近日志和媒体路径验证，并保留失败版本的数据副本用于分析。
