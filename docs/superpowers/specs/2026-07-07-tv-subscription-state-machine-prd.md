# 豆瓣想看订阅状态机 PRD

## 背景与目标

当前豆瓣想看订阅系统的主状态混合了生命周期节点、执行中状态、失败状态和跳过状态。现有状态包括 `unprocessed`、`matching`、`processing`、`pushed`、`downloading`、`completed`、`linked`、`failed`、`skipped`。这对电影的线性流程尚可工作，但无法清晰表达 TV 的并行流程：一边搜索后续集，一边同步已推送任务进度，一边对已完成任务做硬链接。

本 PRD 定义新的订阅状态机：

- 主状态只表示生命周期节点：`queued`、`meta`、`searching`、`downloading`、`linking`、`completed`。
- `failed`、`skipped`、`running`、`idle` 不再作为主状态，而是执行状态、关注标签或结构化错误信息。
- 电影保持线性流程。
- TV 使用父级展示状态 + 分集目标 + 下载任务 + operation lane，支持单集、范围合集、整季合集和并行处理。
- 后端使用统一 tick 调度；成功流转到下一状态后，下一 tick 不等待间隔；只有同一状态或同一 lane 重复尝试时才等待对应间隔。
- 手动重试跳过间隔，让对应记录、lane、task 或 episode 立即 eligible。

## 非目标

- 不重写豆瓣、M-Team、qBittorrent、硬链接等外部集成协议。
- 不要求 TV 立即支持跨多季父级订阅；MVP 以单季或明确 episode 范围为目标。
- 不把“未发布/搜不到”视为系统异常。
- 不支持自动质量升级或替换已完成 episode；高质量替换属于单独的重跑/升级流程。
- 不要求一次 tick 内连续执行多个主状态转换，例如 `meta -> searching -> downloading` 必须跨 tick 完成。

## 术语

- 订阅记录：来自豆瓣想看的父级实体，表示一部电影或一个 TV 目标季。
- 主状态：父级生命周期节点，只能是 `queued`、`meta`、`searching`、`downloading`、`linking`、`completed`。
- 执行状态：单选字段，表示当前是否正在执行，取值为 `idle` 或 `running`。
- 关注标签：可选提示，如 `waiting_release`、`failed`、`retry_blocked`、`skipped`、`needs_reconciliation`。
- tick：后端统一调度的一次扫描和处理周期。
- operation lane：TV 的可调度操作通道，包括 `search`、`progress`、`link`。每条 lane 有自己的间隔、重试和 next_attempt。
- episode record：TV 的目标分集记录，表示用户最终期望完成的每一集。
- download task record：一次成功推送到 qB 的 torrent 任务，可能覆盖单集、范围合集或整季合集。
- search cursor：`search_cursor_episode`，表示 TV 下一次搜索必须覆盖的最小目标集数。
- tentative coverage：根据标题或候选信息推断的临时覆盖范围。
- verified coverage：经过 qB 文件列表或硬链接结果确认的有效覆盖范围。
- waiting release：搜索不到 cursor 对应集的业务等待状态，不增加失败重试次数。

## 状态模型

### 主状态

| 主状态 | 含义 |
| --- | --- |
| `queued` | 订阅已入队，尚未开始元数据准备；历史订阅或跳过记录也停留在此状态并带 `skipped` 标签 |
| `meta` | 准备订阅元数据，如 rexxar 详情、封面、标题、发布日期、TV 集数和目标范围 |
| `searching` | 当前没有活跃下载或链接任务，正在寻找可推送资源或等待发布 |
| `downloading` | 存在活跃 qB 下载任务；TV 仍可继续搜索后续未覆盖集 |
| `linking` | 存在已下载但未完成硬链接的任务或分集；TV 仍可继续搜索和同步下载 |
| `completed` | 所有目标内容均完成硬链接或被明确跳过，是成功终态 |

TV 父级主状态是展示和聚合状态，不是唯一调度入口。TV 的调度由 `search`、`progress`、`link` 三条 lane 决定。父级展示状态必须先满足元数据保护条件：只有 `metadata_ready = true`、`episode_records_initialized = true`、`target_episode_set_known = true` 时，才能派生 `searching`、`downloading`、`linking` 或 `completed`；否则保持 `meta`。

元数据完成后，父级展示状态按以下优先级派生：

1. 满足 TV 完成条件：`completed`
2. 存在可链接或正在链接的 task/episode：`linking`
3. 存在活跃下载 task：`downloading`
4. 存在未覆盖 episode 或 cursor 等待发布：`searching`
5. 元数据未完成或目标集合未初始化：`meta`
6. 尚未开始：`queued`

因此 TV 处于 `linking` 展示状态时，tick 仍可继续同步其他下载任务，也可按搜索间隔继续搜索后续未覆盖集。

### 执行状态与关注标签

为避免标签互相冲突，执行状态和关注标签分开：

- `execution_state`：单选，`idle` 或 `running`。
- `attention_tags`：可包含 `waiting_release`、`failed`、`retry_blocked`、`skipped`、`needs_reconciliation`。
- `failure`：结构化错误对象，记录错误发生的 lane/task/episode、错误类型、消息、retry 计数和下次重试时间。
- `skip_reason`：跳过原因。

展示优先级：`skipped > retry_blocked > failed > waiting_release > running > idle`。TV 的 `waiting_release` 应尽量附着在 cursor 或 episode 上，父级展示为“下载中 · 等待 E05 发布”而不是泛化为整部剧等待。

## 电影流程

电影是单目标线性流程：

1. 新增订阅进入 `queued(idle)`。
2. tick 处理 `queued` 后进入 `meta`，下一 tick 可立即执行 `meta`。
3. `meta` 补齐 rexxar 元数据和封面，成功后进入 `searching`。
4. `searching` 搜索种子并推送 qB，成功后进入 `downloading`。
5. `downloading` 同步 qB 进度，未完成时保持 `downloading` 并按下载间隔再次同步；完成后进入 `linking`。
6. `linking` 执行硬链接，成功后进入 `completed`。

历史订阅或标记跳过的电影展示为 `queued(skipped)`，不参与自动 tick。手动重试清除 `skipped`，进入 `meta`，且不丢失已缓存的元数据。

电影在 `searching` 搜不到候选或未命中匹配规则时，保持 `searching(waiting_release)`，写入下一次搜索时间，不增加失败重试次数。

## TV 流程

TV 的核心不是单个 cursor 阻塞推进，而是父级订阅下并行维护 episode records 和 download task records。

### 初始化

TV 进入 `meta` 时应准备：

- episode total、season number、目标 episode 范围。
- episode records，每集至少包含 season、episode、label、目标状态、下载状态、硬链接状态。
- `search_cursor_episode`，初始为第一个未完成、未跳过、无有效覆盖的目标 episode。
- TV 的 `search`、`progress`、`link` lane 调度字段。

豆瓣 wanted poll 只负责发现和刷新基础 wish item。rexxar 预取可以作为 best-effort 缓存，但 `meta` 是订阅元数据和 episode 初始化的权威校验阶段；若 poll 已提供有效详情，`meta` 可复用而不重复请求。

### Operation Lanes

TV tick 不依赖单一父级状态来决定操作，而是评估三条 lane：

| Lane | 职责 | 间隔作用域 |
| --- | --- | --- |
| `search` | 从 `search_cursor_episode` 搜索并推送覆盖 cursor 的 torrent | 搜不到、未命中或搜索失败后再次搜索 |
| `progress` | 同步活跃 download task 的 qB 进度和文件列表 | 任务未完成时再次同步 |
| `link` | 对已下载 task 覆盖的 episode 执行硬链接 | 链接失败后再次尝试 |

MVP 规则：每个 TV 订阅每个 tick 最多执行一条 due lane，优先级为 `link > progress > search`。选择这个优先级是为了尽快完成已下载内容，同时仍允许在后续 tick 按各自间隔继续搜索。未来如需优化，可把一次 TV aggregate operation 扩展为批量执行多条 lane，但必须保持每条 lane 独立间隔和 retry。

### Search Cursor 与覆盖规则

搜索必须围绕 `search_cursor_episode`：

- 候选 torrent 必须覆盖 cursor 才能被选中。
- 候选可覆盖单集、连续范围或整季合集。
- 推送成功后创建 download task，并把标题解析得到的覆盖范围记为 `tentative coverage`。
- cursor 向后跳过已完成、已跳过、已有有效 active assignment 或 blocked assignment 的 episode。
- 当 cursor 超过 episode total 时，`search` lane 暂停，不再搜索新任务，只继续 `progress` 和 `link`。

示例：8 集 TV 中，推送 E01 成功后 cursor 变为 2；推送 E02-E04 合集后 cursor 变为 5；推送 E05-E08 或整季合集后 cursor 变为 9。

cursor 不是只能单调递增。若覆盖失效，必须重新计算为第一个未完成、未跳过、无有效 active assignment 或 blocked assignment 的 episode。例如 E02-E04 合集推送后 cursor=5，但 qB 文件列表确认该 task 缺失 E03-E04，系统必须释放 E03-E04 的有效覆盖关系，并把 cursor 重算到 3。

`blocked assignment` 表示该 episode 已经被一个 task 负责过，但该 task 或 episode 的下载/链接重试达到上限。blocked assignment 会让 cursor 继续向后搜索后续集，避免 E03 失败阻塞 E04-E08；但 blocked episode 不计入 completed，父级必须展示失败数量，直到用户手动重试成功、释放 assignment 重新搜索，或跳过该 episode。

### 覆盖可信度

合集覆盖范围不能只停留为开放问题，MVP 采用两阶段覆盖：

- `tentative_covered`：搜索结果标题、种子标题或详情解析出的覆盖范围。推送成功后可用于推进 cursor，避免重复搜索同一范围。
- `verified_covered`：qB 文件列表、文件名解析或硬链接结果确认实际包含的 episode。

如果 task 后续变为 `missing`、`superseded`，或 verified coverage 小于 tentative coverage，未被验证且未完成的 episode 必须释放 active assignment，重新进入 cursor 候选。若 task 只是系统错误达到 retry 上限，但覆盖关系仍可信，则对应 episode 进入 blocked assignment，让后续 episode 可以继续搜索。整季合集也遵守这个规则；全集标题可先 tentative 覆盖全季，但不能让未验证且未完成的 episode 永久跳过。

### Episode Assignment 与去重

episode 是目标，download task 是实现目标的下载单元。每个 episode 同一时间最多有一个 `selected_task_id` 负责完成它。

- 新 task 只应绑定尚未 completed、未 skipped、无有效 selected task 的 episode。
- 如果新 task 与已有 selected task 重叠，默认忽略重叠部分，只绑定新增有效覆盖。
- 重复 task 可记录为 `ignored` 或 `superseded`，但不能覆盖已完成 episode 的结果。
- 已完成硬链接的 episode 不被后续 task 自动替换；质量升级或重新下载属于单独流程。
- 硬链接前按 episode 锁定目标路径，避免多个 task 重复写同一目标。

assignment 有效性必须显式持久化：

- `active`：task 正在负责该 episode，cursor 跳过该 episode。
- `blocked`：task/episode 达到 retry 上限，cursor 仍跳过该 episode，但父级展示失败，completed 不成立。
- `released`：覆盖失效、用户释放或 task 被 superseded，清空 `selected_task_id`，episode 重新进入 cursor 候选。
- `completed`：episode 已硬链接完成。
- `skipped`：episode 被用户跳过，completed 条件视为不需要处理。

### TV 完成条件

父级 `completed` 不能只看 cursor。满足以下条件才完成：

- `search_cursor_episode > episode_total` 或所有目标 episode 均 completed/skipped。
- 没有 active downloading task。
- 没有 linkable 或 linking task/episode。
- 所有未 skipped 的目标 episode 均完成硬链接。

如果全集种子已推送且 cursor=9，但下载或硬链接还未完成，父级应显示 `downloading` 或 `linking`，不能显示 `completed`。

## 统一 Tick 调度

后端应有统一 tick 调度器，而不是多个接口各自隐式推进状态。

通用流程：

1. 读取订阅记录。
2. 跳过 `completed`，跳过 `skipped` 且无手动重试的记录。
3. 根据主状态和 TV lane 判断 eligible。
4. 选择一个 due operation 执行。
5. 操作成功并进入下一状态或下一 lane 阶段时，写对应父级状态或 lane 的 `next_attempt_at = now`，下一 tick 不等待。
6. 操作成功但保持同一状态或同一 lane 时，写对应父级状态或 lane 的 `next_attempt_at = now + interval`。
7. 系统错误写入对应作用域的 failure；未超限时按 retry interval 再试，超限时标记 `retry_blocked`。

电影每 tick 执行当前主状态的一次操作。TV 每 tick 最多执行一条 due lane；父级主状态在操作完成后重新派生。

搜索操作结果必须结构化：

- `pushed`：推送成功，创建 task，cursor 重新计算。
- `waiting_release_no_candidates`：无候选，保持 search lane，设置搜索间隔，不增加 retry。
- `waiting_release_no_match`：有候选但未命中规则，保持 search lane，设置搜索间隔，不增加 retry。
- `system_failed`：API、鉴权、qB 推送等系统错误，增加 retry。

## 间隔与重试规则

间隔只限制同一状态或同一 lane 的重复尝试：

- `queued -> meta`、`meta -> searching`、`searching -> downloading`、`downloading -> linking`、`linking -> completed` 成功流转后，下一 tick 可立即执行新状态或新 lane。
- `search` lane 搜不到 cursor 对应集时，按搜索间隔再次尝试。
- `progress` lane 发现 qB 未完成时，按下载进度间隔再次同步。
- `link` lane 链接失败但允许重试时，按链接重试间隔再次尝试。

失败重试按作用域分层：

- 父级 operation retry：`meta`、电影搜索、电影进度、电影链接。
- TV search lane retry：搜索 API、匹配执行、推送 qB 的系统错误。
- download task retry：某个 qB task 的查询或文件列表同步错误。
- episode link retry：某个 episode 或 task 的硬链接错误。

一个 episode 或 task 达到 retry 上限，只阻塞该作用域，不阻塞其他 task 的下载同步或其他 episode 的链接。父级可聚合展示 `has_failed_children` 或 `retry_blocked_count`。

## 失败、跳过与手动操作

### 失败

`failed` 是标签，不是主状态。失败信息至少包含：

- 失败作用域：parent operation、lane、download task、episode。
- 失败发生的主状态或 lane。
- 错误类型和错误消息。
- `retry_count`、`max_retries`。
- 最近失败时间和下次自动重试时间。
- 是否已 `retry_blocked`。

成功操作只清除同一作用域的失败，不应误清除其他 task 或 episode 的失败。

### 跳过

MVP 支持：

- `skip_subscription`：整个订阅进入 `queued(skipped)`，不参与自动 tick。
- `unskip_subscription`：清除父级 skipped，从 `meta` 开始或从已有状态继续，取决于是否已有有效元数据。
- `skip_episode/range`：TV episode 标记 skipped；completed 条件把 skipped episode 视为不需要处理。
- `unskip_episode/range`：清除 episode skipped，cursor 重算到第一个未完成且无有效覆盖的 episode。

跳过已绑定 task 的 episode 时，不取消 qB task。系统应释放该 episode 的 selected assignment，禁止对该 episode 执行硬链接；同一 task 覆盖的其他未跳过 episode 继续下载和链接。如果 task 只剩 skipped episode，则该 task 可保留为执行历史，不再进入 link lane。

取消已推送 qB task、自动删除 qB 任务、质量升级不属于 MVP。

### 手动重试与重新运行

手动重试的语义是忽略当前间隔和阻塞标签，让对应作用域立即 eligible：

- `queued(skipped)`：清除 skipped，进入 `meta`。
- `searching(waiting_release)`：立即重试 search lane。
- `searching(failed/retry_blocked)`：清除 search lane 阻塞，立即重试；retry 计数保留历史，但本次允许强制执行。
- `downloading` task failed：立即同步该 task。
- `linking` episode/task failed：立即执行硬链接。
- `completed` 默认不提供重试。

重新运行是更强操作，MVP 只定义为后续能力：可选择从 `meta` 或 `searching` 重建部分执行数据，但不能默认删除已完成硬链接结果。

## 数据模型建议

本节描述产品和工程边界，不规定具体 SQL。

### 持久化边界

- 父级订阅表应保留可查询字段：主状态、执行状态、关注标签摘要、next_attempt_at、media type、subject_id、updated_at。
- TV episode/task 可以存独立表或 JSON，但必须有 schema version 和可迁移结构。
- 若继续存 JSON，eligible 扫描所需字段必须冗余到父级索引字段，避免每次 tick 全量解析大 JSON。
- 迁移应支持失败回滚：旧 record_json 不被删除，新字段可由旧字段重复生成。

### 订阅父级记录

建议字段：

- `id` / `subject_id`
- `media_kind`: movie / tv
- `state`: `queued` / `meta` / `searching` / `downloading` / `linking` / `completed`
- `execution_state`: `idle` / `running`
- `attention_tags`
- 元数据缓存：标题、原名、年份、封面、发布日期、简介、评分等
- TV 目标范围：season、episode total、目标 episode 集合
- `search_cursor_episode`
- lane 调度摘要：`search_next_attempt_at`、`progress_next_attempt_at`、`link_next_attempt_at`
- `state_entered_at`、`last_tick_at`
- 父级 failure/skip 摘要

### Episode Record

建议字段：

- 订阅 ID、season number、episode number、episode label
- target state：target / skipped / completed
- coverage state：uncovered / tentative_covered / verified_covered
- assignment state：none / active / blocked / released / completed / skipped
- selected task ID
- download state：not_started / downloading / downloaded / failed
- link state：not_linked / linking / linked / failed
- retry/failure 信息
- timestamps

### Download Task Record

建议字段：

- 订阅 ID、task ID
- torrent ID/title/source、qB server/category/save path/hash/name
- task state：pushed / downloading / downloaded / linking / completed / missing / failed / ignored / superseded
- tentative coverage、verified coverage
- pushed_at、checked_at、completed_at
- 下载进度、文件列表、qB 原始状态快照
- 硬链接结果文件列表
- retry/failure 信息

download task 是执行记录，不是目标。TV 父级完成以 episode records 为准。

## API / 前端展示影响

### API

订阅列表和详情 API 应逐步暴露：

- 父级主状态、执行状态、关注标签摘要。
- TV 的 `search_cursor_episode`、waiting episode、目标总集数、已覆盖/已下载/已链接/失败数量。
- TV episode records 和 download task records。
- lane 级 next attempt 和失败摘要。
- 手动重试接口表达为“使指定作用域立即 eligible”，作用域可以是 parent、lane、task、episode。

过渡期可保留旧 `status` 字段，但必须由新状态派生，不能继续作为后端状态机权威来源。

### 前端

前端展示从单 badge 升级为生命周期 + 标签 + 进度：

- 列表卡片展示父级派生状态，例如“搜索中”“下载中”“硬链接中”“已完成”。
- 辅助文案展示具体标签，例如“等待 E05 发布”“2 个分集失败”“下次搜索 30 分钟后”。
- TV 详情展示 episode 矩阵：未覆盖、等待发布、下载中、已下载、链接中、已链接、失败、跳过。
- TV 详情展示 download tasks：种子标题、覆盖集数、qB 进度、链接结果。
- 手动重试按钮应说明作用域：重试搜索、重试下载同步、重试硬链接、取消跳过。

## 迁移策略

迁移应保证现有订阅记录可读、可展示、可继续处理。

基础映射：

| 旧主状态 | 新主状态 | 标签/说明 |
| --- | --- | --- |
| `unprocessed` | `queued` | `idle` |
| `matching` | `searching` | 可附加 `running` 或搜索 lane 信息 |
| `processing` | `searching` | 表示已命中候选并正在推送 |
| `pushed` | `downloading` | 已推送，等待进度同步 |
| `downloading` | `downloading` | 保留下载进度 |
| `completed` | `linking` | qB 下载完成，等待硬链接 |
| `linked` | `completed` | 已完成硬链接 |
| `skipped` | `queued` | `skipped` |

旧 `failed` 记录按确定性顺序推断：

1. `last_completion.status = completed`：迁移为 `completed`。
2. `last_completion.status = failed`：迁移为 `linking(failed)`。
3. `last_completion.status = pending`：迁移为 `downloading`。
4. `last_push.status = linked/completed`：迁移为 `completed` 或 `linking`，取决于硬链接记录是否完整。
5. `last_push.status = downloaded`：迁移为 `linking`。
6. `last_push.status = downloading/pushed`：迁移为 `downloading`。
7. `last_push.status = failed` 且错误为无候选/无匹配：迁移为 `searching(waiting_release)`。
8. `last_push.status = failed` 且为 qB/M-Team 系统错误：迁移为 `searching(failed)`。
9. `processing_stage = link_failed/download_complete/link_planned`：迁移为 `linking`，保留失败或计划标签。
10. `processing_stage = downloading/pushed`：迁移为 `downloading`。
11. `processing_stage = no_candidates/no_match/searching/matched/pushing/push_failed`：迁移为 `searching`，其中 no_candidates/no_match 迁移为 `waiting_release`，push_failed 迁移为 `failed`。
12. 仅有 `candidate_matches`：迁移为 `searching`。
13. 无法判定：迁移为 `queued(needs_reconciliation)`，不自动推进，等待刷新或手动处理。

旧 TV 记录只有单个 `last_push/last_completion` 时：

- 可从 qB 文件列表或旧 episode progress 推断 episode records 和 task coverage。
- 无法确认覆盖范围的 task 标记为 tentative，不推进 cursor 超过 verified coverage。
- 迁移后 cursor 重算为第一个未完成、未跳过、无有效 active assignment 的 episode。

## 验收标准

- 主状态只包含 `queued`、`meta`、`searching`、`downloading`、`linking`、`completed`。
- `failed`、`skipped`、`running`、`idle` 不作为主状态持久化或驱动状态机。
- 电影新订阅能按 `queued -> meta -> searching -> downloading -> linking -> completed` 流转。
- 成功流转到下一主状态后，下一 tick 可立即处理新状态，不受新状态间隔限制。
- 状态保持不变时，下一次自动尝试遵守该状态或 lane 的间隔。
- `queued(skipped)` 手动重试后不丢元数据，并进入 `meta` 或已有有效元数据后的下一阶段。
- 搜索无候选和无匹配返回结构化 waiting result，不增加 retry，不进入 failed。
- TV 搜索候选必须覆盖 `search_cursor_episode`。
- TV 推送 E01 成功后 cursor 变为 2；推送 E02-E04 合集成功后 cursor 变为 5；当 cursor=8 时推送 E08 后 cursor 变为 9；当 cursor=5 时只有 E05-E08 或全集可推进到 9。
- TV cursor 超过总集数后不再搜索新任务，但继续下载和硬链接已有任务。
- TV 存在待链接 E01 且 E05 未覆盖时，父级可显示 `linking`，但 search lane 到期后仍会继续搜索 E05。
- TV 的 link/progress/search 同时 due 时，一个 tick 只执行 link lane；未执行的 progress/search lane 不应被更新 next_attempt。
- E02-E04 合集推送后若 task missing 或 verified coverage 只包含 E02，则 E03-E04 释放覆盖，cursor 重算到 3。
- E03 的 task/episode 达到 retry 上限时，E03 进入 blocked assignment，父级展示失败；cursor 可以继续跳过 E03 搜索 E04-E08，但父级不能 completed，直到 E03 成功、释放重搜或被跳过。
- 一个 episode 同一时间最多有一个 selected task；重复 task 不覆盖已完成 episode。
- TV 父级 `completed` 只能在所有未跳过目标 episode 完成硬链接后出现，不能只看 cursor。
- TV 在元数据未 ready、episode records 未初始化或目标集合未知时，不能派生为 `searching` 或 `completed`。
- TV 搜不到 cursor 对应集时展示为等待具体 episode 发布，例如“等待 E05 发布”。
- task/episode 级失败达到上限只阻塞对应作用域，不阻塞其他 task 同步、其他 episode 链接或后续可搜索 episode。
- API、qB、硬链接等系统错误计入对应作用域 retry；达到上限后停止自动重试并展示可手动重试。
- 迁移旧 `failed` 记录时按 `last_completion`、`last_push`、`processing_stage`、`candidate_matches` 的优先级推断，无法判定的记录进入 `needs_reconciliation`。

## 开放问题

- TV 订阅目标是否始终限定为单季，还是需要支持一个父级订阅跨多季？
- episode 总数和季信息应以豆瓣、TMDB 还是用户配置为准？当来源不一致时如何裁决？
- 手动重试 `retry_blocked` 后 retry 计数是否在 UI 上清零展示，还是保留历史但允许本次强制执行？
- 迁移期旧 `status` 字段保留多久，前端何时完全切换到新状态模型？
