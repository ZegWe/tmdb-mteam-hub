---
status: accepted
owner: tmdb-mteam-hub
date: 2026-07-11
last_verified: 2026-07-11
authoritative_spec: docs/superpowers/specs/2026-07-11-project-architecture-convergence-prd.md
---

# ADR 0003：订阅状态只支持最新 Schema

## 背景

仓库曾包含 `wanted.sqlite`、`wanted_*.json`、account blob、schema-v4 fixtures、离线备份与
v4→v5 converter。它们让最新 repository/runtime 的实现同时承担旧状态解释、冲突裁决、备份、
回滚和双轨测试，显著扩大了存储层和文档表面积。

当前产品不要求继承旧订阅状态；继续维护迁移路径不会增加当前功能，只会延迟最新 runtime 接线并
保留错误的双重权威。

## 决策

- 当前版本只使用 `subscriptions.sqlite`。
- 文件不存在时，直接按 latest manifest 创建并完整验证。
- 文件存在时，必须精确符合 latest manifest；运行时不执行 schema repair 或转换。
- `wanted.sqlite`、`wanted_*.json` 和旧 blob 不枚举、不探测、不打开、不读取、不转换、不修改、
  不删除。
- 删除迁移 CLI、converter、v4 fixtures、legacy blob DDL 和导入 fallback。
- 备份、恢复、健康检查和容器 smoke 只围绕当前 `subscriptions.sqlite`。

## 结果

- 旧状态不会自动出现在新版本中；需要保留时由使用者把旧文件单独归档。
- 存储代码只需要证明当前 manifest、并发、fencing、幂等和恢复语义。
- migration/v4 fixture 与旧 aggregate/blob/JSON/watcher/effect-command runtime 均已删除；仅 L5
  cleanup 就净删至少 15,700 行兼容代码和测试。
- 回滚只支持恢复同一当前 schema 的备份与对应镜像，不支持把当前数据库交给旧 runtime。

## 验证

- fresh initializer 会在旧文件旁创建 `subscriptions.sqlite`，并证明旧文件字节不变。
- 生产源码中不得存在迁移入口或 `subscription_state_blobs` 读写。
- 容器镜像不得包含 `subscription-migrate-v5`。

具体实施与验收门见
[Latest Subscription Storage and Scheduler Plan](../superpowers/plans/2026-07-11-subscription-storage-scheduler.md)。
