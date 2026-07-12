---
status: accepted
owner: tmdb-mteam-hub
last_verified: 2026-07-12
implementation_status: in_progress
workstream: safety-configuration-automation
source_prd: docs/superpowers/specs/2026-07-11-project-architecture-convergence-prd.md
depends_on: []
blocks:
  - backend-boundaries-application-services
  - subscription-storage-scheduler
  - frontend-modularization-contracts
---

# Safety, Configuration, and Automation Implementation Plan

> **执行方式：** 按任务顺序逐项实施。每个任务都必须先添加会失败的测试，再实现，最后只提交该任务列出的文件。不得把存储收敛、前端页面拆分或视觉清理混入本计划。

**目标：** 先收紧管理面和自动化默认行为，保证配置文件不会因解析、迁移或并发更新而丢失；所有 qB 操作只使用已配置服务器；未实现的 TV 不进入调度；完整豆瓣想看快照中消失的记录停止自动处理。

**总体策略：** 保持当前单体部署和现有 API 路径，先建立安全边界与配置协调器。配置仍由一个 TOML 文件持久化，但读取、校验、备份、原子写入和并发 patch 必须只有一个入口。静态前端可公开加载，除登录/鉴权状态外的 `/api` 路由统一受管理凭据保护。自动 watcher 默认关闭；dry-run 可执行轮询、候选匹配和硬链接计划，但不得调用 qB 添加种子或 `std::fs::hard_link`。

**技术栈：** Rust 2021、Axum 0.8、Tokio、Serde/TOML、tower-http、rusqlite、Vue 3、Vitest、Vue Test Utils、happy-dom。

---

## 范围与跨计划边界

本计划负责：

- 配置解析 fail-fast、未知字段策略、按需规范化、迁移备份、原子替换和 `0600` 权限。
- 配置更新的 revision/patch 语义，以及设置保存和豆瓣 QR Cookie 更新之间的并发安全。
- 默认 loopback、显式 CORS allowlist、管理 token、登录 Cookie、配置脱敏。
- qB test/push API 的 `server_id` 边界。
- watcher `enabled = false`、`dry_run = true`、显式启用和持久化轮询调度信息。
- TV executor 未完成期间的硬性 parking。
- 完整想看快照驱动的 inactive/reactivate 语义。
- 完成上述能力所需的最小前端设置与登录交互，以及升级说明。

以下内容交给其他计划，不在这里顺手实现：

- `main.rs` 全面拆分、`build_router`、统一 application service 和 adapter error。
- subscription blob/per-record 双写收敛、claim/lease、revision/attempt 和摘要/详情 API。
- `App.vue` 页面化、RouterView、Vitest/Playwright 全面迁移和样式治理。
- Docker 多阶段前端构建、状态卷拆分、完整 CI/运维文档治理。

为了避免阻塞安全修复，本计划可在 `src/main.rs` 保留 handler；后续 backend plan 只移动已验证行为，不重新设计安全语义。

## 固定安全决策

实施时不得临时更换以下产品语义：

1. 新安装默认 `listen_ip = "127.0.0.1"`。
2. 非 loopback 监听且未配置有效管理 token 时，服务拒绝启动。
3. loopback 且无 token 时只允许 loopback peer 进入一次性本地 bootstrap 模式；设置 token 后立即进入强制鉴权模式。
4. 管理 token 至少 24 个字符，保存在权限受限的 TOML 中，但任何 GET/日志/错误都不得返回它。
5. 浏览器通过 `/api/auth/login` 换取 host-only、`HttpOnly`、`SameSite=Strict`、`Path=/api` Cookie；API 同时接受 `Authorization: Bearer` 供 CLI 使用。
6. 默认不安装 CORS layer；配置了精确 origins 时才允许这些 origin，永不使用 `Any` 或 `*`。
7. 未识别 TOML 字段明确报错，不静默丢弃。已有未知字段必须由用户修正后才能启动。
8. 新安装 watcher 为 `enabled = false`、`dry_run = true`；从 false 切换到 true 必须有 API 级确认字段。
9. dry-run 可以访问豆瓣/M-Team/qB 只读接口并写审计/计划状态，但不得添加 torrent 或创建目录/硬链接。
10. TV 在完整 executor 交付前保留数据模型，但 `select_due_operation` 永远不返回 TV operation。
11. 只有被证明完整的豆瓣想看快照才能把缺失记录设为 inactive；截断或失败的快照不得停用任何记录。

## 任务依赖与提交边界

| 任务                                | 依赖             | 独立提交                                                   | 主要风险                       |
| ----------------------------------- | ---------------- | ---------------------------------------------------------- | ------------------------------ |
| 1. 安全配置存储                     | 无               | `fix: make config loading fail safe`                       | 配置无法启动、权限差异         |
| 2. 原子配置协调器                   | 1                | `fix: serialize config patches`                            | 大量读取调用机械迁移           |
| 3. 监听、鉴权与 CORS                | 1、2             | `feat: secure the management plane`                        | 远程部署升级后被安全锁定       |
| 4. 配置 DTO 与秘密 patch            | 2、3             | `fix: redact configuration secrets`                        | 空值误清除凭据                 |
| 5. qB server_id 边界                | 4                | `fix: bind qb actions to configured servers`               | 未保存服务器不能再直接测试     |
| 6. watcher 开关、dry-run 与轮询状态 | 2、4             | `feat: make subscription automation opt in`                | dry-run 仍触发副作用、每秒循环 |
| 7. TV 调度 parking                  | 6                | `fix: park unsupported tv automation`                      | TV guard 与 selector 漂移      |
| 8. 想看 inactive/reactivate         | 6                | `fix: deactivate subscriptions absent from complete polls` | 截断快照误停用                 |
| 9. 前端安全与自动化控件             | 3、4、5、6、7、8 | `feat: expose safe management controls`                    | 脱敏表单误覆盖秘密             |
| 10. 示例配置与升级说明              | 1-9              | `docs: document secure bootstrap and automation`           | Docker 监听与宿主绑定混淆      |
| 11. 全量验证与迁移演练              | 1-10             | 不单独提交；只验证                                         | 只跑窄测试造成误判             |

每个提交前运行 `git diff --check`，并确认 `git status --short` 中没有把其他代理创建的计划或现有未跟踪文件加入暂存区。

## Implementation progress

Last updated: 2026-07-11.

- Completed with code and tests: Tasks 1-5, covering fail-fast/atomic configuration storage,
  serialized revision-aware updates, management authentication/CORS, redacted secret patch DTOs,
  and qB actions restricted to configured `server_id` values.
- Subscription diagnostics now reuse one `SubscriptionDiagnosticRedactor` instead of separate or
  identity mappers. It is built from validated configuration, redacts TMDB/M-Team keys, Douban Cookie
  and components, management token, qB passwords, URL userinfo, and sensitive query values, then trims
  and caps output. URL structure is sanitized before configured-string replacement, closing the
  combined leak where a short or syntax-shaped configured secret could destroy URL delimiters before
  dynamic userinfo/passkeys were removed. The latest Poll failure path and production subscription
  detail DTO path both use this implementation.
- The production list/detail HTTP adapter reads one `ConfigManager` snapshot per request and derives both
  the Douban account scope and its concrete redactor from that same snapshot. A hot-update test proves
  cursor scope and secret redaction change together. These routes are registered through the
  authenticated production router and use the latest repository from `AppState`.
- Completed frontend contract slice: AuthGate prevents pre-auth application requests; Settings uses
  redacted snapshots, required `expected_revision`, Keep/Set/Clear semantics for configured secrets
  and qB passwords, saved-server-only qB test/push payloads, and `cookie_saved` QR completion without
  receiving or round-tripping the Douban Cookie. Management-token Set/Clear now consumes only the
  redacted `has_admin_token` flag; a replacement is used once to refresh the HttpOnly session and is
  then cleared from the draft. Any protected 401 returns the mounted app to AuthGate.
- Login failures are rate-limited per direct peer (five failures in five minutes, then a 15-minute
  block with `Retry-After`). Cookie-authenticated mutations require browser-controlled
  `Sec-Fetch-Site: same-origin`; Bearer callers are unaffected, and explicit cross-site loopback
  bootstrap mutations fail closed.
- Tasks 6-8 are preserved by the latest Poll runtime:
  watcher defaults are
  `enabled=false`/`dry_run=true`, false -> true requires explicit API/UI confirmation, disabled mode
  performs no upstream Poll, scheduler claim or external effect, poll attempt/backoff/generation
  survives restart,
  dry-run never generates a download token/adds a torrent/creates a hardlink, TV is parked as
  `tv_not_supported`, and only a proven complete snapshot can deactivate missing records.
- Task 9 behavior is complete: AuthGate, redacted Settings/qB/QR flows, watcher enabled/dry-run
  controls, the first-enable confirmation, and inactive/TV/schedulable capability display are
  complete. Unported subscription side-effect endpoint exports, store commands, events and buttons are
  deleted, leaving list/detail history read-only, and the routes display the runtime
  disabled/dry-run/live watcher mode without inferring an unloaded configuration. Management-token
  rotation and stale-session handling are covered by form/store/mounted-gate tests.
- Task 10 is complete: Compose defaults to host loopback, explicit LAN opt-in is documented, and the
  dedicated security-bootstrap runbook covers install, upgrade, login, token recovery/rotation, CORS,
  watcher dry-run, and atomic backup restoration. Task 11 remains open until the full combined gate and
  manual destructive-config/security smoke matrices are exercised.
- Verification baseline: the service-seam, watcher-default, and persisted-poll batches passed their
  Rust formatting, strict Clippy, and all-target gates; the completed frontend page/store batches
  passed `npm run verify:frontend`; the combined scheduler/claim, running-detail gate, Poll supersede,
  HTTP/health/repository and latest-only cleanup now pass 302 unit, 9 router-contract, and 9
  upstream-contract tests,
  while the frontend passes 247 Vitest cases across 43 files plus checked API types through one runner. Strict Clippy, Rust
  formatting, and diff checks are green; lifecycle status remains `in_progress` because the remaining
  security/UI/manual acceptance work is open.

---

### Task 1: 建立 fail-fast、可备份、权限受限的配置存储

**Files:**

- Modify: `src/config.rs`
- Modify: `src/main.rs`
- Modify: `config.example.toml`
- Add: `tests/fixtures/config/valid-pre-safety.toml`
- Add: `tests/fixtures/config/malformed.toml`
- Add: `tests/fixtures/config/unknown-field.toml`

**行为要求：**

- `FileConfig::load_or_create` 不再用默认值吞掉 TOML 错误。
- 错误必须包含配置路径、行和列；原文件字节、mtime 和 inode 不发生变化。
- 所有配置 struct 显式拒绝未知字段。
- 缺失的新字段可由 serde default 补齐；只有规范化后的语义确实变化时才迁移写回。
- 迁移前创建 `config.toml.bak.<unix-seconds>`，验证备份字节等于源文件后才替换。
- 新配置、临时文件和备份在 Unix 上均为 `0600`；目录不放宽已有权限。
- 写入采用同目录临时文件、`sync_all`、原子 rename；失败保留原文件并清理可识别的临时文件。
- 启动不再无条件调用 `save`。

- [x] **Step 1: 先写失败测试**

在 `src/config.rs` 的测试模块新增以下测试族：

- `config_store_reports_malformed_toml_with_line_and_column`
- `config_store_never_replaces_malformed_existing_file`
- `config_store_rejects_unknown_top_level_and_nested_fields`
- `config_store_does_not_write_when_valid_config_needs_no_normalization`
- `config_store_backs_up_before_normalization_write`
- `config_store_keeps_source_when_atomic_replace_fails`
- `config_store_creates_secret_files_with_mode_0600`（`#[cfg(unix)]`）
- `default_listener_is_loopback`

fixtures 中只放假值，不放真实 API key、Cookie 或 qB 密码。`valid-pre-safety.toml` 应模拟旧版本缺少后续字段但结构合法的配置。

- [x] **Step 2: 运行测试并确认失败原因正确**

Run:

```bash
cargo test config::tests::config_store_
cargo test config::tests::default_listener_is_loopback
```

Expected before implementation: malformed TOML 被吞掉、未知字段被忽略、默认地址仍为 `0.0.0.0`，至少上述断言失败；不得因为 fixture 路径拼错而失败。

- [x] **Step 3: 实现配置错误和存储结果类型**

在 `src/config.rs` 中引入明确类型，例如：

- `ConfigError::{Read, Parse, Validate, Backup, Serialize, Persist}`
- `ConfigLoadOutcome { config, migration, backup_path }`
- `ConfigMigration::{None, Created, Normalized}`

保留 `FileConfig` 作为数据模型，但将加载和保存细节集中在私有 helper。解析错误用 `toml::de::Error::span()` 计算行列，不把原始配置内容拼进错误消息。

给顶层和所有嵌套配置 struct 增加 `#[serde(deny_unknown_fields)]`。将 `default_listen_ip()` 改为 `127.0.0.1`。

- [x] **Step 4: 实现按需迁移、备份和权限**

比较“原始反序列化结果”和“校验/规范化结果”；只有确有字段补齐、ID 规范化或安全默认迁移时才调用带备份的保存。普通启动不重写格式不同但语义相同的 TOML。

Unix 使用 `OpenOptionsExt::mode(0o600)` 创建文件，并在 rename 后再次校验最终 mode。非 Unix 保持可编译，并在文档中说明权限由平台控制。

在 `src/main.rs::run` 删除启动时无条件 `file_cfg.save(&config_path)` 的路径；任何配置加载或迁移错误直接返回并退出非零。

- [x] **Step 5: 更新示例中的安全默认值**

仅在此任务先更新 `config.example.toml` 的 `listen_ip` 为 `127.0.0.1`，并注明现有显式 `0.0.0.0` 不会自动改写。管理 token 和 watcher 字段留到对应任务补齐。

- [x] **Step 6: 运行目标测试**

Run:

```bash
cargo fmt --check
cargo test config::tests::config_store_
cargo test config::tests::default_listener_is_loopback
```

Expected: PASS。另用临时目录运行一次 malformed fixture，确认命令退出非零且 `sha256sum` 前后相同。

- **Delivery action after explicit Git adoption (former Step 7):**

```bash
git add src/config.rs src/main.rs config.example.toml tests/fixtures/config
git commit -m "fix: make config loading fail safe"
```

**兼容/回滚：** 未知字段从“忽略”变为“拒绝”是有意的兼容收紧。回滚实现前先保留 `.bak.<timestamp>`；不得通过删除损坏配置来恢复启动。

---

### Task 2: 用单一配置协调器消除并发覆盖

**Files:**

- Modify: `src/config.rs`
- Modify: `src/main.rs`

**行为要求：**

- `AppState` 不再暴露“clone 配置 -> 文件 save -> RwLock 覆盖”的写法。
- 所有写入经过一个 `ConfigManager`，在同一互斥区内执行：读取当前值、检查 revision、应用 patch、校验、原子保存、替换内存、revision + 1。
- 文件写失败时内存值和 revision 不变。
- QR 登录只 patch `douban_cookie`，设置保存未显式提交的 secret 保持原值。
- 两个设置页面并发全量编辑使用 `expected_revision`；过期 revision 返回 conflict，不静默覆盖。

- [x] **Step 1: 先写失败测试**

在 `src/config.rs` 新增异步测试：

- `config_manager_failed_persist_keeps_memory_and_revision`
- `config_manager_rejects_stale_revision`
- `config_manager_qr_cookie_patch_preserves_concurrent_settings_change`
- `config_manager_settings_patch_preserves_cookie_when_secret_is_omitted`
- `config_manager_serializes_two_disjoint_patches`

测试必须使用真实临时 TOML，而不是只测内存 struct。

- [x] **Step 2: 运行并确认当前 race 模型无法满足测试**

Run:

```bash
cargo test config::tests::config_manager_
```

Expected before implementation: `ConfigManager`/patch 类型不存在，测试编译失败。

- [x] **Step 3: 实现 `ConfigManager` 与 patch API**

在 `src/config.rs` 增加：

- `ConfigSnapshot { revision: u64, value: FileConfig }`
- `ConfigManager { path, state: tokio::sync::Mutex<ConfigSnapshot> }`
- `snapshot()`
- `update(expected_revision, reason, closure)`
- `patch_douban_cookie(cookie)`

revision 只承担进程内并发控制，从 1 开始；重启后重置是允许的。所有 handler 读取改为 `state.config.snapshot().await`，写入只调用 manager。不要在 manager 中依赖 Axum `ApiError`。

- [x] **Step 4: 改造两条现有写路径**

在 `src/main.rs`：

- `put_config` 使用 manager 的 revision-aware update。
- `douban_qr_poll` 使用 `patch_douban_cookie`；响应不再携带 `cookie_header`，只返回 `cookie_saved: true`。
- 删除所有直接 `new_cfg.save(...)` 和 `*state.config.write().await = ...`。

用 `rg` 检查没有旁路：

```bash
rg -n "\.save\(&state\.config_path\)|state\.config\.write\(\)" src
```

Expected: 无匹配。

- [x] **Step 5: 运行测试**

Run:

```bash
cargo fmt --check
cargo test config::tests::config_manager_
cargo test
```

Expected: PASS。

- **Delivery action after explicit Git adoption (former Step 6):**

```bash
git add src/config.rs src/main.rs
git commit -m "fix: serialize config patches"
```

**兼容/回滚：** stale update 从 last-write-wins 变为 HTTP 409；前端必须在 Task 9 处理并重新加载。若需要回滚，只能回滚代码，不能用旧进程覆盖新进程已经写入的配置。

---

### Task 3: 收紧监听面，加入管理鉴权并移除任意 CORS

**Files:**

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `src/config.rs`
- Add: `src/http/mod.rs`
- Add: `src/http/auth.rs`
- Add: `src/http/security.rs`
- Modify: `src/main.rs`

**行为要求：**

- 新增 `[management]`：`admin_token`、`allowed_origins`、`secure_cookie`。
- token 少于 24 字符、URL 中包含 userinfo、origin 不是完整 `http(s)://host[:port]` 均校验失败。
- 非 loopback + 空 token 启动失败；loopback + 空 token 只允许 loopback peer bootstrap。
- `/api/auth/status`、`/api/auth/login`、`/api/auth/logout` 是唯一未登录 API；未来 health endpoint 由运维计划单独豁免。
- 登录成功设置 host-only `HttpOnly; SameSite=Strict; Path=/api` Cookie；`secure_cookie=true` 时增加 `Secure`。
- token 比较使用固定长度摘要和 constant-time equality；不得把 token 写入 tracing 或 operation log。
- 默认完全无 CORS header；显式 allowlist 精确匹配，拒绝 wildcard、`null` 和带 path 的 origin。

- [x] **Step 1: 先写失败测试**

在 `src/http/auth.rs` 和 `src/http/security.rs` 添加可对小型 Axum Router 运行的测试：

- `management_api_rejects_missing_cookie_when_token_configured`
- `management_api_accepts_valid_login_cookie`
- `management_api_accepts_valid_bearer_token`
- `management_api_rejects_wrong_token_without_echoing_it`
- `local_bootstrap_requires_loopback_peer`
- `non_loopback_listener_requires_admin_token`
- `same_origin_mode_emits_no_cors_headers`
- `cors_allows_only_configured_exact_origin`
- `cors_rejects_wildcard_and_null_origin`
- `login_cookie_has_httponly_samesite_and_api_path`

为 Router `oneshot` 测试在 dev-dependencies 增加精确版本的 `tower` util；认证摘要使用 `sha2` 与 `subtle`，不要复用 torrent 的 SHA-1。

- [x] **Step 2: 运行测试并确认鉴权/CORS 尚不存在**

Run:

```bash
cargo test http::auth::tests::
cargo test http::security::tests::
```

Expected before implementation: 新模块或类型不存在，测试编译失败。

- [x] **Step 3: 实现管理安全配置和启动校验**

在 `src/config.rs` 增加 `ManagementConfig` 及默认值。`FileConfig::validate` 必须验证监听地址与 token 的组合，并返回明确修复提示：改为 loopback，或设置高熵 token 后再开放监听。

`allowed_origins` 仅作为启动时配置；设置后响应应标记 `restart_required`，不得热替换半个 Router。

- [x] **Step 4: 实现 auth middleware 和登录端点**

在 `src/http/auth.rs` 实现：

- 从 manager snapshot 读取当前 token，因此 token 更新后旧 Cookie 立即失效。
- bearer 和 Cookie 两种凭据解析。
- loopback bootstrap 检查 `ConnectInfo<SocketAddr>`。
- 登录、登出和状态 DTO；状态只返回 `authenticated`、`token_configured`、`bootstrap_allowed`。

在 `src/main.rs` 使用 `into_make_service_with_connect_info::<SocketAddr>()` 提供 peer 信息，并把 auth layer 只包在 `/api` 管理 Router 上；静态资源仍可加载登录界面。

- [x] **Step 5: 实现无默认 CORS 的安全层**

删除 `CorsLayer::allow_origin(Any).allow_methods(Any).allow_headers(Any)`。`src/http/security.rs` 只在 allowlist 非空时构造 layer，并限制 methods/headers/credentials 到实际需要的集合。

用下面的 source guard 确认 wildcard 消失：

```bash
rg -n "allow_origin\(Any\)|allow_methods\(Any\)|allow_headers\(Any\)" src
```

Expected: 无匹配。

- [x] **Step 6: 运行测试**

Run:

```bash
cargo fmt --check
cargo test http::auth::tests::
cargo test http::security::tests::
cargo test
```

Expected: PASS。

- **Delivery action after explicit Git adoption (former Step 7):**

```bash
git add Cargo.toml Cargo.lock src/config.rs src/http src/main.rs
git commit -m "feat: secure the management plane"
```

**兼容/迁移：** 现有配置若显式监听 `0.0.0.0` 且无 token，升级后会拒绝启动，这是安全门而不是回归。Task 10 必须在发布前给出预升级操作。禁止以“临时恢复”为由重新允许 LAN 上的无鉴权 API。

---

### Task 4: 把配置 API 改为脱敏 DTO 和显式 secret patch

**Files:**

- Modify: `src/main.rs`
- Modify: `src/config.rs`

**行为要求：**

- `GET /api/config` 永不返回 TMDB key、M-Team key、豆瓣 Cookie、管理 token 或 qB password。
- 返回 `revision`、`has_tmdb_api_key`、`has_mteam_api_key`、`has_douban_cookie`、`has_admin_token`。
- qB 只返回 `id/name/base_url/username/insecure_tls/has_password`；拒绝 base URL 内嵌 userinfo。
- PUT 使用 Keep/Set/Clear 语义。字段省略代表保留；清空必须显式 `clear_* = true`。
- qB password 按稳定 server ID 合并：同 ID 且 password 省略时保留；新 ID 未给 password 时为空；删除 server 必须通过新的 server 列表明确表达。
- `expected_revision` 必填；冲突返回 409，不写文件。

- [x] **Step 1: 先写失败测试**

在 `src/main.rs` 测试模块新增：

- `config_response_never_serializes_any_secret_value`
- `config_response_uses_has_flags_and_redacted_qb_servers`
- `config_patch_omitted_secrets_are_kept`
- `config_patch_clear_secret_is_explicit`
- `config_patch_preserves_existing_qb_password_by_id`
- `config_patch_rejects_stale_revision_without_writing`
- `qr_poll_response_does_not_return_cookie_header`

测试应使用哨兵 secret（如 `SECRET_MUST_NOT_LEAK_...`），并对整个响应 JSON 字符串做负断言。

- [x] **Step 2: 运行并确认当前 GET 会泄露哨兵值**

Run:

```bash
cargo test tests::config_response_
cargo test tests::config_patch_
cargo test tests::qr_poll_response_does_not_return_cookie_header
```

Expected before implementation: GET 响应包含 secret，或新 DTO/patch 类型不存在。

- [x] **Step 3: 定义明确 DTO 和 patch 合并函数**

在 `src/main.rs` 暂时定义 HTTP DTO：

- `ConfigResponse`
- `RedactedQbServerResponse`
- `PutConfigBody`
- `SecretPatch`
- `QbServerPatch`

DTO 后续由 backend plan 移到 `src/http/handlers/config.rs`；本任务不提前做整套 handler 拆分。

把合并逻辑写成无 I/O 纯函数并先通过单元测试，再交给 `ConfigManager::update` 保存。

- [x] **Step 4: 返回脱敏响应并统一 revision**

GET 和成功 PUT 都返回同一 `ConfigResponse` 形状。PUT 响应包含 `restart_required` 和变更后的 revision，前端可立即刷新保存基线。

扫描泄密字段：

```bash
rg -n '"(tmdb_api_key|mteam_api_key|douban_cookie|admin_token|password)"\s*:' src/main.rs
```

Expected: 只允许 request 解析、内部构造或测试 fixture；任何 response JSON 构造都不得命中。

- [x] **Step 5: 运行测试**

Run:

```bash
cargo fmt --check
cargo test tests::config_response_
cargo test tests::config_patch_
cargo test tests::qr_poll_response_does_not_return_cookie_header
```

Expected: PASS。

- **Delivery action after explicit Git adoption (former Step 6):**

```bash
git add src/config.rs src/main.rs
git commit -m "fix: redact configuration secrets"
```

**兼容/迁移：** 旧前端依赖 secret 回填，必须与 Task 9 同一发布批次交付。不要让空密码输入在旧客户端 round-trip 时清除已有密码。

补充交付证据（2026-07-11）：配置响应 DTO 的脱敏与运行时诊断脱敏是两个边界，但复用同一份
validated `FileConfig` 语义。订阅诊断统一使用 `app/redaction.rs` 的 config-aware redactor；先解析并
清理 URL userinfo/敏感 query，再按最长优先替换 configured secrets，避免组合泄漏。HTTP staged
list/detail 每个请求只读取一次 `ConfigManager` snapshot，并从该 snapshot 同时派生 account scope 与
redactor。production router 尚未接入该 staged slice。

---

### Task 5: 把 qB test/push 限制到已配置 `server_id`

**Files:**

- Modify: `src/lib.rs`
- Modify: `frontend/src/features/settings/form-model.js`
- Modify: `frontend/src/features/settings/api.js`
- Add: `frontend/src/__tests__/settings-form-model.vitest.js`
- Add: `frontend/src/__tests__/settings-contract.vitest.js`

**行为要求：**

- `/api/qb/test` body 只接受 `{ "server_id": "..." }`。
- `/api/qb/push-mteam` body 只接受 `server_id/torrent_id/category/savepath`。
- handler 从同一个 config snapshot 按 ID 解析完整服务器；未知/空/重复 ID 被拒绝。
- 请求 JSON 中出现 `server/base_url/username/password/insecure_tls` 应由 `deny_unknown_fields` 拒绝，不可悄悄忽略。
- 前端未保存的新服务器显示“先保存后测试”，不向 qB endpoint 发送凭据。

- [x] **Step 1: 先写失败测试**

Rust tests：

- `qb_test_request_rejects_inline_server_credentials`
- `qb_push_request_rejects_inline_server_credentials`
- `qb_actions_resolve_exact_configured_server_id`
- `qb_actions_reject_unknown_server_id_before_network_io`

Vitest 在 `settings-form-model.vitest.js` 直接 import
`frontend/src/features/settings/form-model.js`：

- `qbTestPayload` 只返回 `server_id`。
- `qbPushPayload` 不包含 server object 或 secret。
- redacted qB form 转 payload 时，未填写 password 产生 Keep 语义。

- [x] **Step 2: 运行并确认当前 API 接受完整 server**

Run:

```bash
cargo test tests::qb_actions_
cargo test tests::qb_test_request_rejects_inline_server_credentials
cargo test tests::qb_push_request_rejects_inline_server_credentials
npm test -- settings-form-model settings-contract
```

Expected before implementation: Rust 请求模型仍含 `QbServerEntry`，Node helper 不存在。

- [x] **Step 3: 实现后端 ID 解析**

在 HTTP handler 中从 config snapshot 按 `server_id` 解析 server。`QbPushMteamBody` 删除
`server`，加入 `server_id`。解析 helper 必须在任何 M-Team/qB 网络调用之前返回错误。

订阅自动推送已经通过分类中的 `qb_server_id` 选择，不改变其现有含义。

- [x] **Step 4: 实现前端安全 payload helper**

在 `frontend/src/features/settings/form-model.js` 放置可直接 import 的纯函数，页面/store
只调用这些函数构造 qB 请求。删除向 `/api/qb/*` 发送完整 server object 的路径。

- [x] **Step 5: 运行测试和构建**

Run:

```bash
cargo fmt --check
cargo test tests::qb_actions_
npm run verify:frontend
```

Expected: PASS。

- **Delivery action after explicit Git adoption (former Step 6):**

```bash
git add src/lib.rs frontend/src/features/settings frontend/src/__tests__/settings-form-model.vitest.js frontend/src/__tests__/settings-contract.vitest.js
git commit -m "fix: bind qb actions to configured servers"
```

**兼容/迁移：** “先测试再保存新 qB”不再支持。UI 必须清楚提示先保存；这是消除任意 URL/密码 SSRF 边界所需的有意变化。

---

### Task 6: 让 watcher 显式启用、支持无副作用 dry-run，并持久化轮询节奏

**Files:**

- Modify: `src/config.rs`
- Modify: `src/subscription.rs`
- Modify: `src/main.rs`
- Modify: `config.example.toml`

**Implementation status: complete.** Safe defaults, API/UI false-to-true confirmation, disabled-mode
automation isolation, persisted poll scheduling, pure execution policy, and dry-run side-effect
isolation are covered by unit/router/frontend tests. The commit checkbox remains intentionally open.

**行为要求：**

- `SubscriptionWatcherConfig` 新增 `enabled: bool`（default false）和 `dry_run: bool`（default true）。
- false -> true 的 PUT 必须同时带 `confirm_enable_automation: true`，否则 400。
- disabled 时 watcher 不请求豆瓣/M-Team/qB，不执行 filesystem，不修改 poll 时间。
- dry-run 时允许完整想看 poll、详情缓存、候选搜索和 qB 只读进度；不得 gen download token、add torrent、create_dir_all 或 hard_link。
- dry-run 搜索把候选匹配和 operation log 持久化，record 保持 Searching，并把下次尝试延后到 `search_interval_secs`。
- dry-run link 使用已有 `dry_run_hardlink_plan`，但下次尝试至少延后 `link_retry_interval_secs`，不得形成一秒循环。
- poll attempt/success/failure/backoff/next poll 跨重启持久化在 latest repository 的独立 account
  metadata 表。

- [x] **Step 1: 先写失败测试**

在 `src/config.rs`：

- `subscription_watcher_defaults_to_disabled_dry_run`

在 `src/subscription.rs`：

- `poll_attempt_success_and_failure_schedule_are_persistable`
- `poll_failure_uses_bounded_exponential_backoff`
- `dry_run_search_result_is_not_due_again_immediately`
- `dry_run_link_plan_is_not_due_again_immediately`

在 `src/main.rs`：

- `disabled_watcher_selects_no_tick_action`
- `enabling_watcher_requires_explicit_confirmation`
- `dry_run_movie_search_selects_candidate_plan_not_qb_push`
- `dry_run_movie_link_selects_plan_not_filesystem_execution`
- `watcher_uses_persisted_next_poll_after_restart`

- [x] **Step 2: 运行并确认默认/dispatcher 不满足安全要求**

Run:

```bash
cargo test config::tests::subscription_watcher_defaults_to_disabled_dry_run
cargo test subscription::tests::poll_
cargo test subscription::tests::dry_run_
cargo test tests::disabled_watcher_
cargo test tests::dry_run_movie_
cargo test tests::watcher_uses_persisted_next_poll_after_restart
```

Expected before implementation: 新字段/helper 不存在，或 watcher 仍在有 Cookie 时立刻运行。

- [x] **Step 3: 扩展 watcher config 和设置校验**

在 `src/config.rs` 添加两个字段与安全默认。旧配置缺字段时规范化为 disabled + dry-run，并按 Task 1 流程先备份再迁移。

在 config patch 合并中比较旧/新 `enabled`；仅 false -> true 需要 confirmation。true -> false 必须立即生效，无需重启。

- [x] **Step 4: 持久化 account poll metadata**

在 `WantedSubscriptionState` 增加向后兼容的默认字段：

- `last_poll_attempt_at`
- `last_poll_success_at`
- `poll_failure_count`
- `next_poll_at`
- `last_poll_error`

新增纯 transition helper 和 store 方法，写入 account-level state，不复制到每条 record。成功清零 failure；失败采用 `system_retry_interval_secs * 2^(n-1)`，上限为 `poll_interval_secs`。

worker 每次决策读取持久化 `next_poll_at`，删除只存在于 task 内存的 `last_wanted_poll_at` 权威值。

- [x] **Step 5: 增加显式执行策略**

在 `src/main.rs` 用纯 enum/helper 将 operation 映射为：

- disabled -> `Noop`
- live MovieSearch -> `Push`
- dry-run MovieSearch -> `CandidatePlan`
- live MovieLink -> `ExecuteHardlink`
- dry-run MovieLink -> `HardlinkPlan`
- progress -> 只读同步

worker 不再通过给 handler 填一个默认 false 来间接决定 dry-run。所有副作用前必须根据 policy 再检查一次。

- [x] **Step 6: 用 source guards 检查 dry-run 分支**

Run:

```bash
rg -n "process_wanted_push_step|execute_hardlink_plan|add_torrent" src/main.rs
```

人工确认每个 worker 调用点都受 live policy 控制；source guard 不是测试替代，必须与上述行为测试一起通过。

- [x] **Step 7: 更新示例配置并运行测试**

在 `config.example.toml` 的 `[subscription_watcher]` 首部增加：

```toml
enabled = false
dry_run = true
```

Run:

```bash
cargo fmt --check
cargo test config::tests::subscription_watcher_defaults_to_disabled_dry_run
cargo test subscription::tests::poll_
cargo test subscription::tests::dry_run_
cargo test tests::disabled_watcher_
cargo test tests::dry_run_movie_
cargo test tests::watcher_uses_persisted_next_poll_after_restart
```

Expected: PASS。

- **Delivery action after explicit Git adoption (former Step 8):**

```bash
git add src/config.rs src/subscription.rs src/main.rs config.example.toml
git commit -m "feat: make subscription automation opt in"
```

**兼容/回滚：** 升级后自动化会停用，必须由管理员显式重新开启。回滚安全方式是保持 `enabled=false`；不得为了兼容旧行为把缺失字段 default 改回 true。

---

### Task 7: 在 TV executor 完成前硬性 park 所有 TV 自动任务

**Files:**

- Modify: `src/subscription.rs`
- Modify: `src/main.rs`

**Implementation status: complete in the latest-only runtime.** Poll persists TV as
`tv_not_supported`, Execution selection does not claim it, and all unported manual effect routes are
absent. The `WantedSubscriptionRecord`/manual-command steps below record the superseded implementation
that first established this safety rule; those types and routes are now deleted.

**行为要求：**

- 保留 TV 模型和历史数据，不删除 episode/lane 字段。
- `WantedSubscriptionRecord` 使用可序列化的 `blocked_reason`；旧记录默认 None。
- 新识别或重新 poll 到的 TV 记录设置 `tv_not_supported`、`next_attempt_at=None`、`execution_state=Idle`。
- `select_due_operation` 对 TV 即使 force/lane due 也返回 None。
- retry/rerun/push/progress/completion 对 TV 返回明确 conflict，不改变 due 时间。
- watcher 不再调用 `process_tv_meta_operation` 或 `process_tv_lane_operation`；可以保留函数供未来实现，但生产 dispatcher 必须不可达。

- [x] **Step 1: 先写失败测试**

在 `src/subscription.rs`：

- `new_tv_record_is_parked_with_explicit_reason`
- `forced_tv_record_is_never_due_while_executor_unavailable`
- `existing_due_tv_record_is_parked_on_complete_poll_refresh`
- `parked_tv_record_preserves_episode_and_download_history`

在 `src/main.rs`：

- `watcher_dispatcher_has_no_tv_execution_branch_while_unsupported`
- `manual_tv_retry_rerun_and_side_effect_commands_return_conflict`

更新之前期待 `TvMeta`/`TvLane` 被生产 selector 选中的测试；保留 lane 算法的纯 domain tests，但不要让它们证明生产可调度。

- [x] **Step 2: 运行并确认当前 TV 会进入一秒错误路径**

Run:

```bash
cargo test subscription::tests::new_tv_record_is_parked
cargo test subscription::tests::forced_tv_record_is_never_due
cargo test tests::watcher_dispatcher_has_no_tv_execution_branch
cargo test tests::manual_tv_
```

Expected before implementation: selector 返回 `TvMeta`/`TvLane` 或 dispatcher 仍调用未实现函数。

- [x] **Step 3: 实现统一 parking helper**

在 `src/subscription.rs` 添加 `park_unsupported_tv_automation(record, now)`，由 record 创建、完整 poll refresh 和旧记录 repair 共用。helper 只清自动执行字段，不清候选、下载或 episode 历史。

`select_due_operation` 在所有 force/next-at 判断之前检查 block reason/media kind。

- [x] **Step 4: 移除生产 dispatcher 的 TV stub 路径**

在 `src/main.rs` 的 watcher policy/dispatcher 中不再构造 TV action。HTTP 手动命令在任何 qB/M-Team/filesystem 调用前拒绝。

- [x] **Step 5: 运行测试**

Run:

```bash
cargo fmt --check
cargo test subscription::tests::new_tv_record_is_parked
cargo test subscription::tests::forced_tv_record_is_never_due
cargo test subscription::tests::existing_due_tv_record_is_parked
cargo test subscription::tests::parked_tv_record_preserves_episode
cargo test tests::watcher_dispatcher_has_no_tv_execution_branch
cargo test tests::manual_tv_
```

Expected: PASS，并确认日志不再每秒出现 “TV ... not implemented”。

- **Delivery action after explicit Git adoption (former Step 6):**

```bash
git add src/subscription.rs src/main.rs
git commit -m "fix: park unsupported tv automation"
```

**兼容/回滚：** 这是临时能力门，不是删除 TV。未来启用 TV 时必须在同一提交中清除 block、恢复 selector、实现全部 executor 并通过端到端测试；不可只删除这个 guard。

---

### Task 8: 完整想看快照驱动 inactive/reactivate

**Files:**

- Modify: `src/douban.rs`
- Modify: `src/subscription.rs`
- Modify: `src/main.rs`

**Implementation status: complete in the latest Poll repository.** Completeness metadata, stable
per-attempt tokens, transactional complete-only deactivation, history-preserving reactivation, and
inactive guards are implemented directly in fresh latest state. The old compatibility implementation
can be deleted; no old-state migration is required.

**行为要求：**

- `DoubanLibraryList` 明确返回 `snapshot_complete`。
- 只有实际抓到空尾页才证明 complete；因 `library_limit`、最大页数、重复页或其他提前停止均为 partial，upstream 错误返回 error。
- record 增加 `active`（旧数据 default true）和 `inactive_at`。
- complete snapshot 中缺失的旧 record 设为 inactive，清自动 due/force/running，但保留历史。
- incomplete snapshot 只更新本次看到的 record，绝不 inactive 缺失项。
- inactive record 永不被 scheduler 选择。
- 后续再次出现时 re-activate；非 completed movie 恢复为 `next_attempt_at=now`，TV 仍保持 Task 7 parking。
- poll outcome/log 增加 `snapshot_complete/deactivated/reactivated`。

- [x] **Step 1: 先写失败测试**

在 `src/douban.rs`：

- `library_empty_first_page_is_complete`
- `library_short_last_page_is_complete`
- `library_limit_truncation_is_incomplete`
- `library_max_page_or_duplicate_stop_is_incomplete`

把分页完成性判断提取为纯 helper 测试，不要求访问真实豆瓣。

在 `src/subscription.rs`：

- `complete_poll_deactivates_records_not_seen`
- `partial_poll_never_deactivates_records_not_seen`
- `inactive_record_is_never_due_even_when_forced`
- `reappearing_movie_reactivates_without_losing_history`
- `reappearing_tv_stays_parked`
- `complete_empty_snapshot_deactivates_all_records`

在 `src/main.rs`：

- `failed_or_incomplete_wanted_poll_never_marks_missing_inactive`

- [x] **Step 2: 运行并确认当前合并只增不减**

Run:

```bash
cargo test douban::tests::library_
cargo test subscription::tests::complete_poll_
cargo test subscription::tests::partial_poll_
cargo test subscription::tests::inactive_record_
cargo test subscription::tests::reappearing_
cargo test tests::failed_or_incomplete_wanted_poll_
```

Expected before implementation: complete marker/active 字段不存在，或缺失 record 仍保持可调度。

- [x] **Step 3: 实现保守的 snapshot completeness**

在 `src/douban.rs::library` 记录停止原因。不要通过 `items.len() < configured_limit` 推断完整；必须看到可靠的末页信号。请求中途失败继续返回 Err，不返回“带部分 items 的成功”。

- [x] **Step 4: 扩展 record 和 poll merge**

为 `WantedSubscriptionRecord.active` 使用 `#[serde(default = "default_true")]`，保证旧 JSON/SQLite record 不被误判 inactive。

将 `apply_wish_items_with_details_to_state` 增加 `snapshot_complete` 参数：先记录 seen IDs，再刷新/创建/复活；只有 complete 时遍历缺失 active records 执行 deactivate。重复 complete poll 必须幂等，不重复增加 deactivated 计数。

- [x] **Step 5: 在所有自动/手动副作用入口检查 active**

`select_due_operation` 首先检查 active。`load_wanted_record_context` 或各副作用 handler 对 inactive 返回明确错误；只读详情和 operation log 仍可查看。

- [x] **Step 6: 运行测试**

Run:

```bash
cargo fmt --check
cargo test douban::tests::library_
cargo test subscription::tests::complete_poll_
cargo test subscription::tests::partial_poll_
cargo test subscription::tests::inactive_record_
cargo test subscription::tests::reappearing_
cargo test tests::failed_or_incomplete_wanted_poll_
```

Expected: PASS。

- **Delivery action after explicit Git adoption (former Step 7):**

```bash
git add src/douban.rs src/subscription.rs src/main.rs
git commit -m "fix: deactivate subscriptions absent from complete polls"
```

**兼容/迁移：** `active` 缺失必须解释为 true。若 completeness 存在任何不确定性，宁可不 deactivate；不得用“本次请求成功”替代“快照完整”的证明。

---

### Task 9: 接入前端登录、脱敏设置和自动化确认

**Files:**

- Modify: `frontend/src/app/AuthGate.vue`
- Modify: `frontend/src/pages/SettingsPage.vue`
- Modify: `frontend/src/features/settings/form-model.js`
- Modify: `frontend/src/shared/api/endpoints/auth.js`
- Add: `frontend/src/__tests__/auth-gate.vitest.js`
- Modify: `frontend/src/__tests__/settings-form-model.vitest.js`
- Modify: `frontend/src/__tests__/settings-contract.vitest.js`

**Implementation status: complete (commit pending).** The authenticated startup gate, Settings/qB/QR security contract,
watcher enabled/dry-run controls, and explicit first-enable confirmation are complete in the feature
stores/pages and their Vitest suites. Inactive/TV/blocked/schedulable capability badges, stable
explanations, a read-only latest subscription detail, and the runtime watcher-mode banner are also
complete. Management-token Set/Clear, immediate session refresh, and protected-401 fallback are
implemented without persisting or echoing the replacement token.

**行为要求：**

- 启动先读取 `/api/auth/status`；未认证显示 token 登录 gate，不加载其他 API。
- token 只发送给 login endpoint，不写 localStorage、不插入 URL、不输出 console。
- API helper 在 401 时回到登录 gate；同源 Cookie 自动携带。
- 设置页根据 `has_*` 显示“已配置”，secret input 初始为空；留空代表 keep，清除需要独立勾选/按钮。
- qB password 使用 `has_password` 和 Keep/Set/Clear 语义。
- 设置 payload 带 `expected_revision`。
- watcher 显示 enabled/dry-run；false -> true 弹出明确确认，并发送 `confirm_enable_automation=true`。
- disabled、dry-run、live 三种配置状态在订阅页可见；当前 latest-only 详情是只读的，不渲染尚未
  接线的副作用按钮。
- QR 登录完成只根据 `cookie_saved` 更新 `has_douban_cookie`，不期待服务端返回 Cookie。

- [x] **Step 1: 先写失败测试**

`auth-gate.vitest.js` 与 shared API endpoint tests 用 mounted gate/fake fetch 验证：

- login token 只出现在一次 POST body。
- 不调用 localStorage。
- 401 清理认证状态并触发 gate。
- logout 清理本地认证状态。

`settings-form-model.vitest.js` 与 `settings-contract.vitest.js` 验证：

- redacted response 转 form 不产生 secret value。
- untouched secret 生成 Keep，明确清除生成 Clear。
- qB password 同样处理。
- expected revision 被带回。

Settings contract tests 直接覆盖 watcher DTO 和 enable-confirmation helper：

- false -> true 需要 confirmation。
- false/true -> false 不需要 confirmation。
- watcher defaults 显示 disabled + dry-run。
- 订阅前端不导出或调用 push/retry/rerun effect endpoint。

- [x] **Step 2: 运行并确认 helper/UI 尚未支持新契约**

Run:

```bash
npm test -- auth-gate settings-form-model settings-contract
```

Expected before implementation: module/exports 不存在或断言失败。

- [x] **Step 3: 实现可测试的前端纯 helper**

把 auth 请求、脱敏 DTO -> form、form -> patch、watcher enable confirmation、record capability 判断放入两个可 import 模块。`App.vue` 负责 UI orchestration，不复制 payload 逻辑。

- [x] **Step 4: 改造登录 gate 和设置页**

在 `App.vue` 顶层加入最小登录/登出交互。bootstrap 模式下提示“当前仅 loopback 可用，请立即设置管理 token”；设置 token 成功后立即调用 login，并清空输入。

设置页增加：

- 管理 token Set/Clear（非 loopback 上禁止清除最后凭据）。
- secret has flags 与显式 clear。
- watcher enabled/dry-run。
- 配置 revision conflict 提示“配置已被其他操作更新，请重新加载”，不得自动重放旧全量表单。

- [x] **Step 5: 更新订阅状态展示和按钮能力**

inactive 卡片保留可查看历史。TV parked 显示“TV 自动化尚未开放”，不显示可重试暗示。当前
latest-only 详情不渲染未接线的副作用按钮；dry-run 全局 banner 明确写“不会推送 qB 或创建硬链接”。

完成状态：inactive、TV unsupported、generic blocked 与 schedulable badges 已接入列表和详情；
所有 subscription effect commands/buttons 已删除，movie/TV/inactive 均为可查看历史的只读详情。
全局 watcher banner 直接读取 AppShell 的 runtime settings，明确区分 unloaded、disabled、dry-run
与 live 配置；disabled/dry-run 不 claim Execution，只有显式 live 配置才会执行 qB 或 hardlink。
纯 helper 和 mounted route tests 覆盖全部模式。

- [x] **Step 6: 运行前端测试与构建**

Run:

```bash
npm run verify:frontend
```

Expected: 本任务新增测试和生产构建 PASS；若 `npm run check` 仍命中其他历史文档格式问题，必须引用 CI/documentation plan 的对应任务，不能忽略本任务文件的错误。

Historical capability/watcher-display slice verification: targeted settings-helper and
subscriptions-page Vitest passed at 31 files / 116 tests. The current combined frontend baseline is
recorded above as 43 files / 247 tests plus checked API types, formatting, lint, and the production
build.

- **Delivery action after explicit Git adoption (former Step 7):**

```bash
git add frontend/src/app/AuthGate.vue frontend/src/pages/SettingsPage.vue frontend/src/features/settings frontend/src/shared/api frontend/src/__tests__
git commit -m "feat: expose safe management controls"
```

**兼容/迁移：** 本任务必须与 Task 4 同版本发布。不能先发布脱敏后端配旧前端，也不能先发布会发送 patch DTO 的前端配旧后端。

---

### Task 10: 更新安全 bootstrap、Docker/NAS 和回滚说明

**Files:**

- Modify: `config.example.toml`
- Modify: `deploy/nas/docker-compose.yml`
- Modify: `deploy/nas/README.md`
- Add: `docs/operations/security-bootstrap.md`

**Implementation status: complete (commit pending).** General security, configuration, NAS, backup,
and rollback docs describe token/Cookie/CORS and secret-file behavior. Compose now defaults to host
loopback with explicit authenticated LAN opt-in. Watcher enable/dry-run guidance and the dedicated
bootstrap/upgrade/recovery runbook are present and pass the local documentation contract checks.

**必须记录的事实：**

- host 开发默认 `127.0.0.1:8787`。
- 容器内部若监听 `0.0.0.0`，必须预先配置至少 24 字符 token。
- 默认 Compose 端口绑定为宿主 loopback（`127.0.0.1:8787:8787`）；直接 LAN 暴露是显式选择，并仍要求 token。
- 反向代理负责 TLS；`secure_cookie=true` 时浏览器只通过 HTTPS 使用。
- 同源不配置 CORS；只有明确的跨源客户端才填写 allowlist。
- 升级前复制 config、检查权限、设置 token；配置迁移产生 backup。
- watcher 升级后 disabled + dry-run，验证候选/硬链接计划后再显式 live。
- 恢复配置使用最新已验证 backup，原子替换并设 `0600`；不要启动后再覆盖运行中配置。

- [x] **Step 1: 先运行文档契约检查并确认缺失**

Run:

```bash
rg -n "127\.0\.0\.1|admin_token|allowed_origins|secure_cookie|enabled = false|dry_run = true" config.example.toml deploy/nas docs/operations
```

Expected before implementation: 多个必需主题缺失；若 `docs/operations` 尚不存在，命令可报告路径不存在，但不得据此跳过任务。

- [x] **Step 2: 更新示例与部署默认**

`config.example.toml` 使用假 token 占位说明但不能提供可复制的固定默认 token。Compose 使用宿主 loopback 端口，并在 README 给出显式 LAN 绑定示例，而不是默认开放。

- [x] **Step 3: 编写 bootstrap/升级/回滚 runbook**

`docs/operations/security-bootstrap.md` 至少包含：新安装、旧 `0.0.0.0` 配置升级、容器部署、登录、token 轮换、忘记 token、CORS、watcher dry-run、配置 backup 恢复、常见启动错误。

禁止在示例中展示真实 token、Cookie、API key 或密码。

- [x] **Step 4: 验证文档覆盖和格式**

Run:

```bash
rg -n "127\.0\.0\.1|admin_token|allowed_origins|secure_cookie|enabled = false|dry_run = true" config.example.toml deploy/nas docs/operations/security-bootstrap.md
npm run check
```

Expected: 所有主题均有命中，Markdown/TOML/YAML 格式检查 PASS。

- **Delivery action after explicit Git adoption (former Step 5):**

```bash
git add config.example.toml deploy/nas/docker-compose.yml deploy/nas/README.md docs/operations/security-bootstrap.md
git commit -m "docs: document secure bootstrap and automation"
```

**兼容/回滚：** Compose 默认改为宿主 loopback 可能影响直接 LAN 访问；runbook 必须提供经过鉴权的显式 opt-in，而不是让用户删除 auth。

---

### Task 11: 执行全量安全验收与迁移演练

**Files:**

- No production changes expected.
- If a failure reveals a bug, return to the owning task and amend only that task's files/commit.

- [x] **Step 1: 全量静态与单元验证**

Run:

```bash
cargo fmt --all -- --check
cargo clippy --locked --offline --all-targets -- -D warnings
cargo test --locked --offline --all-targets
npm run verify:frontend
git diff --check
```

Expected: 全部 PASS。不得以“目标测试已过”替代全量结果。

Latest combined local evidence: Rustfmt, strict offline Clippy, the current Rust/router test suites,
247 frontend tests across 43 files, checked API types, frontend format/lint/build, Markdown
relative-link validation, and `git diff --check` all pass.

- [x] **Step 2: 配置破坏性场景演练（仅临时目录）**

在 `/tmp` 复制 fixture 并验证：

1. malformed TOML：进程退出非零，原文件 hash/mode/mtime 不变。
2. unknown field：进程退出非零并指出字段和位置。
3. 旧合法配置：产生一个 `0600` backup 和一个规范化配置。
4. 模拟写失败：原文件仍可加载，内存 revision 不变。
5. 两个并发 patch + QR Cookie patch：最终文件包含三个互不覆盖的变化。

不得对仓库根目录的真实 `config.toml` 执行演练。

The full suite executes these cases against isolated temp files through the config store/manager tests:
malformed and unknown TOML are zero-write failures, normalization creates a verified `0600` backup,
persist failure preserves file/memory/revision, and serialized disjoint/QR patches do not overwrite.

- [ ] **Step 3: 管理面手工 smoke**

验证矩阵：

| 监听/token        | 来源               | 结果                 |
| ----------------- | ------------------ | -------------------- |
| loopback/空       | loopback           | bootstrap 可用       |
| loopback/已配     | 未登录             | 401                  |
| loopback/已配     | 正确 Cookie/Bearer | 2xx                  |
| non-loopback/空   | 任意               | 启动失败             |
| non-loopback/已配 | 未登录             | 401                  |
| allowlist 空      | cross-origin       | 无 CORS allow header |
| allowlist 精确值  | 匹配 origin        | 允许                 |
| allowlist 精确值  | 其他 origin        | 拒绝                 |

读取 `/api/config` 响应并对真实 secret 做本地负匹配；输出中不得打印 secret 本身。

Automated real-router coverage for this matrix, login rate limiting, token rotation, Fetch-Metadata
CSRF rejection, redacted config, and exact CORS is green. This checkbox remains open until the same
matrix is recorded against the target proxy/NAS topology; in-process router evidence cannot prove
forwarded peer identity, TLS termination, or browser Cookie behavior through that proxy.

- [x] **Step 4: 自动化副作用 smoke**

使用测试 qB/临时媒体目录或 fake adapter：

1. disabled 连续运行两个 poll interval：外部调用计数与 filesystem 变化均为 0。
2. enabled + dry-run：允许候选和 link plan，qB add 调用计数 0，目录/inode 不变。
3. enabled + live：只执行 movie 路径；TV 保持 parked。
4. dry-run link 后下一秒不再次执行。
5. 重启后使用 persisted `next_poll_at`，不立即重复 poll。

The all-target suite covers disabled zero-upstream/zero-claim/zero-effect behavior, dry-run candidate/link plans
without qB or hardlinks, movie-only execution with TV parking, bounded dry-run scheduling, and persisted
restart cadence through fake adapters and temporary media/state paths.

- [x] **Step 5: 想看生命周期 smoke**

用 fixture 快照验证：

1. complete A+B -> complete A：B inactive 且不 due。
2. incomplete A：B 保持 active。
3. complete A+B 再出现 B：B active，历史 artifacts 保留。
4. complete empty：全部 inactive。
5. TV 在以上每一步都不进入 executor。

Complete/incomplete/empty/reappearance fixtures and the latest Poll adapter tests cover all five cases,
including history preservation, inactive due suppression, and TV parking.

- [ ] **Step 6: 安全完成审计**

逐项对照源 PRD R1、R2、R3 和本计划负责的 R6 条目，记录每条的测试名、命令输出或 smoke 证据。只有以下全部成立才能把本计划 `implementation_status` 改为 `implemented`：

- malformed TOML 零写回。
- secret-bearing GET endpoint 为零。
- wildcard CORS 为零。
- LAN 无 token 启动路径为零。
- 接收 inline qB URL/password 的 action endpoint 为零。
- 新安装自动化副作用为零。
- scheduler 选择未实现 TV executor 的路径为零。
- complete wanted snapshot 中已移除但仍 active 的记录为零。

---

## 最终回滚原则

- 配置问题：停止服务，验证并恢复 timestamped backup，权限设为 `0600`，再启动；不要让两个版本进程同时写同一配置。
- 鉴权问题：先退回 loopback，不得通过移除 token 后继续暴露 `0.0.0.0`。
- watcher 问题：设置 `enabled=false` 是第一止损动作；dry-run 不是禁用的替代品。
- inactive 误判：先禁用 watcher，修复 snapshot completeness 或恢复 DB backup，再重新 poll；不要直接删除历史 record。
- TV 问题：保持 parking；不能通过 force/retry 绕过能力门。

## 与后续计划的交接契约

- Backend boundaries plan 移动 auth/config handler 时，必须保留本计划所有行为测试、DTO 负泄密
  断言、URL-first 诊断脱敏，以及“同一请求只用一个配置快照派生 account/redactor”的契约。
- Storage/scheduler runtime 已在 fresh latest per-record/account schema 中直接实现 poll metadata 和
  active 索引，并删除 blob/store/JSON/watcher/service/policy 兼容代码；旧状态文件继续完全忽略。
- Frontend modularization plan 可移动 `auth-session.js` 与 `config-form.js`，但 login、secret Keep/Set/Clear 和 automation confirmation 的公开行为不变。
- CI/deployment plan 必须把本计划 Task 11 的命令和至少一个未鉴权 401/container smoke 纳入发布 gate。
