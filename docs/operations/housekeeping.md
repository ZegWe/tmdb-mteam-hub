---
status: accepted
owner: tmdb-mteam-hub
last_verified: 2026-07-11
---

# Retention 与日常维护

## 操作日志

操作日志与订阅状态同处 `subscriptions.sqlite`，属于持久 state，但不应无限增长。运行时按账号独立应用
两种限制：

| 环境变量                             | 默认值  | `0` 的含义       |
| ------------------------------------ | ------- | ---------------- |
| `OPERATION_LOG_RETENTION_DAYS`       | `90`    | 不按年龄删除     |
| `OPERATION_LOG_MAX_ROWS_PER_ACCOUNT` | `10000` | 不按账号行数删除 |

每次成功插入一条操作日志时，当前 runtime 会调用共享 storage retention policy，在同一 SQLite
事务中清理该账号超过年龄或条数的旧日志。其他账号、订阅记录、配置和 account metadata 不在
删除语句范围内。日志只报告扫描/删除计数，不输出账号 key、标题、错误正文或秘密。production
operation-log HTTP 和应用审计现在都通过 latest repository 复用该 policy，不在 adapter 内执行
无界的直接 INSERT。

条数限制按 `created_at DESC, id DESC` 保留最新记录。修改策略后不必手工删除数据库；对应账号下次
写日志时会执行新策略。需要立即覆盖所有账号时，应先停服务并使用经过审阅的维护工具，禁止在
运行中直接执行宽范围 `DELETE`。

## JSON 缓存

TMDB 与豆瓣 JSON 缓存是可重建数据。TTL 分别由 `TMDB_CACHE_TTL_SECS` 和
`DOUBAN_CACHE_TTL_SECS` 控制。过期项在以下时机删除：

- 读取发现过期时；
- 服务启动时；
- 周期清理时，默认每 `21600` 秒运行一次。

`CACHE_CLEANUP_INTERVAL_SECS=0` 可关闭周期任务，但读取和启动清理仍保留。清理器只处理缓存根
目录中的普通 `.json` 和 `.json.tmp` 文件，不跟随 symlink，不递归进入目录，也不匹配
`config.toml`、`subscriptions.sqlite`、旧 `wanted.sqlite` 或其他 state 文件。

## Docker 日志

NAS Compose 的 `json-file` driver 默认通过 `LOG_MAX_SIZE=10m`、`LOG_MAX_FILE=3` 轮转。根据
设备容量和排障窗口调整；不要通过无限增大 Docker 日志代替操作日志 retention。

## SQLite VACUUM

日常按行删除不会立刻缩小 SQLite 文件，这是正常行为。只有同时满足以下条件时才考虑完整
`VACUUM`：

- 已删除大量历史日志，确实需要把空闲页归还文件系统；
- 服务和所有维护进程均已停止；
- 已完成并验证同一时间点的 config/state 备份；
- 文件系统至少有接近数据库大小的额外可用空间；
- 当前二进制明确支持该 schema 版本，且不存在 WAL/journal sidecar。

维护前后运行只读完整性检查并记录文件大小。不要把 `VACUUM` 放进每次启动、健康检查或高频
定时任务；它会重写整个数据库并需要独占访问。当前 schema 未启用 incremental auto-vacuum，
因此不能把 `PRAGMA incremental_vacuum` 当作完整 `VACUUM` 的等价替代。若未来启用，必须通过
新的数据库 manifest 版本中显式完成；不能在普通启动中原地改造已有数据库。

备份和恢复步骤见[备份与恢复](backup-restore.md)。
