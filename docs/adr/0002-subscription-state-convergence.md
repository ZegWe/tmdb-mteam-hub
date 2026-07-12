---
status: accepted
owner: tmdb-mteam-hub
date: 2026-07-08
last_verified: 2026-07-11
authoritative_spec: docs/superpowers/specs/2026-07-11-project-architecture-convergence-prd.md
---

# ADR 0002：订阅状态收敛

## 背景

历史订阅模型同时使用 `status`、`processing_stage`、push/completion artifact status 和新的
lifecycle 字段，导致“下载完成”和“全流程完成”等语义冲突。

## 决策

订阅主状态只由以下字段表达：

- `lifecycle_state`
- `execution_state`
- `attention_tags`
- `failure`
- `next_attempt_at`
- TV lane due/failure 状态

电影生命周期固定为：

```text
queued -> meta -> searching -> downloading -> linking -> completed
```

`last_push.status`、`last_completion.status`、episode/file status 和 operation log status 只描述
操作产物，不参与父级主状态决策。旧字段和旧数据库不属于当前实现输入；生产启动、调度和 API
路径不得枚举、探测、打开或读取旧状态，也不得从中转换或双向推导当前状态。

HTTP 和后台 worker 最终应调用同一 application service；SQLite 调度收敛到可索引、可原子
claim 的 per-record 状态。

## 结果

- 主状态语义唯一，前端无需猜测 artifact status。
- 等待发布、失败、跳过和重试阻塞不再伪装为生命周期节点。
- 最新版本直接创建新的 per-record SQLite；旧状态文件保持原样，生产代码不探测、不打开、不
  读取、不转换、不修改、不删除。
- TV lane 必须端到端实现，或在调度入口明确停用。

## 权威文档

[架构收敛 PRD](../superpowers/specs/2026-07-11-project-architecture-convergence-prd.md)
是该决策及后续存储、调度和 API 边界的权威规格。

历史设计见已 supersede 的
[Subscription State Convergence Implementation Plan](../archive/superpowers/plans/2026-07-09-subscription-state-convergence.md)。
