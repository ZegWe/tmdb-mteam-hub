---
status: accepted
owner: tmdb-mteam-hub
last_verified: 2026-07-12
---

# 架构总览

## 形态

TMDB M-Team Hub 保持模块化单体，不拆分微服务：

- 一个 Rust/Axum 进程提供 HTTP API、后台订阅调度和静态文件服务。
- 一个 Vue SPA 提供搜索、详情、订阅、日志和设置页面。
- 一个 SQLite 数据库保存订阅状态与操作日志。
- TMDB、豆瓣、M-Team、qBittorrent 和本地文件系统作为外部适配器。

部署和演进必须维持单一配置源、单一订阅权威状态和可恢复的外部副作用。

## 目标依赖方向

```text
HTTP handlers ----\
                   -> application services -> domain + repository/client ports
worker -----------/                              ^
                                                  |
                         SQLite / HTTP / filesystem adapters
```

HTTP handler 和后台 worker 必须调用同一应用服务；worker 不应调用 Axum handler。领域规则不
依赖 Axum、Reqwest 或 Rusqlite，外部协议错误在适配器边界转换。

## 主要运行流

```text
Vue route/page
  -> shared API client
  -> Axum router
  -> application command/query
  -> subscription/config/media domain
  -> SQLite, upstream HTTP, qBittorrent, filesystem
```

后台订阅流程使用相同 command/service，只是调用入口来自 scheduler，而不是 HTTP。

## 收敛状态

当前代码仍处在架构收敛期，但入口层已明显收敛：`main.rs` 只负责启动，所有前端路由都通过
`AppShell` 加载真实页面，HTTP 错误/extractor 与健康探针也已有独立模块。生产启动现已直接创建或
打开 `subscriptions.sqlite`；同一 latest repository 驱动 production summary list、detail-by-ID、
手动 Poll、Execution worker、readiness 和 operation logs。生产代码不枚举、探测、打开、读取、
转换、修改或删除旧 `wanted.sqlite` 和 `wanted_*.json`，也不提供迁移、导入或双写。

Execution persistence 已经实现 `claim_due(limit = 1)`、`claim_one`、过期 attempt reclaim、
strict-forward lease extension、exact finish/fail、before-effect release、repository clock/nonce、
碰撞重试和原子审计；终态只把 operation-owned delta 合并进最新 payload，不依赖 claim-time
revision。running detail CAS 会以 `ExecutionGateConflict` 拒绝 attention tags 或 `skip_reason`
变化，Poll 也会在 complete missing 和 complete/incomplete movie→TV 两条清除 running attempt
路径原子记录 supersede 审计。`SubscriptionExecutionService` 负责 bounded claim、effect dispatch 和
exact finish/fail；worker 从持久化的 `next_poll_at` 恢复 Poll 下界，并使用可配置 batch/concurrency、
jitter、backpressure 和 graceful cancellation 执行 movie due work。disabled 与 dry-run 都不会 claim，
TV 仍由仓储硬性停放为 `tv_not_supported`。

旧 `WantedSubscriptionStore`、schema-v4/blob/JSON、whole-account load/save、旧 watcher/service/policy
和 effect command stack 已全部删除；`lib.rs` 已缩至 41 行模块/启动 facade，qB、M-Team、TMDB、
Douban、配置、日志和订阅 HTTP 边界均由 feature module 所有。Poll 先把豆瓣响应映射为 provider-
neutral wanted values，Execution 接收窄策略而非完整 `FileConfig`，Rusqlite schema manifest 归属
`storage/schema_v5.rs`。

稳定管理 DTO 由 `src/http/openapi.json` 的 OpenAPI 3.1 schema 描述，生成脚本产出前端 checked-JS
类型、枚举和字段常量；CI 会校验 schema digest 和类型检查，避免前后端继续手写两套契约。

外部副作用已经有 transport/filesystem-neutral foundation：稳定 effect key、qB hash/tag
reconciliation、response-loss/crash retry，以及 deterministic hardlink 的 same-inode success、
different-inode conflict 和逐文件 retry/skip。真实 M-Team/qB/filesystem adapters 已通过
`LatestSubscriptionExecutionEffects` 接入 Execution：Search 以稳定 tag/hash reconcile 后至多 add
一次，Progress 精确观察同一 qB task，Link 在 bounded blocking executor 中逐文件执行确定性硬链接。
后续工作按以下顺序继续：

1. 扩展剩余 endpoint schema/type coverage，并继续收窄 provider/application ports。
2. 完成剩余 CSS 精简和首次 hosted Chromium/Firefox E2E gate。
3. 完成首次 GitHub-hosted container CI smoke 与剩余人工安全验收。

详细目标与验收条件以
[架构收敛 PRD](../superpowers/specs/2026-07-11-project-architecture-convergence-prd.md)为准。

## 相关文档

- [数据与持久化](data-storage.md)
- [ADR 0001：独立详情路由](../adr/0001-standalone-detail-routes.md)
- [ADR 0002：订阅状态收敛](../adr/0002-subscription-state-convergence.md)
- [ADR 0003：订阅状态只支持最新 Schema](../adr/0003-latest-subscription-storage-only.md)
- [配置说明](../operations/configuration.md)
- [备份与恢复](../operations/backup-restore.md)
- [Retention 与日常维护](../operations/housekeeping.md)
