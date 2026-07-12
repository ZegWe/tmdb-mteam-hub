---
status: accepted
owner: tmdb-mteam-hub
last_verified: 2026-07-12
---

# 数据与持久化

## 数据分类

| 数据         | 默认/容器路径                                                                   | 是否权威     | 备份要求    |
| ------------ | ------------------------------------------------------------------------------- | ------------ | ----------- |
| 配置与密钥   | `config.toml` / `/data/config/config.toml`                                      | 是           | 必须        |
| 订阅 SQLite  | `cache/subscriptions/subscriptions.sqlite` / `/data/state/subscriptions.sqlite` | 是           | 必须        |
| TMDB 缓存    | `cache/tmdb` / `/data/cache/tmdb`                                               | 否           | 可省略      |
| 豆瓣缓存     | `cache/douban` / `/data/cache/douban`                                           | 否           | 可省略      |
| 下载与媒体库 | `/srv/media/...`                                                                | 外部媒体数据 | 按 NAS 策略 |

容器部署通过 `SUBSCRIPTION_STATE_DIR=/data/state` 把权威状态与缓存分开。旧的
`wanted.sqlite` 和 `wanted_*.json` 不属于当前实现的输入：服务不会枚举、探测或打开这些文件，
也不会读取、转换、修改或删除它们。

## 配置

配置的权威来源是经过验证的内存对象和一份 TOML 文件。解析失败必须阻止启动，不能回退为
空默认值。规范化写回必须先备份，并以限制权限的临时文件原子替换。

详见[配置说明](../operations/configuration.md)。

## 订阅状态

`subscriptions.sqlite` 只保存当前 schema 的 per-record aggregate、调度索引、账户轮询元数据和
操作日志。项目不提供旧数据库导入或 schema 迁移；新数据库直接按最新 manifest 创建。如果该
文件自身不是当前 schema，服务应明确拒绝使用，不得尝试修复或降级。

latest repository adapter 提供独立的
Read/Mutation/Poll/Execution capabilities，包括有界 summary/detail 读取、单记录 optimistic
detail CAS、完整 Poll 持久化，以及 `claim_due(limit = 1)`、`claim_one`、expired reclaim、
strict-forward lease extension、exact finish/fail 和 before-effect release。
detail CAS 只更新详情 JSON、源投影、attention tags、时间和 revision；当 execution 正在 running
时，attention-tag 集合或 `skip_reason` 变化会以 `ExecutionGateConflict` 失败，其他合法详情仍可
推进 revision 而不改变 attempt identity。

Poll 写入通过 `BEGIN IMMEDIATE` 精确消费 account attempt token，部分快照只合并 seen rows，
只有完整快照可停用缺失记录；当 complete missing 或 complete/incomplete movie→TV 必须清除
running attempt 时，行更新、`subscription_scheduler/supersede_attempt` 审计和 poll meta/token
消费原子提交。Execution 使用 repository clock、注入 nonce 和有界碰撞重试原子完成
claim/reclaim；后续命令只用 key、typed operation、attempt ID 和 live lease fencing，不使用
claim-time revision。成功/失败终态只把 operation-owned delta 合并进最新 payload，保留期间发生的
Poll/detail revision 更新；release 保留 due/force。claim、reclaim、extend、finish、fail、release
的行/payload 变化与审计均在同一事务，审计失败会全部回滚。

副作用领域定义稳定的 qB effect key、hash/tag reconciliation、response-loss/crash retry 和非破坏性
hardlink reconciliation；同 inode、目标冲突、逐文件 retry/verified skip 均有 domain、真实 adapter
和临时文件系统测试。生产 Execution service 已接入 M-Team、qB 和 bounded filesystem executor；
response loss 的下一 attempt 固定复用已持久化 torrent identity，并先按 stable tag/hash reconcile。

生产启动现在直接创建或打开 `subscriptions.sqlite`，并把同一 latest repository 注入
`AppState` 的 query、Poll、Execution、readiness、operation-log 和 worker 路径。生产 API 只注册
summary list、detail-by-ID 和手动 Poll；尚未移植的 candidates/push/retry/rerun/progress/completion
路径返回 404，避免恢复旧 handler contract。worker 调用独立 application services，不调用 HTTP
handler；disabled/dry-run 不 claim，live 模式按 persisted Poll schedule 与 bounded Execution batch
运行 movie effects。TV 永远不会被 claim。
旧 `WantedSubscriptionStore`、schema-v4/blob/JSON、whole-account load/save、旧 watcher/service/policy
和 effect command stack 已删除。旧存储名称只允许出现在 latest manifest 的 forbidden-object 列表
和“旧文件保持字节不变”的 sentinel/保护测试中。

运维侧不应直接编辑 SQLite 表。升级前只备份 `config.toml` 和
`subscriptions.sqlite`；恢复不会把 state 目录中的其他文件带入当前服务。

## 缓存

TMDB 和豆瓣缓存只用于减少上游请求，可以在服务停止后删除并重建。缓存目录不得存放配置、
订阅 SQLite，也不得与 `SUBSCRIPTION_STATE_DIR` 相同或互为父子目录。启动时会解析真实路径
并拒绝重叠配置，确保缓存清理不会枚举订阅状态目录。

## 媒体目录与硬链接

容器只挂载一个共享宿主媒体根目录到 `/srv/media`。下载源和媒体库目标必须是该根目录下的
子目录，例如：

```text
/srv/media/downloads/movies
/srv/media/library/movies
```

硬链接要求源和目标位于同一文件系统。跨 NAS volume 或跨设备挂载会返回 `EXDEV`，不能通过
修改容器内路径绕过。

## 最新版本不变量

- 不可读配置或当前数据库不能被空文件替代。
- 当前 runtime 只创建或打开 `subscriptions.sqlite`。
- `wanted.sqlite`、`wanted_*.json` 和旧 blob 永远不被生产代码枚举、探测或打开，也不参与启动、
  查询或调度。
- 不保留双写、导入 fallback、旧 schema converter 或迁移 CLI。

恢复流程见[备份与恢复](../operations/backup-restore.md)。
latest-only 文件决策见
[ADR 0003：订阅状态只支持最新 Schema](../adr/0003-latest-subscription-storage-only.md)。
