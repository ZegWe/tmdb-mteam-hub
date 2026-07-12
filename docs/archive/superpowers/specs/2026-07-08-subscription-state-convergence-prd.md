---
---

status: superseded
owner: tmdb-mteam-hub
last_verified: 2026-07-11
implementation_status: superseded
authoritative: false
archived_at: 2026-07-11
executable: false
superseded_by: docs/superpowers/specs/2026-07-11-project-architecture-convergence-prd.md
superseded_scope_note: The newer PRD retains lifecycle semantics but replaces TV execution and artifact persistence requirements.
tracking_note: Historical body retained only as design history; migration requirements are obsolete and must not be implemented.
related_adr: docs/adr/0002-subscription-state-convergence.md

---

# 订阅状态机收敛 PRD

> 历史归档：本文档中的 migration、legacy runtime、TV execution 与兼容性要求均不可执行。

## 背景

当前订阅系统存在两套状态模型并行运行：

- legacy 模型：`status`、`processing_stage`、`last_push.status`、`last_completion.status` 共同驱动后端自动 pipeline 和前端展示。
- 新模型：`lifecycle_state`、`execution_state`、`attention_tags`、`failure`、`next_attempt_at`、TV operation lanes 已经建模，但生产调度尚未完全接管。

这种并行状态导致语义冲突。例如 legacy `status = Completed` 表示“下载完成待硬链接”，而新模型 `lifecycle_state = Completed` 表示“整个订阅完成”。因此代码中容易把“下载完成”误判为“全流程完成”，造成下载完成后不进入硬链接等问题。

本 PRD 要求将订阅状态管理收敛到新模型。收敛后，后端调度、API 返回、前端展示和按钮行为均以新模型为权威；legacy 状态字段和由 legacy 字段派生新状态的兼容层不再作为运行逻辑的一部分。

## 目标

- 订阅主状态只使用 `lifecycle_state`，取值为 `queued`、`meta`、`searching`、`downloading`、`linking`、`completed`。
- `failed`、`skipped`、`waiting_release`、`retry_blocked` 不作为主状态，统一作为 `attention_tags` 或结构化 `failure` 表达。
- 自动 tick 调度只使用 `select_due_operation()` 和 TV lane due 逻辑，不再使用 `status` 或 `processing_stage` 选择动作。
- 电影流程符合线性状态机：`queued -> meta -> searching -> downloading -> linking -> completed`。
- TV 流程使用父级 `lifecycle_state` 展示状态，并用 `search`、`progress`、`link` 三条 lane 执行并行任务。
- 前端订阅主状态展示只依赖 `lifecycle_state`、`attention_tags` 和 `failure`，不再从 `status`、`processing_stage`、`last_push.status`、`last_completion.status` 推断主状态。
- 删除 legacy 状态兼容层，不保留“新老状态互相反推”的运行路径。

## 非目标

- 不重写豆瓣、M-Team、qBittorrent 或硬链接外部协议。
- 不要求改变 `last_push`、`last_completion`、episode file status 等操作产物的内部状态字段；这些字段可继续表示下载任务、硬链接任务或文件粒度结果。
- 不要求在本次 PRD 中实现质量升级、替换已完成资源或多季 TV 订阅。
- 不保留旧 API contract 对 `status` 和 `processing_stage` 的兼容行为。

## 术语

- 主状态：`lifecycle_state`，表示订阅生命周期节点。
- 执行状态：`execution_state`，表示是否有当前操作正在运行，取值 `idle` 或 `running`。
- 关注标签：`attention_tags`，表示非主状态提示，如 `skipped`、`failed`、`waiting_release`、`retry_blocked`。
- 失败对象：`failure`，结构化记录失败作用域、错误消息、重试次数和下次重试时间。
- 操作产物状态：`last_push.status`、`last_completion.status`、episode/file status，仅描述某次下载、硬链接或文件结果，不表示订阅主状态。
- due operation：一次 tick 选出的具体操作，如电影 meta、电影搜索、电影进度同步、电影硬链接，或 TV lane 操作。
- TV lane：TV 的 `search`、`progress`、`link` 三条独立调度通道。

## 当前问题

### 1. 生产调度仍由 legacy 字段驱动

当前自动 watcher 调度路径仍是：

```text
process_wanted_watch_queue
  -> automatic_pipeline_action_for_wanted_record
  -> pipeline_action_for_wanted_record
```

`pipeline_action_for_wanted_record()` 以 legacy `status` 决定下一步：

```text
Unprocessed / Matching / Processing / Skipped -> Push
Pushed / Downloading -> Progress
Completed -> Completion
Linked / Failed -> None
```

这绕过了新模型里的 `select_due_operation()` 和 TV lane due 选择。

### 2. 新状态多由旧状态派生

`lifecycle_state` 当前多数由 `infer_lifecycle_from_legacy()` 从 `status`、`processing_stage`、`last_push`、`last_completion` 反推。`derive_legacy_status()` 又能从新状态反推 legacy status。双向反推让状态权威不清晰。

### 3. 前端仍从多处 legacy 字段推断主状态

订阅卡片、状态徽章、状态条、summary、重试按钮可用性仍读取 `status`、`processing_stage`、`last_push.status`、`last_completion.status`。这些字段混合了主状态、阶段、任务结果和错误结果，容易展示错误状态。

### 4. TV 新 lane 模型未接入运行路径

TV lane、cursor、episode assignment、父级状态派生等模型已经存在，但生产路径没有完整使用。当前仍把 TV 当成接近电影的 legacy status pipeline。

## 目标状态模型

### 权威字段

订阅记录必须以以下字段作为权威状态：

| 字段                | 作用                                          |
| ------------------- | --------------------------------------------- |
| `lifecycle_state`   | 主生命周期状态                                |
| `execution_state`   | 当前是否正在执行操作                          |
| `attention_tags`    | 跳过、失败、等待发布、重试阻塞等非主状态提示  |
| `failure`           | 父级或当前操作失败信息                        |
| `next_attempt_at`   | 电影或父级操作下一次尝试时间                  |
| `tv.lanes.*`        | TV 各 lane 的下一次尝试、失败、重试信息       |
| `tv.episodes`       | TV episode 目标、assignment、下载和硬链接结果 |
| `tv.download_tasks` | TV 下载任务及覆盖范围                         |

### 主状态语义

| `lifecycle_state` | 语义                                                             |
| ----------------- | ---------------------------------------------------------------- |
| `queued`          | 已入队，尚未准备元数据；跳过订阅也停留在此状态并带 `skipped` tag |
| `meta`            | 正在准备或校验元数据                                             |
| `searching`       | 正在搜索资源或等待资源发布                                       |
| `downloading`     | 至少存在活跃下载任务                                             |
| `linking`         | 至少存在已下载待硬链接或正在硬链接的内容                         |
| `completed`       | 所有目标内容均完成或被跳过                                       |

主状态不得表达失败、跳过、等待发布或重试阻塞。这些必须通过 `attention_tags` 和 `failure` 表达。

### 操作产物状态

以下字段可继续存在，但不得用于订阅主状态决策：

- `last_push.status`
- `last_completion.status`
- episode `status`
- file `status`

它们只用于展示下载任务、硬链接任务、episode 或文件的具体结果。

## 后端行为要求

### 统一 tick 调度

自动 watcher 每秒 tick，但每次 tick 只执行 due operation。调度入口必须改为：

```text
snapshot records
  -> select_due_operation(record, now)
  -> execute selected operation
  -> persist lifecycle/lane outcome
```

自动调度不得使用 legacy `status` 或 `processing_stage` 作为动作选择输入。

### 电影状态转换

电影必须按以下规则流转：

```text
queued -> meta -> searching -> downloading -> linking -> completed
```

转换要求：

- `queued` tick：进入 `meta`。
- `meta` 成功：进入 `searching`，`next_attempt_at = now`。
- `searching` 推送成功：进入 `downloading`，`next_attempt_at = now`。
- `searching` 无候选或无匹配：保持 `searching`，添加 `waiting_release`，`next_attempt_at = now + search_interval_secs`，不增加系统失败 retry。
- `downloading` 未完成：保持 `downloading`，`next_attempt_at = now + progress_interval_secs`。
- `downloading` 确认完成：进入 `linking`，`next_attempt_at = now`。
- `linking` 成功：进入 `completed`。
- `linking` 失败但可重试：保持 `linking`，写 `failure`，`next_attempt_at = now + link_retry_interval_secs`。

### TV 状态转换

TV 父级 `lifecycle_state` 由 TV 子状态派生：

1. 元数据未完成：`meta`
2. 全部目标完成或跳过：`completed`
3. 存在可链接或正在链接内容：`linking`
4. 存在活跃下载任务：`downloading`
5. 存在未覆盖目标或 search cursor：`searching`
6. 尚未开始：`queued`

TV 每个 tick 最多执行一条 due lane，优先级为：

```text
link > progress > search
```

lane 之间有独立的 `next_attempt_at`、`failure` 和 retry 计数。一个 lane 失败不得阻止其他 due lane 在后续 tick 运行，除非父级进入 `completed` 或被 `skipped`。

### 手动操作

详情页只保留两个操作：

- 重试当前节点
- 重跑任务

后端语义：

- 重试当前节点：根据 `select_due_operation()` 当前可执行操作或失败作用域，清除对应 `failure/retry_blocked`，设置对应 `next_attempt_at = now`。
- 重跑任务：重置订阅到 `meta` 或 `searching` 的确定入口，清除下载/硬链接派生产物中会阻碍新任务的字段，但保留豆瓣元数据缓存。

手动操作不得依赖 legacy `processing_stage` 判断当前节点。

### 失败处理

失败必须按作用域记录：

- 父级 operation failure：电影 meta/search/progress/link 等。
- TV lane failure：search/progress/link lane。
- TV task failure：某个 qB download task。
- TV episode failure：某个 episode 硬链接或覆盖验证失败。

失败规则：

- 系统错误增加 retry。
- 业务等待，如无候选、无匹配、资源未发布，不增加系统失败 retry。
- 达到重试上限后写 `retry_blocked`。
- `failed` 只作为 `attention_tags`，不能成为 `lifecycle_state`。

## API 要求

### 订阅记录返回

API 返回必须以新状态字段为主：

```json
{
  "lifecycle_state": "downloading",
  "execution_state": "idle",
  "attention_tags": [],
  "failure": null,
  "next_attempt_at": 1234567890
}
```

不得要求前端读取 `status` 或 `processing_stage` 来展示主状态。

### 删除 legacy status API

`/api/subscriptions/wanted/{id}/status` 是 legacy API，必须删除。PRD 要求不保留“直接设置 status”的能力。

替代操作只能是语义化命令：

- 重试当前节点。
- 重跑任务。
- 跳过订阅或跳过 episode。

这些命令必须由后端根据新状态模型修改 `lifecycle_state`、`attention_tags`、`failure` 和 due 时间，不能让调用方直接写主状态。

### 保留操作产物字段

以下字段可继续返回，用于详情展示：

- `last_push`
- `last_completion`
- `tv.episodes`
- `tv.download_tasks`
- operation logs

但 API 文档必须说明它们不是订阅主状态来源。

## 前端要求

### 主状态展示

前端订阅列表和详情页主状态必须只基于：

- `lifecycle_state`
- `attention_tags`
- `failure`
- `execution_state`

状态徽章示例：

| 条件                                    | 展示         |
| --------------------------------------- | ------------ |
| `lifecycle_state = queued`              | 待处理       |
| `lifecycle_state = meta`                | 准备元数据   |
| `lifecycle_state = searching`           | 搜索中       |
| `lifecycle_state = downloading`         | 下载中       |
| `lifecycle_state = linking`             | 硬链接中     |
| `lifecycle_state = completed`           | 已完成       |
| `attention_tags` 包含 `skipped`         | 跳过         |
| `attention_tags` 包含 `failed`          | 当前节点失败 |
| `attention_tags` 包含 `waiting_release` | 等待发布     |
| `attention_tags` 包含 `retry_blocked`   | 重试阻塞     |

### 状态条

状态条节点固定为：

```text
queued -> meta -> searching -> downloading -> linking -> completed
```

当前节点由 `lifecycle_state` 决定。节点上的提示由 `attention_tags` 和 `failure` 决定。前端不得再用 `processing_stage`、`status`、`last_push.status` 或 `last_completion.status` 推断当前节点。

### 详情操作

详情页只显示：

- 重试当前节点
- 重跑任务

按钮可用性：

- `重试当前节点`：订阅未 `completed`，且后端允许重试当前 due/failure 作用域。
- `重跑任务`：存在 `subject_id` 且未被不可恢复策略禁止。

前端不再提供“刷新下载进度”或“检查完成并硬链接”按钮。下载进度和硬链接应由自动 tick 或“重试当前节点”覆盖。

## 数据迁移要求

本项目不保留 legacy runtime 兼容层，但现有持久化数据需要一次性迁移：

1. 读取旧记录。
2. 根据旧 `status`、`processing_stage`、`last_push`、`last_completion` 计算目标 `lifecycle_state`、`attention_tags`、`failure`。
3. 写入新字段。
4. 迁移后生产代码不得再从旧字段反推状态。

迁移完成后，`status` 和 `processing_stage` 必须从 API record 结构、前端消费、运行时调度、状态写入和测试夹具中删除。SQLite 物理列应通过迁移删除；如果底层 SQLite 迁移需要分阶段执行，过渡期也不得再写入、序列化或读取这些列来参与任何运行决策。

迁移函数是唯一允许读取旧字段的代码。迁移函数完成后，生产 tick、手动操作、API handler 和前端都不得再访问旧字段。

## 删除清单

必须删除或停止使用以下 legacy 调度/兼容逻辑：

- `WantedSubscriptionStatus` 作为主状态。
- `processing_stage` 作为当前节点。
- `WantedStatusUpdate` 和直接设置 legacy status 的 API。
- `pipeline_action_for_wanted_record()`。
- `wanted_record_needs_automatic_pipeline()`。
- `retry_action_for_wanted_record()` 中基于 legacy stage/status 的判断。
- `infer_lifecycle_from_legacy()` 的运行时调用。
- `derive_legacy_status()` 的运行时调用。
- `sync_lifecycle_from_legacy_stage()`。
- `apply_status_stage()`、`apply_push_stage()`、`apply_completion_stage()` 这类通过 legacy stage 派生新状态的写入路径。
- 前端 `SUB_LEGACY_LIFECYCLE_BY_STATUS`。
- 前端从 `record.status`、`record.processing_stage` 推断主状态的逻辑。

## 保留清单

以下逻辑应保留并成为权威或辅助：

- `SubscriptionLifecycleState`。
- `SubscriptionExecutionState`。
- `SubscriptionAttentionTag`。
- `SubscriptionFailure`。
- `select_due_operation()`。
- `select_due_tv_lane()`。
- `OperationLaneState`。
- TV `search/progress/link` lanes。
- TV episode records 和 download task records。
- `next_attempt_at` 和 lane-level next attempt。
- operation logs 的 `status` 字段，因为它表示日志结果，不是订阅主状态。
- `last_push.status`、`last_completion.status` 等操作产物状态，仅用于详情展示。

## 验收标准

### 后端

- 自动 watcher 不再调用 legacy `pipeline_action_for_wanted_record()` 选择操作。
- 自动 watcher 使用 `select_due_operation()` 选择电影操作和 TV lane。
- 电影下载完成后写 `lifecycle_state = linking`，下一 tick 或同次显式链路进入硬链接操作。
- 硬链接成功后写 `lifecycle_state = completed`。
- `failed`、`skipped`、`waiting_release` 不再作为主状态出现。
- 无候选/无匹配不会增加系统失败 retry。
- 下载进度同步失败不会使订阅退出自动 pipeline；它应保持 `downloading` 并按 retry/interval 再试。
- TV lane `link > progress > search` 优先级有测试覆盖。
- 旧 `/status` API 被删除。

### 前端

- 列表卡片状态徽章只读 `lifecycle_state`、`attention_tags`、`failure`。
- 详情状态条只读 `lifecycle_state`。
- 详情页只保留两个操作按钮：重试当前节点、重跑任务。
- 前端源码中不存在从 `record.status` 或 `record.processing_stage` 推断订阅主状态的逻辑。
- `last_push.status` 和 `last_completion.status` 只用于下载/硬链接详情行展示。

### 数据

- 现有记录可迁移到新模型。
- 迁移后没有运行路径、API 返回或前端展示依赖旧字段。
- SQLite 查询可通过 `lifecycle_state`、`next_attempt_at` 和 TV lane next attempt 找到 due 记录。

## 测试要求

- 单元测试覆盖电影完整状态流：`queued -> meta -> searching -> downloading -> linking -> completed`。
- 单元测试覆盖电影 waiting release：无候选/无匹配保持 `searching(waiting_release)` 并使用搜索间隔。
- 单元测试覆盖下载未完成：保持 `downloading` 并使用 5 秒进度间隔。
- 单元测试覆盖下载完成：进入 `linking`，不能停在 `completed`。
- 单元测试覆盖硬链接成功：进入 `completed`。
- 单元测试覆盖硬链接失败：保持 `linking`，写 `failure` 和 retry 时间。
- 单元测试覆盖 TV lane due 优先级。
- 单元测试覆盖 TV 某 lane 失败不阻止其他 due lane。
- 前端源码测试覆盖不再引用 `record.status` / `processing_stage` 推断主状态。
- API 测试覆盖返回的新状态字段。

## 实施约束

- 这是一次状态机收敛，不是兼容性修补。实现过程中不得新增“新老状态互相反推”的运行路径。
- 可以有一次性迁移函数读取旧字段，但迁移函数不能被生产 tick 反复依赖。
- 每个阶段必须先加测试再改实现。
- 删除旧字段时应同步清理前端、API、SQLite 索引和测试。
- 迁移脚本可以临时读取旧字段；迁移完成后的生产代码不得读取、写入或序列化旧字段。

## 开放问题

无。当前决策是彻底收敛到新模型，不保留 legacy runtime 兼容层。
