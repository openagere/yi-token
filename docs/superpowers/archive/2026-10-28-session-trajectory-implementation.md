> 历史实施计划（已完成，仅作记录）。复选框保留原始状态，不代表未完成；当前行为以 `docs/spec-coverage.md` 与代码为准。

# 观测体系重构（运行 / 会话 / 用量）实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把代理页拆成"运行 / 会话 / 用量"三个观测页面；新增按终端、按会话的完整轨迹采集与展示，会话页参考 deepseek-harness 的轨迹视图实现。

**Architecture:** 采集挂在现有请求管线的旁路上，复用 `protocol::Source` 已经产出的 IR 流事件和 `responses::stream::Collector`。采集记录经一个按字节限流的队列送到独立写入线程，写入单独的 `trajectory.db`：前缀链加内容寻址 blob，并用 zstd 压缩。用量仍写主库 `usage` 表，只增加关联列和耗时列。投影（回合 → 步骤 → 记录）在 Rust 侧完成，前端只负责展示状态。

**Tech Stack:** Rust 2024、axum 0.8、rusqlite 0.40（bundled）+ r2d2 + rusqlite_migration、sha2、**zstd（新增）**、ts-rs；前端 React 19、TanStack Query、`@tanstack/react-virtual`（已有）、**react-markdown + remark-gfm（新增）**、recharts（已有）；测试用 cargo test、wiremock、insta、Playwright（`tests/mock-backend.ts`）。

**Spec:** `docs/superpowers/specs/2026-10-28-session-trajectory-design.md`（执行者必须同时阅读，尤其是第 3、4、5、8.3.1 节）。前置 spec：`docs/superpowers/specs/2026-10-25-yi-token-design.md`。

> **关于本计划的写法：** 沿用前一份计划 `docs/superpowers/archive/2026-10-25-yi-token-implementation.md` 的约定：不附完整实现代码。每个任务给出文件、接口签名、SQL、行为要求、测试用例（名称与断言要点）和验证命令。行为细节以 spec 为准。会话页的交互细节以 deepseek-harness 源码为准，路径见每个任务的"参考"。

## 如何阅读 deepseek-harness 参考代码

位置：`D:/Github/deepseek-harness/packages/client/ui-trajectory/src/client/`。**只读，不要修改那个仓库。** 可以移植算法（折叠、时间线投影、虚拟行分组），但要用本项目的组件、样式 token 和 i18n 重写。不要引入它的 Slot、Factory、`dsh-*` 依赖。

| 文件 | 移植什么 |
| --- | --- |
| `trajectory-record.ts` | 记录类型 `TrajectoryCellKind`、字段语义、`trajectoryRecordId` |
| `timeline.ts` | `deriveTrajectoryTimeline`、三种投影、`laneFor`、`trajectoryTimelineFocusIndexes`、空闲段压缩 |
| `trajectory-virtual-rows.ts` | `groupTrajectoryVirtualRows`、行高常量（30 / 20 / 9） |
| `TrajectoryTable.tsx` 第 600–760 行 | `summarizeTurn`、`collapseTurnRecords`、`collapseAssistantRecords`、`stateOf` |
| `TrajectoryTable.tsx` 第 976–1013 行 | `detailTabs`（检查器页签随类型变化） |
| `TrajectoryTable.tsx` 第 2690–2930 行 | 行渲染：请求边界标记、回合标签、类型标签、"调用 → 结果"行内展示、双击与键盘行为 |
| `TrajectoryToolbar.tsx` | 工具栏：时长投影切换、实际时间、轮次和调用的全部折叠、搜索 |
| `TrajectoryTimeline.tsx` | 拖选、滚轮缩放、右键清除、右键拖动平移、500ms 悬停提示 |
| `locales.ts` | 中英文文案（改成本项目 i18n 的 key 结构） |

## Global Constraints

- **本轮不做 git 提交**（用户明确要求，优先于 `AGENTS.md`）。每个任务结束时只运行验证，不执行 `git add` / `git commit`。如果用户之后允许提交：提交说明中不得出现 `Co-authored-by`，作者和提交者邮箱 `yipen9@gmail.com`，不修改 git config。
- 轨迹库路径 `<数据目录>/trajectory.db`；主库仍为 `<数据目录>/yi-token.db`；数据目录 `$YI_TOKEN_HOME`，默认 `~/.yi-token`。
- 轨迹库是独立的 r2d2 池，WAL，`foreign_keys=ON`，`busy_timeout=5000`，`user_version` 从 1 起，迁移与主库分开编号。
- 采集是旁路：采集失败、队列满、轨迹库不可用，都**不得**改变代理转发的字节、状态码，也不得影响 `usage` 写入。
- `core` 不依赖 `tauri`；`src-tauri` 只调用 `core::services`；`proxy` 通过 trait（`CaptureSink`）投递，不直接依赖 `storage`。
- 代码中禁止 `unsafe`；`cargo clippy --workspace --all-targets -- -D warnings` 必须通过。
- 默认值：采集开启；保留 30 天；容量上限 2 GiB（`2_147_483_648` 字节）；采集队列在途上限 64 MiB；写入批次 64 步或 32 MiB；单个 blob 原文上限 1 MiB，截断时保留首尾各 256 KiB；`data:` 内联图片超过 64 KiB 时只存元数据；检查点间隔 64 步；尾部索引 LRU 2000 项；前缀链只在同终端、12 小时内活跃的线程中匹配；`LiveRegistry` 环形缓冲 500 条；`error` 截断 4 KiB；记录正文摘要 2 KiB；轨迹分页默认 20 个回合。
- 前端文案必须同时提供 `zh-CN` 与 `en-US`；样式只用 `src/styles/tokens.css` 的 token，`npm run check:theme` 必须通过。
- 全部验证命令：`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、`npm run typecheck`、`npm run lint`、`npm run check:theme`、`npm run build`、`npm run test:ui`。

## Review Focus

1. **客户端回传的历史和代理记录的输出不完全一致**（推理签名被剥离、条目 ID 改变、Anthropic thinking 块被丢弃）：用户期望同一会话的所有步骤仍然串在一条线程上。→ 任务 7 `chain_survives_stripped_reasoning_and_new_ids`。
2. **客户端在流式响应中途断开**：期望该步骤以 `cancelled` 落库，已收到的部分输出可见，`usage` 照常记录，并且不出现第二条记录。→ 任务 8 `client_disconnect_mid_stream_keeps_partial_output`。
3. **超大请求体（8 MiB 历史）或采集队列饱和**：期望请求成功、延迟不受影响，步骤降级为 `metadata_only`，会话页仍显示这一步。→ 任务 8 `oversized_or_saturated_capture_degrades_to_metadata`。
4. **`trajectory.db` 损坏、被占用或迁移失败**：期望应用正常启动、代理正常转发、用量正常写入，设置页显示原因并可以重建。→ 任务 11 `corrupt_trajectory_db_does_not_block_startup`。
5. **几百步的长会话**：期望会话页按回合分页打开，单步"完整上下文"还原时回溯不超过 64 层父步骤。→ 任务 10 `long_session_reconstruction_is_bounded_by_checkpoints`。

---

## 文件结构

**Rust（`core/`）**

| 路径 | 职责 |
| --- | --- |
| `src/paths.rs` | 新增 `AppPaths.trajectory_db` |
| `src/storage/pool.rs` | `Db` 支持传入迁移集（主库 / 轨迹库共用连接池代码） |
| `src/storage/migrations.rs` | 主库 v4：`usage` 新列、`settings` 轨迹列 |
| `src/storage/trajectory/mod.rs` | 轨迹库子模块入口：`TrajectoryDb` 新类型 |
| `src/storage/trajectory/migrations.rs` | 轨迹库 v1 建表 |
| `src/storage/trajectory/blobs.rs` | blob 写入（去重、zstd）、读取、孤儿清理 |
| `src/storage/trajectory/repo.rs` | sessions / threads / steps / step_items 的读写 |
| `src/storage/trajectory/writer.rs` | 采集写入线程：按字节限流的队列、批量写入 |
| `src/storage/trajectory/retention.rs` | 按天数和容量清理 |
| `src/trajectory/mod.rs` | 采集领域逻辑入口（不依赖 storage） |
| `src/trajectory/normalize.rs` | 条目规范化、存储哈希、语义指纹、截断、内联图片剥离 |
| `src/trajectory/keys.rs` | 从请求头和请求体提取客户端会话标识、回合标识 |
| `src/trajectory/link.rs` | 会话、线程、父步骤、回合、线程类型的判定，以及尾部指纹索引 |
| `src/trajectory/record.rs` | `CaptureRecord`、`CaptureSink` trait、`StopReason`、`CaptureLevel` |
| `src/proxy/capture.rs` | 流事件 tap：`CaptureTap`（`Collector` + 首 token 时间） |
| `src/proxy/live.rs` | `LiveRegistry`：在途请求和最近 500 条 |
| `src/proxy/pipeline.rs` | 挂接点：克隆请求体、tap、`UsageGuard` 的 `Drop` 时投递 |
| `src/proxy/router.rs` | `ProxyState` 增加 `capture: Option<Arc<dyn CaptureSink>>`、`live: LiveRegistry` |
| `src/protocol/mod.rs` | `StreamTranslator` 增加 `set_tap`；`PassthroughStream::finish` 调用 `on_end` |
| `src/domain/session.rs` | 对外 DTO：`SessionSummary`、`ThreadView`、`TrajectoryPage`、`TrajectoryRecord` 等（ts-rs） |
| `src/domain/usage.rs` | `UsageEvent` / `UsageRecord` 增加字段 |
| `src/domain/settings.rs` | `TrajectorySettings` |
| `src/services/sessions.rs` | 会话服务方法 |
| `src/services/trajectory_view.rs` | 投影：步骤 → 回合 / 记录；完整上下文还原 |
| `src/services/proxy.rs` | `live_requests` |
| `src/services/usage.rs` | 终端筛选、`group_by`、耗时与缓存率 |
| `src/services/mod.rs` | 启动轨迹库与写入线程，失败时隔离；关闭时 shutdown |
| `tests/trajectory_*.rs` | 集成测试 |

**Tauri（`src-tauri/`）**：`src/commands.rs` 新增命令；`src/lib.rs` 注册。

**前端（`src/`）**

| 路径 | 职责 |
| --- | --- |
| `shared/navigation/store.ts` | `Page` 增加 `"sessions"`；带参数的导航 `openSession({sessionId, stepId?, client?, model?})` |
| `app/layout/Sidebar.tsx` | "配置 / 观测"分组 |
| `app/routes.tsx` | 注册会话页 |
| `features/proxy/components/RunPage.tsx` | 运行页（替代 `ProxyPage.tsx`） |
| `features/proxy/components/LiveRequests.tsx` | 请求流 |
| `features/proxy/components/LogDrawer.tsx` | 底部日志抽屉（复用 `ProxyLogs` 内部） |
| `features/sessions/api.ts` `hooks.ts` `types.ts` `index.ts` `pages.ts` | 数据接入 |
| `features/sessions/components/SessionsPage.tsx` | 双栏布局 |
| `features/sessions/components/SessionList.tsx` | 左栏 |
| `features/sessions/components/SessionHeader.tsx` | 会话头与线程切换 |
| `features/sessions/trajectory/records.ts` | 扁平化、折叠、搜索、状态（移植自 `TrajectoryTable.tsx`） |
| `features/sessions/trajectory/virtualRows.ts` | 移植自 `trajectory-virtual-rows.ts` |
| `features/sessions/trajectory/timeline.ts` | 移植自 `timeline.ts` |
| `features/sessions/trajectory/TrajectoryToolbar.tsx` | 工具栏 |
| `features/sessions/trajectory/TrajectoryTimeline.tsx` | 时间线总览 |
| `features/sessions/trajectory/TrajectoryLedger.tsx` | 账本表格 |
| `features/sessions/trajectory/TrajectoryInspector.tsx` | 检查器 |
| `features/sessions/trajectory/TrajectoryView.tsx` | 组合以上四部分 |
| `features/usage/components/UsagePage.tsx` 等 | 用量页重构 |
| `features/settings/components/ListenSettings.tsx`、`TrajectorySettings.tsx` | 设置页新分组 |
| `shared/i18n/locales/{zh-CN,en-US}/session.ts` | 会话 / 轨迹文案 |
| `styles/sessions.css`、`styles/trajectory.css` | 样式 |
| `tests/mock-backend.ts`、`tests/fixtures/trajectory.ts` | 模拟后端与固定数据 |
| `tests/sessions.spec.ts`、`tests/run-page.spec.ts`、`tests/usage.spec.ts` | Playwright |

---

## 阶段 A：后端基础

### Task 1：抓包探针

**Files:**
- Modify: `core/src/proxy/pipeline.rs`（`run` 开头）
- Create: `core/src/trajectory/mod.rs`（先只放 `pub fn probe_shape(headers: &HeaderMap, body: &Value) -> String`）
- Modify: `core/src/lib.rs`（`pub mod trajectory;`）
- Test: `core/src/trajectory/mod.rs` 内的 `#[cfg(test)]`

**Interfaces:**
- Produces: `trajectory::probe_shape(headers, body) -> String`。只返回请求头**名称**列表、请求体顶层键列表，以及 `metadata` 下的键名，不含任何值。

- [ ] **Step 1: 写测试** `probe_lists_names_without_values`：构造请求头 `session_id: abc`、`authorization: secret`，请求体 `{"model":"m","metadata":{"user_id":"u_x"}}`。断言输出含 `session_id`、`authorization`、`metadata.user_id`，并且不含 `abc`、`secret`、`u_x`。
- [ ] **Step 2: 运行** `cargo test -p yi-core trajectory::tests::probe` → 编译失败（函数不存在）。
- [ ] **Step 3: 实现**，并在 `pipeline::run` 中、`resolve` 之前加上 `tracing::debug!(target: "yi_token::probe", client = ?scope, shape = %probe_shape(..))`。每个 `(scope, inbound)` 组合在一次进程生命周期内只打一次，用 `OnceLock<Mutex<HashSet<..>>>` 去重。
- [ ] **Step 4: 运行测试** → PASS。
- [ ] **Step 5: 人工抓包。** 把日志级别设为 `debug`，五个终端各跑一次带工具调用的任务，把结论填到本文件末尾的"附录 A：会话标识抓包结论"。**如果某个客户端的结论与 spec 4.3 的假设不一致，先修改 spec 4.3，再做任务 6。** 这一步需要人工，执行代理遇到时应提醒用户，不要编造结论。
- [ ] **Step 6: 验证** `cargo clippy -p yi-core --all-targets -- -D warnings`。不提交。

### Task 2：`Db` 支持多套迁移，并新增轨迹库建表

**Files:**
- Modify: `core/src/storage/pool.rs`
- Modify: `core/src/paths.rs`
- Create: `core/src/storage/trajectory/mod.rs`、`core/src/storage/trajectory/migrations.rs`
- Modify: `core/src/storage/mod.rs`（`pub mod trajectory;`）
- Test: `core/tests/storage_repos.rs`（追加）、`core/src/paths.rs` 内部测试

**Interfaces:**
- Produces:
  - `pub type MigrateFn = fn(&mut Connection) -> CoreResult<()>;`
  - `Db::open_with(path: &Path, migrate: MigrateFn) -> CoreResult<Db>`、`Db::open_in_memory_with(migrate: MigrateFn) -> CoreResult<Db>`。现有的 `Db::open` / `Db::open_in_memory` 改为以 `migrations::run` 调用它们，行为不变。
  - `pub struct TrajectoryDb(pub Db);`，`TrajectoryDb::open(path) -> CoreResult<TrajectoryDb>`，`TrajectoryDb::open_in_memory()`。
  - `AppPaths.trajectory_db: PathBuf`（`home.join("trajectory.db")`）。
  - 轨迹库额外设置 `PRAGMA auto_vacuum = INCREMENTAL`，必须在建表**之前**执行；新库在 v1 迁移的第一句设置。

**轨迹库 v1 SQL**（`storage/trajectory/migrations.rs`，原样使用）：

```sql
CREATE TABLE sessions (
    id               TEXT PRIMARY KEY,
    client           TEXT NOT NULL,
    client_key       TEXT,
    key_source       TEXT NOT NULL CHECK (key_source IN ('client', 'chain', 'new')),
    title            TEXT NOT NULL DEFAULT '',
    started_at       INTEGER NOT NULL,
    last_at          INTEGER NOT NULL,
    step_count       INTEGER NOT NULL DEFAULT 0,
    input_tokens     INTEGER NOT NULL DEFAULT 0,
    output_tokens    INTEGER NOT NULL DEFAULT 0,
    bytes            INTEGER NOT NULL DEFAULT 0,
    UNIQUE (client, client_key)
);
CREATE INDEX idx_sessions_last_at ON sessions(last_at);
CREATE INDEX idx_sessions_client_last_at ON sessions(client, last_at);

CREATE TABLE threads (
    id               TEXT PRIMARY KEY,
    session_id       TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    kind             TEXT NOT NULL CHECK (kind IN ('main', 'subagent', 'side', 'pending')),
    parent_thread_id TEXT REFERENCES threads(id) ON DELETE SET NULL,
    spawn_call_id    TEXT,
    reset            INTEGER NOT NULL DEFAULT 0,
    started_at       INTEGER NOT NULL
);
CREATE INDEX idx_threads_session ON threads(session_id);

CREATE TABLE steps (
    id                TEXT PRIMARY KEY,
    seq               INTEGER NOT NULL UNIQUE,
    session_id        TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    thread_id         TEXT NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    parent_step_id    TEXT REFERENCES steps(id) ON DELETE SET NULL,
    prefix_len        INTEGER NOT NULL,
    checkpoint        INTEGER NOT NULL DEFAULT 0,
    depth             INTEGER NOT NULL DEFAULT 0,
    turn_no           INTEGER NOT NULL,
    client_turn_id    TEXT,
    requested_at      INTEGER NOT NULL,
    first_token_at    INTEGER,
    finished_at       INTEGER,
    status            INTEGER,
    stop_reason       TEXT NOT NULL,
    error             TEXT,
    inbound_protocol  TEXT NOT NULL,
    upstream_protocol TEXT NOT NULL,
    provider_name     TEXT NOT NULL,
    model             TEXT NOT NULL,
    upstream_model    TEXT NOT NULL,
    route_mode        TEXT,
    input_tokens      INTEGER NOT NULL DEFAULT 0,
    output_tokens     INTEGER NOT NULL DEFAULT 0,
    cached_tokens     INTEGER NOT NULL DEFAULT 0,
    reasoning_tokens  INTEGER NOT NULL DEFAULT 0,
    system_blob       TEXT,
    tools_blob        TEXT,
    options_json      TEXT NOT NULL DEFAULT '{}',
    tail_fp           TEXT NOT NULL,
    capture           TEXT NOT NULL CHECK (capture IN ('full', 'metadata_only'))
);
CREATE INDEX idx_steps_session_turn ON steps(session_id, turn_no);
CREATE INDEX idx_steps_thread ON steps(thread_id, seq);
CREATE INDEX idx_steps_tail_fp ON steps(tail_fp);
CREATE INDEX idx_steps_system_blob ON steps(system_blob) WHERE system_blob IS NOT NULL;
CREATE INDEX idx_steps_tools_blob ON steps(tools_blob) WHERE tools_blob IS NOT NULL;

CREATE TABLE step_items (
    step_id   TEXT NOT NULL REFERENCES steps(id) ON DELETE CASCADE,
    side      TEXT NOT NULL CHECK (side IN ('input', 'output')),
    ord       INTEGER NOT NULL,
    blob      TEXT NOT NULL,
    kind      TEXT NOT NULL,
    role      TEXT,
    call_id   TEXT,
    tool_name TEXT,
    PRIMARY KEY (step_id, side, ord)
);
CREATE INDEX idx_step_items_call ON step_items(call_id) WHERE call_id IS NOT NULL;
CREATE INDEX idx_step_items_blob ON step_items(blob);

CREATE TABLE blobs (
    hash      TEXT PRIMARY KEY,
    kind      TEXT NOT NULL,
    raw_bytes INTEGER NOT NULL,
    data      BLOB NOT NULL,
    truncated INTEGER NOT NULL DEFAULT 0
);
```

说明：`steps.depth` 是到最近检查点的父链长度（检查点为 0）。`steps.options_json` 存模型、温度、`reasoning`、`max_output_tokens`、`tool_choice` 等请求选项（不含输入条目）。`threads.kind = 'pending'` 表示类型还没判定，见任务 7。`system_blob` / `tools_blob` 引用 blob 时不建外键，孤儿清理统一走任务 9。

- [ ] **Step 1: 写测试**
  - `paths.rs` 内：`trajectory_db_lives_under_home`，断言 `AppPaths::under("/h").trajectory_db == "/h/trajectory.db"`。
  - `storage_repos.rs`：`trajectory_db_creates_schema_with_incremental_vacuum`。打开内存库，断言五张表存在、`PRAGMA user_version = 1`、`PRAGMA auto_vacuum = 2`（INCREMENTAL）。内存库不支持时改用 `tempfile` 文件库。
  - `storage_repos.rs`：`main_and_trajectory_migrations_are_independent`。同一个临时目录下分别打开主库和轨迹库，断言主库没有 `steps` 表，轨迹库没有 `providers` 表。
- [ ] **Step 2: 运行** `cargo test -p yi-core --test storage_repos trajectory` → FAIL。
- [ ] **Step 3: 实现。** `Db::build` 改为接收 `migrate: MigrateFn`。轨迹库迁移入口 `pub fn run(conn: &mut Connection) -> CoreResult<()>`：先执行 `PRAGMA auto_vacuum = INCREMENTAL`，再执行 `Migrations::new(vec![M::up(V1_SQL)]).to_latest(conn)`。
- [ ] **Step 4: 运行** `cargo test -p yi-core` → 全部 PASS（现有测试不能回归）。
- [ ] **Step 5: 验证** clippy。不提交。

### Task 3：主库 v4 迁移：`usage` 新列与轨迹设置

**Files:**
- Modify: `core/src/storage/migrations.rs`（新增 `v4()`，加入 `migrations()` 列表）
- Modify: `core/src/domain/usage.rs`、`core/src/domain/settings.rs`
- Modify: `core/src/storage/usage.rs`（insert / 查询带上新列）、`core/src/storage/settings.rs`
- Modify: `core/src/proxy/pipeline.rs`（`UsageGuard` 先给新字段填 `None`，任务 6 再填真实值）
- Modify: `core/src/storage/usage_writer.rs` 测试里的 `event()` 构造
- Test: `core/tests/storage_repos.rs`

**v4 SQL：**

```sql
ALTER TABLE usage ADD COLUMN step_id TEXT;
ALTER TABLE usage ADD COLUMN session_id TEXT;
ALTER TABLE usage ADD COLUMN ttft_ms INTEGER;
ALTER TABLE usage ADD COLUMN duration_ms INTEGER;
ALTER TABLE usage ADD COLUMN status INTEGER;
CREATE INDEX idx_usage_session ON usage(session_id);
ALTER TABLE settings ADD COLUMN trajectory_enabled INTEGER NOT NULL DEFAULT 1;
ALTER TABLE settings ADD COLUMN trajectory_disabled_clients TEXT NOT NULL DEFAULT '[]';
ALTER TABLE settings ADD COLUMN trajectory_retention_days INTEGER NOT NULL DEFAULT 30;
ALTER TABLE settings ADD COLUMN trajectory_max_bytes INTEGER NOT NULL DEFAULT 2147483648;
ALTER TABLE settings ADD COLUMN trajectory_notice_seen INTEGER NOT NULL DEFAULT 0;
```

**Interfaces:**
- Produces:
  - `UsageEvent` 和 `UsageRecord` 新增 `step_id: Option<String>`、`session_id: Option<String>`、`ttft_ms: Option<u32>`、`duration_ms: Option<u32>`、`status: Option<u16>`。
  - `domain::settings::TrajectorySettings { enabled: bool, disabled_clients: Vec<Client>, retention_days: u32, max_bytes: u64, notice_seen: bool }`（ts-rs 导出）。`max_bytes` 加 `#[ts(type = "number")]`。
  - `storage::settings::get_trajectory(conn) -> CoreResult<TrajectorySettings>`、`storage::settings::save_trajectory(conn, &TrajectorySettings) -> CoreResult<()>`。校验：`retention_days ∈ [1, 3650]`，`max_bytes ∈ [64 MiB, 1 TiB]`，否则返回 `CoreError::Validation`。

- [ ] **Step 1: 写测试**
  - `v4_adds_usage_columns_and_keeps_old_rows`：先只跑 v1–v3 插入一条旧 `usage` 行，再跑到最新版本。断言旧行可读，五个新字段为 `None`。
  - `trajectory_settings_round_trip_and_validation`：默认值为 `enabled=true, retention_days=30, max_bytes=2147483648, disabled_clients=[], notice_seen=false`；保存 `retention_days = 0` 返回 `Validation`。
  - `usage_insert_persists_new_columns`：写入带 `ttft_ms = Some(120)` 的事件，读回相等。
- [ ] **Step 2: 运行** → FAIL。
- [ ] **Step 3: 实现。** 字段名、默认值与 SQL 一致。`trajectory_disabled_clients` 用 `serde_json` 读写。
- [ ] **Step 4: 运行** `cargo test -p yi-core` → PASS；运行 `npm run gen:types`，确认 `src/generated/` 下 `UsageEvent.ts`、`UsageRecord.ts` 更新，并新增 `TrajectorySettings.ts`。
- [ ] **Step 5: 验证** `npm run typecheck`（前端若有用到 `UsageRecord` 的地方，类型新增可选字段不应报错）。不提交。

### Task 4：条目规范化、哈希与指纹

**Files:**
- Create: `core/src/trajectory/normalize.rs`
- Modify: `core/src/trajectory/mod.rs`
- Test: `core/src/trajectory/normalize.rs` 内部测试

**Interfaces:**
- Consumes: `crate::protocol::ir::{InputItem, Tool, Request, Response}`。
- Produces:

```rust
pub enum ItemKind { Message, ToolCall, ToolResult, Reasoning, System, Tools, Unknown }
pub struct NormalizedItem {
    pub kind: ItemKind,
    pub role: Option<String>,        // message 的 role
    pub call_id: Option<String>,
    pub tool_name: Option<String>,
    pub body: Vec<u8>,               // 规范化 JSON（键排序、紧凑），已截断、已剥离内联图片
    pub raw_bytes: u64,              // 截断前的字节数
    pub truncated: bool,
    pub hash: String,                // hex(SHA-256(body))
    pub fp: [u8; 32],                // 语义指纹，见下
}
pub fn normalize_item(item: &InputItem) -> NormalizedItem;
pub fn normalize_system(instructions: &str) -> NormalizedItem;
pub fn normalize_tools(tools: &[Tool]) -> Option<NormalizedItem>;   // 空列表返回 None
pub fn rolling_fp(prev: &[u8; 32], next: &[u8; 32]) -> [u8; 32];    // SHA-256(prev || next)
pub const EMPTY_FP: [u8; 32] = [0; 32];
pub fn first_user_text(items: &[NormalizedItem]) -> Option<String>; // 标题用，最多 80 个字符（按 char 计）
```

**规则：**
- **规范化 JSON**：`serde_json::to_value(item)`，递归把对象键排序，序列化为紧凑字符串。删除各层级的 `id`、`created_at` 字段（见下面指纹规则的说明；这里存储侧只删时间戳 `created_at`，**保留** `id`，因为检查器的原始页签需要它）。
- **截断**：`body` 超过 1 MiB 时，把其中最长的字符串字段截成"前 256 KiB + `…[truncated N bytes]…` + 后 256 KiB"（按 UTF-8 字符边界），`truncated = true`。截断后仍超限，就对整个 body 做同样处理并包成 `{"truncated_json": "<前后片段>"}`。
- **内联图片**：任何以 `data:` 开头、长度超过 64 KiB 的字符串值，替换为 `{"inline_image": {"media_type": "...", "bytes": N, "sha256": "..."}}`。
- **语义指纹 `fp`**：从同一个 item 另算一份"语义 JSON"再哈希，与 `body` 无关。
  - message：`{role, text}`，`text` 为所有文本片段按顺序拼接后去掉首尾空白；图片片段以它的 sha256 或 URL 代替。
  - tool_call：`{name, call_id, args}`，`args` 解析为 JSON 后键排序；解析失败时用去掉空白的原字符串。
  - tool_result：`{call_id, output}`，`output` 按文本处理，规则同 message。
  - reasoning：**固定常量** `{"reasoning": true}`。签名和摘要都可能被客户端改写，所以推理条目只按位置参与匹配。
  - unknown：`{type}`。

- [ ] **Step 1: 写测试**
  - `hash_is_independent_of_key_order`：两个只有键顺序不同的 message 得到相同的 `hash`。
  - `fp_ignores_ids_and_signatures`：同一条 assistant 消息，一份带 `id: "msg_1"` 和 extra 字段 `signature`，另一份没有，`fp` 相等、`hash` 不等。
  - `reasoning_fp_is_constant`：两条摘要不同的 reasoning 条目 `fp` 相等。
  - `tool_call_args_whitespace_does_not_change_fp`：`{"a":1}` 和 `{ "a" : 1 }` 的 `fp` 相等。
  - `large_text_is_truncated_on_char_boundary`：3 MiB 的中文文本，结果 `truncated = true`，`body` ≤ 1 MiB + 200 字节，并且是合法 UTF-8 / JSON。
  - `inline_image_over_64k_is_replaced`：断言 `body` 不含 `base64` 原文，并含 `inline_image`。
  - `first_user_text_takes_80_chars`：100 个汉字的用户消息返回前 80 个字符；只有 tool_result 时返回 `None`。
  - `rolling_fp_is_order_sensitive`：`rolling(rolling(E,a),b) != rolling(rolling(E,b),a)`。
- [ ] **Step 2: 运行** `cargo test -p yi-core trajectory::normalize` → FAIL。
- [ ] **Step 3: 实现。**
- [ ] **Step 4: 运行** → PASS。
- [ ] **Step 5: 验证** clippy。不提交。

### Task 5：blob 存储（去重 + zstd）

**Files:**
- Modify: `Cargo.toml`（workspace `zstd = "0.13"`）、`core/Cargo.toml`（`zstd.workspace = true`）
- Create: `core/src/storage/trajectory/blobs.rs`
- Test: 文件内部测试

**Interfaces:**
- Consumes: `NormalizedItem`（任务 4）。
- Produces:
  - `pub fn put(conn: &Connection, item: &NormalizedItem) -> CoreResult<bool>`：返回 `true` 表示新写入。使用 `INSERT OR IGNORE`，压缩级别 3。
  - `pub fn get(conn: &Connection, hash: &str) -> CoreResult<Option<StoredBlob>>`，`pub struct StoredBlob { pub kind: String, pub raw_bytes: u64, pub truncated: bool, pub json: String }`。
  - `pub fn delete_orphans(conn: &Connection, limit: u32) -> CoreResult<u32>`：删除未被 `step_items.blob`、`steps.system_blob`、`steps.tools_blob` 引用的 blob，返回删除条数。
  - `pub fn total_bytes(conn: &Connection) -> CoreResult<u64>`：`SUM(length(data))`。

- [ ] **Step 1: 写测试**：`put_is_idempotent`（第二次返回 `false`，行数为 1）；`round_trip_decompresses`；`orphans_are_removed_but_referenced_kept`（造一个被 `step_items` 引用的 blob 和一个孤儿 blob）；`compression_shrinks_repetitive_json`（100 KiB 重复文本压缩后小于 10 KiB）。
- [ ] **Step 2: 运行** → FAIL。
- [ ] **Step 3: 实现。** 孤儿清理 SQL：`DELETE FROM blobs WHERE hash IN (SELECT hash FROM blobs b WHERE NOT EXISTS (SELECT 1 FROM step_items i WHERE i.blob = b.hash) AND NOT EXISTS (SELECT 1 FROM steps s WHERE s.system_blob = b.hash) AND NOT EXISTS (SELECT 1 FROM steps s WHERE s.tools_blob = b.hash) LIMIT ?1)`。三个 `NOT EXISTS` 分别命中 `idx_step_items_blob`、`idx_steps_system_blob`、`idx_steps_tools_blob`（任务 2 已建）。写成三个而不是 `OR`，是为了让每个子查询都能走索引。测试里用 `EXPLAIN QUERY PLAN` 断言输出中不含 `SCAN steps`。
- [ ] **Step 4: 运行** → PASS；`cargo build --workspace` 在 Windows 上通过。
- [ ] **Step 5: 验证** clippy。不提交。

### Task 6：流事件 tap、会话键提取、`LiveRegistry`、管线挂接

**Files:**
- Modify: `core/src/protocol/mod.rs`
- Create: `core/src/proxy/capture.rs`、`core/src/proxy/live.rs`、`core/src/trajectory/keys.rs`、`core/src/trajectory/record.rs`
- Modify: `core/src/proxy/pipeline.rs`、`core/src/proxy/router.rs`、`core/src/proxy/mod.rs`
- Test: `core/tests/proxy_pipeline.rs`（追加）、`core/src/trajectory/keys.rs` 内部测试、`core/src/proxy/live.rs` 内部测试

**Interfaces:**

```rust
// protocol/mod.rs
pub type EventTap = Box<dyn FnMut(&ir::StreamEvent) + Send>;
pub trait StreamTranslator {
    fn on_frame(&mut self, f: SseFrame) -> CoreResult<Vec<Bytes>>;
    fn finish(&mut self) -> Vec<Bytes>;
    fn usage(&self) -> Usage;
    fn set_tap(&mut self, tap: EventTap);   // 新增
}
```

- `PassthroughStream` 和 `TranslatingStream` 都持有 `tap: Option<EventTap>`。`on_frame` 拿到 `source.on_frame` 产出的事件后，先逐个交给 tap，再走原逻辑。**`PassthroughStream::finish` 改为调用 `self.source.on_end()` 并把事件交给 tap，但不输出任何字节**，这样 Chat 直通流的结束事件也能被采集到。
- `PassthroughStream` 里 `source.on_frame` 出错时仍然忽略（保持"用量提取不影响转发"），只是不调用 tap。

```rust
// domain/session.rs（本任务新建；ts-rs 导出。任务 10 在同一文件追加其余 DTO）
pub enum KeySource { Client, Chain, New }                     // serde snake_case
pub enum LivePhase { Waiting, Streaming, Done, Error }        // serde snake_case
pub struct LiveRequest { pub seq: u64, pub step_id: String, pub client: Option<Client>, pub model: String,
    pub provider_name: String, pub started_at: i64, pub first_token_at: Option<i64>, pub finished_at: Option<i64>,
    pub phase: LivePhase, pub status: Option<u16>, pub input_tokens: u64, pub output_tokens: u64,
    pub session_id: Option<String> }   // session_id 由写入线程回填，见任务 8
pub struct LiveSnapshot { pub in_flight: Vec<LiveRequest>, pub completed: Vec<LiveRequest>, pub seq: u64 }
```

```rust
// trajectory/record.rs
pub enum StopReason { End, ToolUse, MaxTokens, Refusal, Error, Cancelled, Unknown }   // serde snake_case
pub enum CaptureLevel { Full, MetadataOnly }
pub struct ClientKeys { pub session_key: Option<String>, pub turn_id: Option<String> }
pub struct CaptureRecord {
    pub step_id: String,                 // UUID，与 usage.step_id 一致
    pub client: Option<Client>,
    pub keys: ClientKeys,
    pub inbound: Protocol,
    pub upstream: Protocol,
    pub provider_name: String,
    pub model: String,                   // exposed name，与 usage.model 一致
    pub upstream_model: String,
    pub route_mode: Option<RouteMode>,
    pub requested_at: i64,
    pub first_token_at: Option<i64>,
    pub finished_at: Option<i64>,
    pub status: Option<u16>,
    pub stop_reason: StopReason,
    pub error: Option<String>,           // 截断 4 KiB
    pub usage: Usage,
    pub request_body: Option<Bytes>,     // 客户端原始请求体
    pub response: Option<ir::Response>,  // Collector 结果，或非流式解析结果
    pub response_body: Option<Bytes>,    // 非流式且尚未解析时的原始响应
}
pub trait CaptureSink: Send + Sync {
    fn enabled_for(&self, client: Option<Client>) -> bool;
    fn submit(&self, record: CaptureRecord);   // 绝不阻塞
}
/// 可在运行中替换的采集出口。arc-swap 只能存放 Sized 类型，所以用一层具体包装。
pub struct CaptureHandle(pub Arc<dyn CaptureSink>);
#[derive(Clone, Default)]
pub struct CaptureSlot(Arc<arc_swap::ArcSwapOption<CaptureHandle>>);
impl CaptureSlot {
    pub fn set(&self, sink: Option<Arc<dyn CaptureSink>>);
    pub fn get(&self) -> Option<Arc<dyn CaptureSink>>;   // load_full 后取出内部 Arc
}
```

```rust
// trajectory/keys.rs
pub fn extract(client: Option<Client>, headers: &HeaderMap, body: &Value) -> ClientKeys;
```

规则以附录 A 的抓包结论为准。在结论出来之前，按 spec 4.3 实现：
- 所有客户端：`session_id` / `session-id` / `x-session-id` 头，取第一个非空的。
- Codex：`turn_id` 取 `x-codex-turn-id` 头。
- Claude Code：`metadata.user_id` 里匹配 `_session_([0-9a-f-]{36})`，取第一个捕获组。不要为此引入 `regex` crate，手写查找子串 `"_session_"` 之后截取 36 个字符，再校验是否为 UUID 格式。
- `session_key` 的最终值加客户端前缀，例如 `claude-code:<uuid>`，避免不同客户端之间撞值。

```rust
// proxy/capture.rs
pub struct CaptureTap { inner: Arc<Mutex<TapState>> }
struct TapState { collector: Collector, first_token_at: Option<i64>, saw_error: Option<String> }
impl CaptureTap {
    pub fn new() -> Self;
    pub fn tap(&self) -> EventTap;       // 遇到第一个 OutputTextDelta / FunctionCallArgumentsDelta / ReasoningSummaryTextDelta 时记录 now_ms
    pub fn take(&self) -> (Option<ir::Response>, Option<i64>, Option<String>);
}
pub fn stop_reason(resp: Option<&ir::Response>, status: Option<u16>, finished: bool, error: Option<&str>) -> StopReason;
```

`stop_reason` 规则：
- 未 `finish`（客户端断开）→ `Cancelled`。
- `status >= 400` 或有 error → `Error`。
- `resp.status == "incomplete"` 且 `incomplete_details.reason == "max_output_tokens"` → `MaxTokens`。
- 输出含 `FunctionCall` → `ToolUse`。
- 输出中有 message 的 extra 含 `refusal` → `Refusal`。
- `resp.status == "completed"` → `End`。
- 其余 → `Unknown`。

```rust
// proxy/live.rs（LivePhase / LiveRequest / LiveSnapshot 来自 domain::session）
#[derive(Clone, Default)] pub struct LiveRegistry { .. }
impl LiveRegistry {
    pub fn begin(&self, step_id: &str, client: Option<Client>, model: &str, provider: &str, started_at: i64);
    pub fn first_token(&self, step_id: &str, at: i64);
    pub fn finish(&self, step_id: &str, phase: LivePhase, status: Option<u16>, usage: Usage, at: Option<i64>);
    pub fn set_session(&self, step_id: &str, session_id: &str);
    pub fn snapshot(&self, since_seq: u64) -> LiveSnapshot;
    pub fn clear(&self);
}
```

`seq` 在 `finish` 时分配，全局单调递增。`completed` 只返回 `seq > since_seq` 的条目，最多 500 条；环形缓冲同样是 500 条。

**管线挂接（`pipeline.rs`）：**
1. `run` 开头：`let step_id = Uuid::new_v4()`；`let sink = state.capture.get().filter(|c| c.enabled_for(client))`；`let request_body = sink.is_some().then(|| body.clone())`（在 `serde_json::from_slice` 之前，`Bytes::clone` 只增加引用计数）。请求开始时取一次 sink，整个请求都用这一份，中途替换不影响在途请求。
2. 解析出 `raw` 之后：`keys = trajectory::keys::extract(client, inbound_headers, &raw)`。
3. `UsageGuard` 改名为 `RequestGuard`（文件内私有），新增字段 `step_id`、`keys`、`request_body`、`tap: Option<CaptureTap>`、`first_token_at`、`status`、`error`、`response`、`response_body`，以及 `capture: Option<Arc<dyn CaptureSink>>`、`live: LiveRegistry`。
4. 发出上游请求之前调用 `live.begin(..)`。
5. 流式：`if let Some(tap) = &guard.tap { translator.set_tap(tap.tap()) }`；每个 chunk 之后，`first_token_at` 第一次出现时调用 `live.first_token`。
6. 非流式：在 `offload` 闭包里，`passthrough` 分支已经解析过 `translate_response`，把解析出的 `ir::Response` 一并返回（改为返回 `(Bytes, Usage, Option<ir::Response>)`），不要重复解析。
7. `Drop`：
   - 保持原有的"写 usage"条件；`UsageEvent` 填上 `step_id`、`ttft_ms`、`duration_ms`、`status`。`session_id` 在这里还不知道，留空，由写入线程用 `UPDATE usage SET session_id = ? WHERE step_id = ?` 回填，见任务 8。
   - 无论是否写 usage，只要 `meta` 存在，就调用 `live.finish`；`capture_on` 时再构造 `CaptureRecord` 调用 `submit`。
   - 一个请求恰好投递一次：`meta.take()` 已经保证了这一点。
8. 上游返回错误时，`state.http.send_with` 会返回 `Err`，`?` 让 guard 在 `finished = false` 的状态下被 drop。为了区分"上游错误"和"客户端断开"，在 `send_with` 的 `Err` 分支先设置 `guard.status` 和 `guard.error`，再返回错误；`stop_reason` 看到 `error` 就会判为 `Error` 而不是 `Cancelled`。`UpstreamError { status, body }` 的结构见 `proxy/upstream.rs`。
9. `ProxyState` 增加 `pub capture: CaptureSlot` 和 `pub live: LiveRegistry`。现有的 `new` / `with_monitor` / `with_http` 构造函数都填默认值（空 slot、空 registry），所有现有测试无需修改。测试和 `Services` 通过 `state.capture.set(Some(sink))` 注入。

- [ ] **Step 1: 写测试**
  - `keys.rs`：`codex_header_and_turn_id`；`claude_code_user_id_session_suffix`（`user_abc_account_x_session_123e4567-e89b-12d3-a456-426614174000` → `claude-code:123e4567-…`）；`malformed_user_id_yields_none`；`header_wins_over_body`。
  - `live.rs`：`snapshot_since_returns_only_new_completed`；`ring_keeps_last_500`；`in_flight_moves_to_completed_on_finish`。
  - `proxy_pipeline.rs`，用一个把记录推进 `Mutex<Vec<CaptureRecord>>` 的测试用 `CaptureSink`：
    - `capture_passthrough_stream_collects_output_and_ttft`：Anthropic → Anthropic 流式，wiremock 返回 `tests/fixtures/anthropic/stream_text_tool.sse`。断言恰好 1 条记录；`response.output` 含一条 message 和一条 function_call；`first_token_at.is_some()`；`stop_reason == ToolUse`；客户端收到的字节与不启用采集时完全相同（同一测试里跑两遍逐字节比较）。
    - `capture_translated_stream_matches_client_view`：客户端说 Chat，上游是 Anthropic。断言 `response.output` 的文本与客户端收到的 Chat 增量拼接结果一致。
    - `capture_chat_passthrough_includes_on_end_events`：Chat → Chat 流式，上游最后一帧只有 `[DONE]`、没有 usage 帧。断言 `response.is_some()`，说明 `on_end` 的事件经过了 tap。
    - `capture_non_stream_reuses_parse`：非流式直通，断言 `response.is_some()` 且 `usage` 与 usage 表中的那一行一致。
    - `upstream_error_is_error_not_cancelled`：wiremock 返回 500，断言 `stop_reason == Error`，`status == Some(500)`，`error` 含上游响应体。
    - `capture_disabled_for_client_submits_nothing`：`enabled_for` 返回 false 时没有记录，但 `usage` 照常写入。
    - `usage_row_has_step_id_and_timings`：断言 `usage.step_id == record.step_id`，`ttft_ms` 和 `duration_ms` 有值。
    - `capture_overhead_is_small`（spec 第 9 节的性能基线）：标记 `#[ignore]`，在任务 21 手动执行 `cargo test -p yi-core --release --test proxy_pipeline capture_overhead -- --ignored`。上游 wiremock 立即返回固定响应；4 MiB 请求体分别在关闭和开启采集时各发 200 次（采集出口用一个只 `drop` 记录的 sink）。断言开启采集后 p50 增量 < 1 ms，并打印 p50 / p95。
- [ ] **Step 2: 运行** `cargo test -p yi-core --test proxy_pipeline capture` → FAIL。
- [ ] **Step 3: 实现。**
- [ ] **Step 4: 运行** `cargo test --workspace` → PASS，`insta` 快照没有变化。
- [ ] **Step 5: 验证** clippy。不提交。

### Task 7：链接器：会话、线程、父步骤、回合、线程类型

**Files:**
- Create: `core/src/trajectory/link.rs`
- Test: 文件内部测试（纯内存逻辑，不依赖 SQLite）

这里把判定逻辑和存储分开：链接器只依赖一个查询 trait，写入线程用 SQLite 实现它，测试用内存实现它。

**Interfaces:**

```rust
pub struct StepInput<'a> {
    pub client: Option<Client>,
    pub keys: &'a ClientKeys,
    pub requested_at: i64,
    pub input: &'a [NormalizedItem],      // 完整输入条目（不含 system / tools）
    pub output: &'a [NormalizedItem],     // 本步输出条目
    pub has_tools: bool,
    pub model: &'a str,
}
pub struct TailEntry {                   // 一个可以被续接的步骤尾部
    pub step_id: String, pub session_id: String, pub thread_id: String,
    pub client: Option<Client>, pub last_at: i64,
    pub tail_len: usize,                 // 输入 + 输出的条目数
    pub turn_no: u32, pub depth: u32,
}
pub trait LinkStore {
    fn session_by_key(&self, client: Option<Client>, key: &str) -> Option<String>;
    fn tail_by_fp(&self, fp: &[u8; 32]) -> Option<TailEntry>;   // 先查 LRU，再查 steps.tail_fp
    fn open_tool_calls(&self, session_id: &str) -> Vec<OpenCall>; // 该会话中还没有配对结果的工具调用
    fn thread_kind(&self, thread_id: &str) -> Option<ThreadKind>;
}
pub struct OpenCall { pub thread_id: String, pub call_id: String, pub prompt_fp: Option<[u8; 32]> } // prompt_fp：参数 prompt 字段文本的 fp
pub enum ThreadKind { Main, Subagent, Side, Pending }
pub enum SessionDecision { Existing(String), New { key_source: KeySource, client_key: Option<String> } }
pub enum ThreadDecision { Continue { thread_id: String, parent_step_id: String, prefix_len: usize },
                          New { kind: ThreadKind, parent_thread_id: Option<String>, spawn_call_id: Option<String>, reset: bool } }
pub struct LinkResult {
    pub session: SessionDecision,
    pub thread: ThreadDecision,
    pub turn_no: u32,
    pub opens_turn: bool,
    pub checkpoint: bool,                // true 时存完整输入，prefix_len 视为 0
    pub depth: u32,
    pub tail_fp: [u8; 32],               // fold(input ++ output)
}
pub fn link(store: &dyn LinkStore, step: &StepInput, now: i64) -> LinkResult;
pub const CHECKPOINT_EVERY: u32 = 64;
pub const CHAIN_WINDOW_MS: i64 = 12 * 3600 * 1000;
```

**算法**（与 spec 4.3、4.4 一致）：
1. 计算 `prefix_fps[k] = fold(input[..k])`，k = 0..=n。
2. 从 k = n 往下找第一个满足以下条件的 `tail_by_fp(prefix_fps[k])`：同一 `client`、`now - last_at <= CHAIN_WINDOW_MS`、并且 `tail_len == k`。得到 `parent`（最长前缀匹配）。
3. 判定会话：
   - 有 `keys.session_key`：`session_by_key` 命中则为 `Existing`，否则 `New { key_source: Client }`。
   - 没有：有 `parent` 则为 `Existing(parent.session_id)`（`key_source = chain`），否则 `New { key_source: New }`。
4. 判定线程：
   - `parent` 存在且与会话相同 → `Continue { prefix_len: k }`。
   - 否则新线程。首条用户消息的 fp 等于某个 `open_tool_calls` 的 `prompt_fp` → `Subagent`，记录 `parent_thread_id` 和 `spawn_call_id`。
   - 会话通过客户端会话标识命中，但前缀不匹配，并且会话里已有 `Main` 线程 → `reset = true`，类型 `Pending`。
   - 其余 → `Pending`。
5. 线程类型在后续步骤补充：`Pending` 线程第一次出现工具调用、并且会话里还没有 `Main` 时升为 `Main`。写入线程在会话空闲 10 分钟后，或者在任务 10 投影时，把只有 1 步且没有工具的 `Pending` 线程视为 `Side`。投影时动态计算，不需要回写。
6. 回合：`keys.turn_id` 有值时，与父步骤的 `client_turn_id` 不同就开新回合。否则，在新增条目（`input[prefix_len..]`）里找带非空文本的 `role == "user"` 消息，找到就 `opens_turn = true`，`turn_no = parent.turn_no + 1`；否则沿用 `parent.turn_no`。新线程从 `turn_no = 1` 开始；子 Agent 线程有自己的回合号。
7. 检查点：`depth = parent.depth + 1`；`depth >= CHECKPOINT_EVERY` 时 `checkpoint = true`、`depth = 0`。新线程 `depth = 0`、`checkpoint = true`。
8. `tail_fp = fold(input ++ output)`。

- [ ] **Step 1: 写测试**（内存 `LinkStore`，每次 `link` 之后由测试辅助函数模拟"写入"，更新索引）
  - `five_step_tool_loop_is_one_session_one_thread_one_turn`：用户消息 → 工具调用 → 工具结果 → … → 最终回答，共 5 步。断言只有一个会话、一个线程，`turn_no` 都是 1，第 2–5 步的 `prefix_len` 分别等于上一步的 `tail_len`。
  - `chain_survives_stripped_reasoning_and_new_ids`（Review Focus 1）：第 1 步输出含带签名的 reasoning 和 `id = "msg_a"` 的 message；第 2 步输入里 reasoning 没有签名、message 的 `id = "msg_b"`。断言 `Continue`。
  - `same_first_message_different_sessions_are_not_merged`：两个会话的首条用户消息都是 "hi"，第二个会话的第二步历史与第一个会话不同。断言第二个会话的第二步接在第二个会话上；两个会话在第一步都是 `New`，因为空前缀不参与匹配，必须 k ≥ 1 才算命中。
  - `client_key_wins_and_reset_detected`：带同一个 `session_key`、但历史被压缩重写的步骤，断言 `Existing` 加上 `New { reset: true }`。
  - `subagent_thread_links_to_spawning_call`：主线程有一个未配对的 `Task` 调用，参数 `prompt = "检查测试"`；新请求的首条用户消息是 "检查测试"。断言 `Subagent`，`spawn_call_id` 正确。
  - `new_user_message_opens_turn`、`tool_results_only_do_not_open_turn`。
  - `codex_turn_id_controls_turns`：同一 `turn_id` 的多步回合号相同，换 `turn_id` 后加 1。
  - `checkpoint_every_64_steps`：连续 130 步，断言第 1、65、129 步 `checkpoint = true`，其余 `depth` 在 1..=63。
  - `chain_window_expires_after_12h`：父步骤 `last_at` 在 13 小时前，断言 `New`。
  - `chain_does_not_cross_clients`。
- [ ] **Step 2: 运行** `cargo test -p yi-core trajectory::link` → FAIL。
- [ ] **Step 3: 实现。**
- [ ] **Step 4: 运行** → PASS。
- [ ] **Step 5: 验证** clippy。不提交。

### Task 8：采集写入线程

**Files:**
- Create: `core/src/storage/trajectory/repo.rs`、`core/src/storage/trajectory/writer.rs`
- Test: `core/tests/trajectory_capture.rs`（新建），以及文件内部测试

**Interfaces:**
- Consumes: 任务 4、5、6、7 的全部接口；`protocol::to_ir`；`Db`（主库，用于回填 `usage.session_id`）。
- Produces:

```rust
pub struct TrajectorySink { .. }   // impl CaptureSink
impl TrajectorySink {
    pub fn set_settings(&self, s: &TrajectorySettings);   // 原子更新 enabled 和 disabled_clients
}
pub struct TrajectoryWriterHandle { .. }
impl TrajectoryWriterHandle { pub fn shutdown(self); pub fn flush_for_test(&self); }
pub fn spawn_trajectory_writer(tdb: TrajectoryDb, main: Db, live: LiveRegistry, settings: TrajectorySettings)
    -> (Arc<TrajectorySink>, TrajectoryWriterHandle);
pub const INFLIGHT_LIMIT_BYTES: usize = 64 * 1024 * 1024;
```

**行为：**
- **队列**：`std::sync::mpsc::sync_channel(4096)`，加上 `AtomicUsize inflight_bytes`。`submit` 时先计算 `request_body.len() + response_body.len()`；加上后会超过 `INFLIGHT_LIMIT_BYTES` 时，把 `request_body`、`response_body`、`response` 置为 `None`，并标记为 `MetadataOnly`。`try_send` 失败时 warn 并丢弃，但要先在 `LiveRegistry` 里保留这条请求（它不依赖写入线程）。
- **处理一条记录**：
  1. `to_ir(inbound, serde_json::from_slice(request_body))` 得到请求 IR；失败时降级为 `MetadataOnly`，并 `tracing::warn!`。
  2. 规范化：`instructions` → system blob；`tools` → tools blob；`input` → 条目列表；`response.output`（为空时尝试用 `response_body` 和 `translate_response` 同一套解析）→ 输出条目。
  3. 在同一个事务里：`link` → 写 session（新建或 `UPDATE` 汇总）→ 写 thread → 写 step → 写 blob → 写 `step_items`。`input` 侧写 `checkpoint ? 全部 : input[prefix_len..]`；`output` 侧写全部；`ord` 从 0 开始。
  4. `MetadataOnly` 时：仍然调用 `link`（输入、输出都为空），归入 `client_key` 指向的会话，没有标识就新建；`tail_fp` 写成 `"meta:" + step_id`，保证不会被匹配。
  5. 事务提交后：用主库执行 `UPDATE usage SET session_id = ?1 WHERE step_id = ?2`，再调用 `live.set_session(step_id, session_id)`。主库更新失败只 warn。
- **批次**：累计 64 条或 32 MiB 原文就提交一次，或者每 500ms 提交一次。每条记录一个 SAVEPOINT，单条失败只回滚这一条。
- **尾部 LRU**：写入线程持有 `lru: HashMap<[u8;32], TailEntry>` 加 `VecDeque` 实现的 2000 项 LRU（不引入 lru crate）。`SqliteLinkStore` 先查 LRU，未命中再查 `SELECT … FROM steps WHERE tail_fp = ?`。
- **会话标题**：会话新建时取 `first_user_text(input)`；为空时，在后续步骤第一次拿到非空值时补上。
- **会话 `bytes`**：每条新写入 blob 的压缩后大小累加到该会话；已存在的 blob 不重复计。

- [ ] **Step 1: 写测试**（`core/tests/trajectory_capture.rs`，端到端：wiremock 上游 + 真实 axum 代理 + 内存主库和轨迹库）
  - `tool_loop_writes_linear_storage`：5 步工具循环。断言 1 个会话、1 个线程、5 个步骤；`step_items` 中 `input` 侧的总条数等于"每步新增条目数之和"，而不是 1+2+…+n。
  - `system_and_tools_are_stored_once`：5 步都带相同的 30 KiB 工具定义，断言 `blobs` 中 `kind = 'tools'` 只有 1 行。
  - `client_disconnect_mid_stream_keeps_partial_output`（Review Focus 2）：wiremock 用慢速 SSE 发出 3 帧文本增量后挂起；客户端读完 2 帧就 drop。断言步骤 `stop_reason = 'cancelled'`、`finished_at IS NULL`，输出条目的文本是已收到增量的拼接；`usage` 表只有 1 行；`LiveRegistry` 中该请求为 `Error` 或 `Done`，不会一直在途。
  - `oversized_or_saturated_capture_degrades_to_metadata`（Review Focus 3）：把 `INFLIGHT_LIMIT_BYTES` 通过测试构造函数调成 1 MiB，发送一个 8 MiB 请求。断言 HTTP 200；客户端收到的字节与上游一致；步骤 `capture = 'metadata_only'`；会话列表仍包含这一步。
  - `usage_session_id_is_backfilled`。
  - `bad_request_json_degrades_without_failing_proxy`：上游接受，但请求体是 `to_ir` 无法解析的合法 JSON（例如缺少 `messages` 的 Anthropic 请求，并让 wiremock 返回 200）。断言请求成功，步骤为 `metadata_only`。
- [ ] **Step 2: 运行** `cargo test -p yi-core --test trajectory_capture` → FAIL。
- [ ] **Step 3: 实现。**
- [ ] **Step 4: 运行** → PASS；`cargo test --workspace` → PASS。
- [ ] **Step 5: 验证** clippy。不提交。

### Task 9：保留与清理

**Files:**
- Create: `core/src/storage/trajectory/retention.rs`
- Modify: `core/src/storage/trajectory/writer.rs`（每写 256 步调用一次；启动时调用一次）
- Test: 文件内部测试

**Interfaces:**

```rust
pub struct RetentionReport { pub sessions_deleted: u32, pub blobs_deleted: u32, pub bytes_after: u64 }
pub fn enforce(conn: &Connection, retention_days: u32, max_bytes: u64, now: i64) -> CoreResult<RetentionReport>;
pub fn delete_session(conn: &Connection, id: &str) -> CoreResult<()>;
pub fn clear_client(conn: &Connection, client: Client) -> CoreResult<u32>;
pub fn vacuum_if_needed(conn: &Connection, deleted_bytes: u64, total_bytes: u64) -> CoreResult<bool>; // deleted > 25% total 时执行 PRAGMA incremental_vacuum
```

清空全部不在这里实现，由任务 10 的服务层关闭写入线程、删除文件、重建库、再重启写入线程。

**行为：**
- 先删除 `last_at < now - days` 的会话。然后只要 `blobs::total_bytes > max_bytes`，就按 `last_at` 升序继续删会话，每轮最多 20 个会话。
- 每删完一轮，调用 `blobs::delete_orphans(conn, 1000)` 直到返回 0。
- 每个会话的删除放在单独的事务里，避免长事务阻塞写入。

- [ ] **Step 1: 写测试**：`expired_sessions_are_deleted`；`over_capacity_deletes_oldest_first`；`shared_blob_survives_when_other_session_references_it`；`delete_session_cascades_steps_and_items`；`clear_client_only_touches_that_client`。
- [ ] **Step 2: 运行** → FAIL。
- [ ] **Step 3: 实现。**
- [ ] **Step 4: 运行** → PASS。
- [ ] **Step 5: 验证** clippy。不提交。

### Task 10：投影与会话服务

**Files:**
- Create: `core/src/domain/session.rs`、`core/src/services/trajectory_view.rs`、`core/src/services/sessions.rs`
- Modify: `core/src/domain/mod.rs`、`core/src/services/mod.rs`、`core/src/storage/trajectory/repo.rs`（读取函数）
- Test: `core/tests/trajectory_view.rs`（新建）

**Interfaces（DTO 全部 ts-rs 导出到 `src/generated/`，数字字段按现有习惯加 `#[ts(type = "number")]`）：**

```rust
// 追加到 domain/session.rs（KeySource、LivePhase 已在任务 6 定义）
pub enum ThreadKindView { Main, Subagent, Side, Other }   // Pending 投影成 Side 或 Other
pub struct SessionSummary { pub id: String, pub client: Option<Client>, pub title: String, pub key_source: KeySource,
    pub started_at: i64, pub last_at: i64, pub step_count: u32, pub input_tokens: u64, pub output_tokens: u64,
    pub bytes: u64, pub models: Vec<String>, pub live: bool }   // live：LiveRegistry 中有该会话的在途请求
pub struct SessionListInput { pub client: Option<Client>, pub query: Option<String>, pub model: Option<String>,
    pub start_ms: Option<i64>, pub end_ms: Option<i64>, pub cursor: Option<String>, pub limit: Option<u32> }
pub struct SessionPage { pub items: Vec<SessionSummary>, pub next_cursor: Option<String> }  // 游标 = "{last_at}:{id}"
pub struct ThreadView { pub id: String, pub kind: ThreadKindView, pub parent_thread_id: Option<String>,
    pub spawn_call_id: Option<String>, pub reset: bool, pub step_count: u32, pub started_at: i64 }
pub struct SessionDetail { pub summary: SessionSummary, pub threads: Vec<ThreadView> }

pub enum RecordKind { System, User, Message, Tool, Subtool, Compacted }   // 对应 deepseek-harness 的 TrajectoryCellKind
pub struct TrajectoryRecord {
    pub id: String,                  // 稳定 ID："{step_id}:{side}:{ord}"，tool 用 "call:{call_id}"
    pub kind: RecordKind,
    pub step_id: String,
    pub text: String,                // 单行摘要，最多 200 个字符
    pub body: Option<String>,        // 正文摘要，最多 2 KiB；None 表示没有正文
    pub body_blob: Option<String>,   // 截断或超过摘要长度时，用于加载全文
    pub truncated: bool,
    pub thinking: Option<String>,    // message：推理摘要，最多 2 KiB
    pub call_id: Option<String>, pub tool_name: Option<String>,
    pub args: Option<String>,        // tool：参数，最多 2 KiB
    pub result: Option<String>,      // tool：结果单行摘要
    pub result_body: Option<String>, pub result_blob: Option<String>,
    pub result_step_id: Option<String>,   // 结果所在的步骤
    pub is_error: bool,
    pub started_at: Option<i64>,
    pub duration_ms: Option<i64>,
    pub duration_estimated: bool,    // tool 的时长是估算值
    pub sub_thread_id: Option<String>,    // subtool：子 Agent 线程
    pub system_changed: bool,        // system：相对上一次是否变化
}
pub struct TrajectoryStep {
    pub id: String, pub number: u32,  // number：会话内从 1 起的"请求 #N"
    pub thread_id: String, pub turn_no: u32,
    pub requested_at: i64, pub first_token_at: Option<i64>, pub finished_at: Option<i64>,
    pub status: Option<u16>, pub stop_reason: String, pub error: Option<String>,
    pub model: String, pub upstream_model: String, pub provider_name: String,
    pub inbound_protocol: Protocol, pub upstream_protocol: Protocol, pub route_mode: Option<RouteMode>,
    pub input_tokens: u64, pub output_tokens: u64, pub cached_tokens: u64, pub reasoning_tokens: u64,
    pub capture: String, pub live_phase: Option<LivePhase>,
    pub records: Vec<TrajectoryRecord>,
}
pub struct TrajectoryTurn { pub turn_no: u32, pub thread_id: String, pub steps: Vec<TrajectoryStep> }
pub struct TrajectoryInput { pub session_id: String, pub thread_id: Option<String>, pub before_turn: Option<u32>, pub limit_turns: Option<u32> }
pub struct TrajectoryPage { pub turns: Vec<TrajectoryTurn>, pub has_older: bool, pub seq: u64 }  // seq：本页最大 steps.seq
pub struct StepContext { pub step: TrajectoryStep, pub system: Option<String>, pub tools: Option<String>,
    pub input: Vec<ContextItem>, pub output: Vec<ContextItem>, pub options: serde_json::Value, pub chain_depth: u32 }
pub struct ContextItem { pub kind: String, pub role: Option<String>, pub json: String, pub truncated: bool }
pub struct BlobView { pub kind: String, pub raw_bytes: u64, pub truncated: bool, pub json: String }
pub struct SessionChanges { pub session_ids: Vec<String>, pub seq: u64 }
pub struct TrajectoryStatus { pub available: bool, pub error: Option<String>, pub bytes: u64, pub sessions: u32,
    pub settings: TrajectorySettings }
```

**投影规则（`trajectory_view.rs`）**，每条都与 spec 8.3.1 的表对应：
- 默认线程：`thread_id` 为空时取 `Main`，没有 `Main` 就取步骤最多的线程。
- 每个步骤开头：线程首步，或者 `system_blob` / `tools_blob` 与本线程上一步不同时，生成一条 `System` 记录（`system_changed` 只在后者为 true）。
- `input[prefix_len..]` 里带文本的 user message → `User`；tool_result 条目不生成记录，按 `call_id` 回填到对应 `Tool` 记录的 `result*` 字段（结果可能在下一页：同一次查询里额外 `SELECT … FROM step_items WHERE call_id IN (…) AND side = 'input'`）。
- `output` 中的 message 和 reasoning 合成一条 `Message`：`text` = 文本，`thinking` = 推理摘要；`started_at = requested_at`，`duration_ms = finished_at - requested_at`。
- `output` 中的 function_call → `Tool`：`started_at = 本步 finished_at`；`duration_ms = 结果所在步骤的 requested_at - 本步 finished_at`；`duration_estimated = true`。
- 有子 Agent 线程的 `spawn_call_id` 等于该 `call_id` → 这条 `Tool` 改为 `Subtool`，并填 `sub_thread_id`。
- 线程 `reset = true` 的首步之前插入一条 `Compacted`。
- `Pending` 线程：只有 1 步且没有工具 → `Side`，否则 → `Other`。
- `number`：会话内按 `seq` 排序后的序号（所有线程统一编号）。
- `live_phase`：从 `LiveRegistry` 取；在途请求还没写入轨迹库时，投影在本线程末尾追加一个 `records` 为空的占位步骤（用 `LiveRequest.session_id` 匹配；没有 `session_id` 的在途请求不显示）。
- **分页**：按 `turn_no` 倒序取 `limit_turns`（默认 20）个回合，`before_turn` 为排他上界；返回时按升序排列。

**完整上下文还原（`StepContext`）**：沿 `parent_step_id` 向上，直到遇到 `checkpoint = 1` 的步骤，把各步的 `input` 侧条目按顺序拼接，前面一步的 `output` 也要接上，因为前缀包含上一步的输出。用 `WITH RECURSIVE` 一次取出整条链。`chain_depth` 返回实际回溯层数。

**服务方法（`services/sessions.rs`，全部通过 `self.run` 在阻塞池执行）：**

```rust
pub async fn list_sessions(self: &Arc<Self>, input: SessionListInput) -> CoreResult<SessionPage>;
pub async fn get_session(self: &Arc<Self>, id: String) -> CoreResult<SessionDetail>;
pub async fn get_trajectory(self: &Arc<Self>, input: TrajectoryInput) -> CoreResult<TrajectoryPage>;
pub async fn get_step_context(self: &Arc<Self>, step_id: String) -> CoreResult<StepContext>;
pub async fn get_blob(self: &Arc<Self>, hash: String) -> CoreResult<BlobView>;
pub async fn session_changes(self: &Arc<Self>, since_seq: u64) -> CoreResult<SessionChanges>;
pub async fn delete_session(self: &Arc<Self>, id: String) -> CoreResult<()>;
pub async fn clear_sessions(self: &Arc<Self>, client: Option<Client>) -> CoreResult<u32>;  // None = 全部：关闭写入线程 → 删除并重建文件 → 重启写入线程
pub async fn trajectory_status(self: &Arc<Self>) -> CoreResult<TrajectoryStatus>;
pub async fn save_trajectory_settings(self: &Arc<Self>, s: TrajectorySettings) -> CoreResult<()>;  // 同时调用 sink.set_settings
pub async fn rebuild_trajectory_db(self: &Arc<Self>) -> CoreResult<()>;
```

轨迹库不可用时，以上方法除 `trajectory_status`、`save_trajectory_settings`、`rebuild_trajectory_db` 外，一律返回 `CoreError::Conflict("轨迹库不可用：{原因}")`。

- [ ] **Step 1: 写测试**（`core/tests/trajectory_view.rs`，用任务 8 的端到端夹具产生数据，再调用服务方法）
  - `projection_pairs_tool_call_with_result`：调用在第 1 步、结果在第 2 步的输入里。断言只有一条 `Tool` 记录，`result_step_id` 为第 2 步，`duration_estimated = true`。
  - `system_record_on_first_step_and_on_change`：第 3 步工具定义变化，断言出现两条 `System`，第二条 `system_changed = true`。
  - `subagent_is_projected_as_subtool`。
  - `reset_inserts_compacted_record`。
  - `pagination_by_turns`：25 个回合，第一页返回最后 20 个，`has_older = true`；`before_turn = 6` 返回第 1–5 回合。
  - `long_session_reconstruction_is_bounded_by_checkpoints`（Review Focus 5）：200 步，断言第 200 步的 `chain_depth <= 64`，还原出的 `input` 条数等于第 200 步请求实际发送的条数，最后一条与请求体最后一条语义相等（比较 fp）。
  - `live_request_appears_as_placeholder_step`。
  - `list_sessions_keyset_pagination_and_filters`：按终端、标题关键字、模型筛选；游标翻页不重复、不遗漏。
  - `session_changes_returns_sessions_after_seq`。
  - `clear_all_recreates_db_and_capture_continues`：清空全部后再发一个请求，断言能写入新会话。
- [ ] **Step 2: 运行** `cargo test -p yi-core --test trajectory_view` → FAIL。
- [ ] **Step 3: 实现**；`npm run gen:types`。
- [ ] **Step 4: 运行** → PASS。
- [ ] **Step 5: 验证** clippy。不提交。

### Task 11：服务启动接线、失败隔离与 Tauri 命令

**Files:**
- Modify: `core/src/services/mod.rs`（`start_inner`、`Drop`）、`core/src/services/proxy.rs`（`live_requests`）
- Modify: `src-tauri/src/commands.rs`、`src-tauri/src/lib.rs`
- Test: `core/tests/services_basic.rs`（追加）、`src-tauri` 中已有的 `*_impl` 测试风格

**Interfaces:**
- `Services` 新增字段：`trajectory: Mutex<TrajectoryState>`。

```rust
enum TrajectoryState {
    Ready { db: TrajectoryDb, sink: Arc<TrajectorySink>, writer: TrajectoryWriterHandle },
    Unavailable { error: String },
}
```

- `Services` 持有 `ProxyState.capture` 那个 `CaptureSlot` 的克隆（任务 6）。启动成功时调用 `slot.set(Some(sink))`；清空全部或重建库时先 `slot.set(None)`，再关闭写入线程，换库后再 `set(Some(new_sink))`。重建期间到达的请求只写 usage、不采集，这是预期行为。
- `pub fn live_requests(&self, since_seq: u64) -> LiveSnapshot`。
- Tauri 命令（`camelCase` 参数遵循现有习惯）：`list_sessions`、`get_session`、`get_trajectory`、`get_step_context`、`get_blob`、`get_session_changes`、`delete_session`、`clear_sessions`、`get_trajectory_status`、`save_trajectory_settings`、`rebuild_trajectory_db`、`get_live_requests`。

**行为：**
- `start_inner` 打开主库之后打开轨迹库。失败时 `tracing::error!`，状态记为 `Unavailable`，`ProxyState.capture` 为 `None`，启动继续。
- 读取 `TrajectorySettings`；`enabled = false` 时仍然启动写入线程，`enabled_for` 一律返回 false，这样之后打开开关不需要重启。
- 启动写入线程后立即在写入线程内执行一次 `retention::enforce`。
- `Drop` 时先 shutdown 轨迹写入线程，再 shutdown usage 写入线程。
- `rebuild_trajectory_db`：关闭写入线程，把旧文件重命名为 `trajectory.db.broken-<时间戳>`（不删除，方便用户自行排查），新建库，重启写入线程。

- [ ] **Step 1: 写测试**
  - `corrupt_trajectory_db_does_not_block_startup`（Review Focus 4）：在临时目录写一个内容为 `not a database` 的 `trajectory.db`，调用 `Services::start_inner`。断言启动成功，`trajectory_status().available == false` 且 `error` 非空；通过代理发一个请求成功，`usage` 有 1 行；`rebuild_trajectory_db()` 之后 `available == true`，旧文件被重命名而不是删除。
  - `trajectory_toggle_takes_effect_without_restart`：关闭采集后发请求，没有新步骤；打开后发请求，有新步骤。
  - `disabled_client_is_not_captured`。
  - `backup_export_excludes_trajectory`（spec E2）：有会话数据时导出 `.yitoken`，解密后的载荷 JSON 中不含任何会话标题或步骤 ID。
  - `clear_all_during_traffic_never_fails_requests`：后台持续发请求时调用 `clear_sessions(None)`。断言所有请求都返回 200，并且 `usage` 行数等于请求数。
  - `src-tauri`：为 `list_sessions_impl` 和 `get_trajectory_impl` 各加一个调用测试，沿用现有 `*_impl` 测试的写法。
- [ ] **Step 2: 运行** → FAIL。
- [ ] **Step 3: 实现。**
- [ ] **Step 4: 运行** `cargo test --workspace` → PASS；`npm run gen:types`。
- [ ] **Step 5: 验证** `cargo clippy --workspace --all-targets -- -D warnings`。不提交。

### Task 12：用量服务扩展

**Files:**
- Modify: `core/src/services/dto.rs`（`UsageQueryInput`、`UsageReportView`）、`core/src/storage/usage.rs`、`core/src/services/usage.rs`
- Test: `core/tests/services_basic.rs` 或新建 `core/tests/usage_report.rs`

**Interfaces:**
- `UsageQueryInput` 新增 `client: Option<Client>`、`group_by: Option<UsageGroupBy>`（`Model | Client | Provider`，默认 `Model`）。
- `UsageReportView` 新增：
  - `totals: UsageTotals { requests, input_tokens, output_tokens, cached_tokens, reasoning_tokens, cache_hit_rate: Option<f64>, avg_ttft_ms: Option<f64>, avg_duration_ms: Option<f64>, timed_requests }`
  - `groups: Vec<UsageGroupRow { key: String, label: String, requests, input_tokens, output_tokens, cached_tokens, cache_hit_rate: Option<f64>, avg_duration_ms: Option<f64> }>`
- `cache_hit_rate = cached / input`（`input = 0` 时为 `None`）；`avg_*` 只统计非空行；`timed_requests` 是参与平均的行数，前端据此显示"自启用后统计"。
- 现有字段保持不变（`days`、`models`、`daily_models`、`clients`、`records`），前端在任务 18 中改造后再清理不用的字段。**本任务不删字段。**

- [ ] **Step 1: 写测试**：`client_filter_limits_rows`；`group_by_provider_aggregates`；`cache_hit_rate_none_when_no_input`；`averages_ignore_rows_without_timing`。
- [ ] **Step 2: 运行** → FAIL。
- [ ] **Step 3: 实现**（`FILTER` 常量扩展为 `... AND (?4 IS NULL OR client = ?4)`）。
- [ ] **Step 4: 运行** → PASS；`npm run gen:types`；`npm run typecheck`。
- [ ] **Step 5: 验证** clippy。不提交。

---

## 阶段 B：前端

> 所有前端任务都要：给 `tests/mock-backend.ts` 增加对应命令的模拟分支；中英文 i18n 同步增加文案；样式只用 token。每个任务的验证命令是 `npm run typecheck && npm run lint && npm run check:theme && npm run test:ui -- <本任务的 spec 文件>`。

### Task 13：导航、侧边栏分组与模拟后端夹具

**Files:**
- Modify: `src/shared/navigation/store.ts`、`src/shared/navigation/index.ts`、`src/app/routes.tsx`、`src/app/layout/Sidebar.tsx`
- Create: `src/features/sessions/{api.ts,hooks.ts,types.ts,index.ts,pages.ts}`、`src/features/sessions/components/SessionsPage.tsx`（先放空壳）
- Create: `src/shared/i18n/locales/zh-CN/session.ts`、`src/shared/i18n/locales/en-US/session.ts`，并在两种语言的 `index.ts` 注册
- Create: `tests/fixtures/trajectory.ts`
- Modify: `tests/mock-backend.ts`
- Test: `tests/workspace.spec.ts`（修改）

**Interfaces:**
- `Page` 增加 `"sessions"`。
- `NavigationState` 增加 `sessionTarget: SessionTarget | null`，其中 `interface SessionTarget { sessionId?: string; stepId?: string; client?: Client; model?: string }`。
- `openSessions(target?: SessionTarget)`：设置 `sessionTarget` 后跳到 `"sessions"`。会话页读取一次后调用 `consumeSessionTarget()` 清空。
- `sessionsApi`（仿照 `features/proxy/api.ts`，在 `CommandMap` 里声明）：`list`、`get`、`trajectory`、`stepContext`、`blob`、`changes`、`remove`、`clear`、`status`、`saveSettings`、`rebuild`、`live`。
- Hooks：`useSessionList(filter)` 用 `useInfiniteQuery` 加游标；`useSessionDetail(id)`；`useTrajectory(sessionId, threadId)` 用 `useInfiniteQuery` 按回合向前翻页；`useSessionChanges(active)` 每 2 秒用 `since_seq` 轮询，有变化时让对应查询失效；`useStepContext(stepId)`；`useBlob(hash)`；`useTrajectoryStatus()`。
- 侧边栏：`NAV_GROUPS` 改为 `[["provider-list", "model-list"], ["proxy", "sessions", "usage"]]`。第二组标题从 `nav.tools` 改为 `nav.observe`（"观测" / "Observe"）。`proxy` 的显示名改为 `nav.run`（"运行" / "Run"），页面 ID 仍是 `"proxy"`，避免改动 `proxyVisited` 等逻辑。终端折叠组从"观测"之后移到"配置"组内，排在最后。`sessions` 图标用 lucide `History`；在途请求大于 0 时显示一个计数徽标，数据来自 `useLiveRequests` 的轻量查询（任务 14 提供，本任务先放 0）。
- 夹具 `tests/fixtures/trajectory.ts`：至少 3 个会话（codex、claude-code 各一个，外加一个 `key_source = 'chain'` 的），其中一个包含 3 个回合、8 个步骤、1 个子 Agent 线程、1 次 system 变化、1 个失败步骤、1 个在途占位步骤，并且是长会话（第一页 `has_older = true`）。

- [ ] **Step 1: 改测试** `tests/workspace.spec.ts`：遍历页面的列表里，"代理运行"改为"运行"，新增"会话"（ready 选择器 `.sessions-page`）。新增断言：侧边栏有"配置""观测"两个分组标题。
- [ ] **Step 2: 运行** `npm run test:ui -- workspace.spec.ts` → FAIL。
- [ ] **Step 3: 实现。**
- [ ] **Step 4: 运行** → PASS。
- [ ] **Step 5: 验证**（见阶段 B 说明）。不提交。

### Task 14：运行页

**Files:**
- Create: `src/features/proxy/components/RunPage.tsx`、`LiveRequests.tsx`、`LogDrawer.tsx`、`TrafficStrip.tsx`
- Modify: `src/features/proxy/pages.ts`（导出 `ProxyPage` 指向 `RunPage`，保持 `routes.tsx` 中的 `module.ProxyPage` 不变）、`src/features/proxy/api.ts`、`hooks.ts`、`types.ts`
- Delete: `ProxyConnections.tsx`、`MonitorEntries.tsx`、`ProxyPage.tsx`（确认没有其他引用之后再删）
- Modify: `src/styles/proxy.css`、i18n `proxy.ts`
- Create: `tests/run-page.spec.ts`

**行为（spec 8.2）：**
- **状态条**：状态点、状态文字、监听地址、已运行时长、连接数（TCP）和在途请求数、重启和启停按钮。启动失败时显示错误，以及"去设置"按钮，调用 `navigate("settings")` 并滚动到监听分组。
- **流量**：`TrafficStrip` 用 recharts 画请求/秒和错误/秒两条迷你折线，高度 64px，可以收起，收起状态存在 `localStorage` 的 `yi-token:run-traffic`。数据来自现有的 `useProxyMonitor`。
- **请求流**：
  - 终端筛选片：全部 + 各终端，每片显示请求数和错误数，来自 monitor 的 `routes` 按 client 聚合，即原 `MonitorEntries` 里的 `group` 函数，移到 `format.ts`；"仅错误"开关。
  - 列表：在途请求置顶，然后是已完成请求按时间倒序。列：时间、终端（图标 + 名称）、模型、阶段/状态（`Waiting` 显示"等待首 token"加计时、`Streaming` 显示动画、完成显示状态码）、TTFT、耗时、输入/输出 token、"→ 会话"按钮（有 `session_id` 时可用，调用 `openSessions({ sessionId, stepId })`）。
  - 数据：`useLiveRequests(active)` 每 1 秒用 `since_seq` 轮询；前端维护最近 500 条，与后端一致。
- **日志**：`LogDrawer` 是底部抽屉，默认收起。收起时标题栏显示"日志"和 WARN/ERROR 计数；展开后复用 `ProxyLogs` 组件（`visible` 跟随抽屉状态）。
- 删除"连接"和"条目"两个 tab，以及监听设置弹窗。

- [ ] **Step 1: 写测试** `tests/run-page.spec.ts`：
  - `status strip shows listen address and counts`
  - `live list puts in-flight first and links to session`：点击"→ 会话"后，`.sessions-page` 可见，并且选中了对应步骤（检查器标题含"请求 #"）。
  - `client chips filter the list`、`errors-only toggle`
  - `log drawer is collapsed by default and shows issue count`
  - `start failure offers settings link`
- [ ] **Step 2: 运行** → FAIL。
- [ ] **Step 3: 实现**；mock 增加 `get_live_requests`，第二次调用返回新增的已完成请求，用来验证增量合并。
- [ ] **Step 4: 运行** → PASS；运行 `tests/workspace.spec.ts` 无回归。
- [ ] **Step 5: 验证**。不提交。

### Task 15：会话页布局、会话列表与会话头

**Files:**
- Modify: `src/features/sessions/components/SessionsPage.tsx`
- Create: `SessionList.tsx`、`SessionHeader.tsx`、`TrajectoryNotice.tsx`、`src/styles/sessions.css`
- Create: `tests/sessions.spec.ts`

**行为（spec 8.3）：**
- 双栏：左栏 300px，可拖动调宽，复用 `SidebarResizer` 的交互模式，宽度存 `localStorage` 的 `yi-token:sessions-list-width`。窄于 760px 时只显示一栏，选中会话后显示轨迹，并提供返回按钮。
- **左栏**：终端下拉（全部 + 5 个终端 + `api`）、标题搜索（输入停止 300ms 后生效）。列表按"今天 / 昨天 / 本周 / 更早"分组；每项显示终端图标、标题（为空时显示"未命名会话"）、步数、token、相对时间、`live` 动画点、`key_source = chain` 的 `≈` 标记（悬停提示"根据对话历史推断"）、`new` 的 `?` 标记。滚动到底部时加载下一页（`useInfiniteQuery`）。
- 每项右键菜单或悬停按钮："删除会话"，二次确认，复用 `ConfirmDialog`。
- **会话头**：标题、终端、模型列表、步数、总 token、总时长（`last_at - started_at`）；线程切换片（主线程 / 子 Agent ×N / 旁路 ×N / 其他 ×N），点击切换 `threadId`；有 `reset` 线程时显示"上下文已重置"提示。
- 首次进入且 `notice_seen = false` 时，显示 `TrajectoryNotice` 横幅，文案见 spec 8.3。点"知道了"时调用 `saveSettings({...settings, notice_seen: true})`。
- 轨迹库不可用（`status.available = false`）时，整个右栏显示错误说明和"前往设置"按钮。
- 接收 `sessionTarget`：有 `sessionId` 就选中它；有 `client` 或 `model` 就预设筛选。

- [ ] **Step 1: 写测试** `tests/sessions.spec.ts`：
  - `list groups by day and marks inferred sessions`
  - `client filter and title search narrow the list`
  - `infinite scroll loads next page`
  - `thread chips switch the trajectory`
  - `delete session requires confirmation`
  - `first visit shows privacy notice once`（关闭后刷新页面不再出现）
  - `unavailable trajectory store shows settings link`
- [ ] **Step 2: 运行** → FAIL。
- [ ] **Step 3: 实现。** 右栏先放一个 `<TrajectoryView>` 占位，任务 16 再实现。
- [ ] **Step 4: 运行** → PASS。
- [ ] **Step 5: 验证**。不提交。

### Task 16：轨迹账本（移植 deepseek-harness 的 TrajectoryTable）

**Files:**
- Create: `src/features/sessions/trajectory/records.ts`、`virtualRows.ts`、`TrajectoryLedger.tsx`、`TrajectoryToolbar.tsx`、`TrajectoryView.tsx`、`src/styles/trajectory.css`
- Modify: `tests/sessions.spec.ts`

**参考：** deepseek-harness 的 `trajectory-record.ts`、`trajectory-virtual-rows.ts`、`TrajectoryTable.tsx`（第 490–760 行和第 2690–2930 行）、`TrajectoryToolbar.tsx`。

**Interfaces（`records.ts`，纯函数）：**

```ts
export interface LedgerRecord {
  record: TrajectoryRecord;          // 来自 src/generated
  step: TrajectoryStep;
  turn: number;
  threadId: string;
  index: number;                     // 1 起，全页唯一
  stepStart: boolean;                // 该步骤第一条记录：显示请求边界标记
  turnStart: boolean; turnEnd: boolean;
  collapsedSummary?: string;
  collapsedSummaryKind?: "turn" | "assistant";
}
export type RecordState = "running" | "error" | "complete";
export function flattenTurns(turns: readonly TrajectoryTurn[]): LedgerRecord[];
export function collapseTurns(records: readonly LedgerRecord[], collapsed: ReadonlySet<number>, t: Translate): LedgerRecord[];
export function collapseAssistants(records: readonly LedgerRecord[], collapsed: ReadonlySet<string>, t: Translate): LedgerRecord[];
export function filterRecords(records: readonly LedgerRecord[], query: string): LedgerRecord[];
export function stateOf(r: LedgerRecord): RecordState;   // tool 没有 result 且步骤在途 → running；is_error 或 stop_reason ∈ {error, cancelled} → error
```

折叠算法与 `collapseTurnRecords` / `collapseAssistantRecords` 一致：回合折叠保留首条记录，加一条摘要（"N 步 · M 次工具调用"）；助手折叠把它后面连续的 tool / subtool 合成一条（"M 次工具调用 · 工具名"）。

`virtualRows.ts` 照搬 `groupTrajectoryVirtualRows`。本项目没有 `requestOnly` 记录，但保留这段逻辑，用来承载没有记录的在途占位步骤。行高 30 / 20 / 9。

**账本渲染（`TrajectoryLedger.tsx`）：**
- `<table role="grid">`，两列：类型标签列（宽 96px）、内容列。
- 类型标签：`System` 中性、`User` 默认、`Message` 紫、`Tool` 琥珀、`Subtool` 浅琥珀、`Compacted` 灰。颜色在 `tokens.css` 新增 `--trajectory-kind-*` 的明暗两套。
- **请求边界**：`stepStart` 行在左侧画一个圆点按钮（`aria-label="请求 #N"`，`data-request-status` 取 `running | error | complete`），点击后检查器显示该请求，见任务 18。
- **回合标签**：`turnStart` 行显示"轮次 N"，列宽小于 120px 时显示 `#N`；当前选中的回合左侧画竖条。
- **内容列**：`Tool` / `Subtool` 显示 `tool_name · 参数摘要 → 结果摘要`，结果为空时显示"无输出"；`Message` 显示文本摘要，有 `thinking` 时前面加一个推理图标；`System` 显示"系统提示词"或"系统提示词已变更"。
- **交互**（与参考实现一致）：单击选中；双击回合首行折叠该回合；双击带工具调用的助手行折叠它的调用；Enter / 空格等同于单击；摘要行单击就展开。
- `Subtool` 行的内容列末尾有"展开子 Agent"按钮，切换到对应线程，与会话头的线程切换联动。
- **虚拟化**：`useVirtualizer`，`estimateSize` 取行高常量，`getItemKey` 取 `trajectoryVirtualRecordKey`，overscan 8。
- **跟随末尾**：距离底部小于 40px 时视为在末尾；新数据到达且在末尾时滚到底部。向前加载更早一页后，按新增内容的高度补偿 `scrollTop`，保持视口不跳。
- **"加载更早的回合"**：放在第一行，调用 `fetchNextPage`，加载中显示 spinner，`aria-live="polite"`。

**工具栏（`TrajectoryToolbar.tsx`）**：时长投影（等宽 / 实际时长）、"实际时间"开关、"轮次"全部折叠或展开、"调用"全部折叠或展开、搜索框。选择状态存 `localStorage` 的 `yi-token:trajectory-toolbar`。

- [ ] **Step 1: 写测试**（追加到 `tests/sessions.spec.ts`）：
  - `ledger shows kind tags, turn labels and request markers`：夹具会话应显示 3 个"轮次"标签、8 个请求标记，其中 1 个 `data-request-status="error"`、1 个 `running`。
  - `tool call and result render on one row`
  - `double-click turn header folds and unfolds the turn`：折叠后摘要行文字匹配 `/\d+ 步 · \d+ 次工具调用/`。
  - `toolbar collapses all calls`
  - `search filters loaded records`
  - `load older keeps the viewport anchored`：记录点击前第一条可见记录的 `data-trajectory-row-key`，加载后它仍在视口内。
  - `follow tail pauses when scrolled up`：mock 的 `get_session_changes` 返回变化后，向上滚动时 `scrollTop` 不变，滚到底部后新行出现在视口里。
  - `subtool expands to the subagent thread`
- [ ] **Step 2: 运行** → FAIL。
- [ ] **Step 3: 实现。**
- [ ] **Step 4: 运行** → PASS。
- [ ] **Step 5: 验证**。不提交。

### Task 17：时间线总览（移植 TrajectoryTimeline）

**Files:**
- Create: `src/features/sessions/trajectory/timeline.ts`、`TrajectoryTimeline.tsx`
- Modify: `TrajectoryView.tsx`、`trajectory.css`、`tests/sessions.spec.ts`

**参考：** deepseek-harness 的 `timeline.ts`（整份）、`TrajectoryTimeline.tsx`。

**Interfaces（`timeline.ts`）：**

```ts
export type TimelineMode = "sequence" | "duration" | "time" | "actual";  // 与参考实现一致
export interface TimeRange { start: number; end: number }
export interface TimelineSpan extends TimeRange { index: number; isError: boolean; kind: RecordKind; label: string; lane: 0 | 1 | 2;
  ttftEnd?: number }                 // 仅 Message：TTFT 段与生成段的分界点
export interface TimelineModel extends TimeRange { spans: TimelineSpan[]; turnBoundaries: { turn: number; time: number }[] }
export function deriveTimeline(records: readonly LedgerRecord[], mode: TimelineMode): TimelineModel | null;
export function focusIndexes(records: readonly LedgerRecord[], range: TimeRange, mode: TimelineMode): ReadonlySet<number>;
```

- 泳道：`System` / `User` → 0；`Message` / `Compacted` → 1；`Tool` / `Subtool` → 2。
- `Message` 的 `ttftEnd = step.first_token_at`，`time` / `actual` 模式下需要减去压缩掉的空闲时长。
- 工具栏组合到模式的映射与参考实现一致：等宽 + 不看实际时间 → `sequence`；实际时长 + 不看实际时间 → `duration`；等宽 + 看实际时间 → `time`；实际时长 + 看实际时间 → `actual`。

**组件**：高度 72px 的 SVG，三条泳道，回合边界画竖线。助手条的 TTFT 段用较浅的颜色。
- 左键拖选 → `onSelect(range)`，账本中不在区间内的行加 `data-timeline-focus="outside"`，用 CSS 降低透明度。
- 滚轮 → 以指针位置为中心缩放；右键单击 → 清除选择；右键拖动 → 已缩放时平移。
- 悬停 500ms → 提示框显示开始时间、时长，助手条另显示"首 token X · 生成 Y"。
- 单击某条 → 账本选中对应记录并滚动到可见。
- 没有任何计时数据时显示"无计时数据"。

- [ ] **Step 1: 写测试**（Playwright）：
  - `timeline renders three lanes and turn boundaries`
  - `drag selection focuses ledger rows`
  - `right click clears selection`
  - `mode switch changes span widths`：等宽模式下各条宽度相等，切到实际时长后宽度不全相等。
  - `assistant span shows ttft segment`
  - `clicking a span selects the ledger record`
- [ ] **Step 2: 运行** → FAIL。
- [ ] **Step 3: 实现。**
- [ ] **Step 4: 运行** → PASS。
- [ ] **Step 5: 验证**。不提交。

### Task 18：检查器

**Files:**
- Modify: `package.json`（新增 `react-markdown`、`remark-gfm`），执行 `npm install`
- Create: `src/features/sessions/trajectory/TrajectoryInspector.tsx`、`JsonTree.tsx`、`MarkdownBlock.tsx`
- Modify: `TrajectoryView.tsx`、`trajectory.css`、`tests/sessions.spec.ts`

**参考：** deepseek-harness 的 `TrajectoryTable.tsx` 中 `detailTabs`（第 976–1013 行）、`RequestUsagePanel`、`RequestTiming`、`AssistantTimingPanel`（第 342–390 行），以及检查器抽屉与拖动调宽（第 2950–3120 行）。

**页签（spec 8.3.1 表格）：**

| 选中对象 | 页签 |
| --- | --- |
| `System` | 系统提示词 · 工具 · 差异（仅 `system_changed`） |
| `User` / `Message` | 概述 · 预览 · 原始内容 |
| `Tool` / `Subtool` | 概述 · 参数 · 结果 · Schema · 计时 |
| 请求标记 | 用量 · 选项 · 计时 · 完整上下文 |

- 抽屉在右侧，默认 420px，可拖动，范围 320px 到内容区宽度的 70%，宽度存 `localStorage`。Esc 关闭。
- **概述**：类型、所属请求 #N、模型、开始时间、时长；截断时显示"已截断，原文 X KiB"和"加载全文"按钮（`useBlob(body_blob)`）。
- **预览**：`MarkdownBlock`，`react-markdown` + `remark-gfm`。禁止原始 HTML（不使用 `rehype-raw`），链接用 `target="_blank" rel="noreferrer"`。字号 13px，行高 20px，标题只加粗、不放大，与参考实现一致。
- **原始内容 / 参数 / 结果**：能解析成 JSON 对象或数组时用 `JsonTree`（可折叠，默认展开两层），否则等宽 `pre-wrap` 显示。右上角有复制按钮。
- **Schema**：从 `get_step_context(step_id).tools` 中找到 `name == tool_name` 的那一项。
- **差异**：对比本步和上一步的系统提示词与工具定义（`get_step_context` 分别取两步），按行做 LCS 差异，与参考实现的 `promptDiffLines` 一致。
- **计时**：开始时间、TTFT、生成时长、总时长、输出速率（`output_tokens / 生成时长`）；工具标注"估算：客户端执行工具到发起下一次请求的间隔"。
- **用量**：输入（其中缓存）、输出（其中推理）、合计。
- **选项**：模型、上游模型、服务商、入站协议 → 上游协议、路由模式、`options_json` 的键值。
- **完整上下文**：调用 `get_step_context`，显示系统提示词（折叠）、工具定义（折叠）和输入条目列表（每条可展开为 JSON 树）。顶部显示"共 N 条，回溯 D 层"。

- [ ] **Step 1: 写测试**：
  - `inspector tabs depend on record kind`：分别选中 System、Message、Tool 和请求标记，断言页签名称集合。
  - `preview renders markdown without raw html`：夹具里的文本含 `<script>`，断言 DOM 中没有 `script` 元素，并且文本可见。
  - `truncated body loads full text on demand`
  - `system diff shows added tool`
  - `full context lists reconstructed items`
  - `inspector width persists`
- [ ] **Step 2: 运行** → FAIL。
- [ ] **Step 3: 实现。**
- [ ] **Step 4: 运行** → PASS；`npm run build` 通过（确认新增依赖已打进包）。
- [ ] **Step 5: 验证**。不提交。

### Task 19：用量页重构

**Files:**
- Modify: `src/features/usage/components/UsagePage.tsx`、`UsageOverview.tsx`、`UsageSummary.tsx`、`UsageModels.tsx`（重命名为 `UsageBreakdown.tsx`）、`src/features/usage/api.ts`、`hooks.ts`、`types.ts`、`src/styles/usage.css`、i18n `usage.ts`
- Delete: `UsageRecords.tsx`、`UsageRecordDetail.tsx`（确认没有其他引用之后再删）
- Create: `tests/usage.spec.ts`

**行为（spec 8.4）：**
- 一条筛选栏：日期（复用 `UsageDateFilter`）、终端下拉、模型下拉、刷新按钮。删除三个 tab。
- **汇总卡**：请求数、输入 token、输出 token、缓存命中率、平均 TTFT。平均 TTFT 在 `timed_requests < requests` 时加小字"基于 X 次请求，自启用后统计"。
- **趋势图**：复用 `UsageTrend`，保留 token / 请求切换。
- **明细表**：分组切换片（模型 / 终端 / 服务商）；列：名称（带图标）、请求、输入、输出、缓存率、平均耗时、"→ 会话"按钮（调用 `openSessions({ client, model })`）。
- 筛选条件存 `localStorage` 的 `yi-token:usage-filter`，日期预设也一起保存。
- 清理：前端不再使用的 `UsageReportView` 字段（如 `records`），在后端保留一个版本，等本任务完成后，在任务 21 中统一删除。

- [ ] **Step 1: 写测试** `tests/usage.spec.ts`：
  - `single filter bar without tabs`：页面中不存在 `role="tablist"`。
  - `kpis include cache hit rate and ttft note`
  - `group switch changes breakdown rows`
  - `breakdown row links to filtered sessions`：点击后会话页的终端筛选被预设。
- [ ] **Step 2: 运行** → FAIL。
- [ ] **Step 3: 实现。**
- [ ] **Step 4: 运行** → PASS。
- [ ] **Step 5: 验证**。不提交。

### Task 20：设置页（监听与轨迹）与终端页入口

**Files:**
- Create: `src/features/settings/components/ListenSettings.tsx`（从 `ProxyListenSettings.tsx` 迁移逻辑后删除原文件）、`TrajectorySettings.tsx`
- Modify: `SettingsPage.tsx`、`src/styles/settings.css`、i18n `settings.ts`
- Modify: `src/features/terminal/components/TerminalPage.tsx`（右上角"查看会话"）
- Modify: `tests/workspace.spec.ts` 或新建 `tests/settings.spec.ts`

**行为（spec 8.5）：**
- **代理监听**：地址、端口、日志级别、保存；保存后代理重启的提示沿用原逻辑。分组锚点 `id="settings-listen"`，供运行页"去设置"跳转。
- **轨迹**：
  - 总开关；各终端开关（5 个终端 + `api`）。
  - 保留天数（数字输入，1–3650）；容量上限（下拉：512 MiB / 1 GiB / 2 GiB / 5 GiB / 10 GiB / 自定义）。
  - 当前占用：`bytes` 和会话数。
  - 说明文字："开启后，yi-token 会把经过代理的对话正文（提示词、模型输出、工具调用与结果）保存在本机 `trajectory.db`，不保存请求头和密钥。"
  - "清空某终端"下拉 + 按钮、"清空全部"按钮，都要二次确认，确认文案写明不可恢复。
  - 轨迹库不可用时显示错误原因和"重建轨迹库"按钮；按钮说明"旧文件会被重命名保留"。
- 终端页右上角的"查看会话"调用 `openSessions({ client })`。

- [ ] **Step 1: 写测试**：
  - `listen settings moved to settings page`
  - `trajectory toggles save`
  - `clear all requires confirmation and shows result`
  - `rebuild shown when store unavailable`
  - `terminal page links to its sessions`
- [ ] **Step 2: 运行** → FAIL。
- [ ] **Step 3: 实现。**
- [ ] **Step 4: 运行** → PASS。
- [ ] **Step 5: 验证**。不提交。

### Task 21：清理、视觉基线与全量验证

**Files:**
- Modify: `core/src/services/dto.rs`、`core/src/storage/usage.rs`（删除前端已不使用的 `UsageReportView.records` 等字段，并删除对应查询）
- Modify: `tests/zz-visual.spec.ts`（新增 `sessions`、`usage` 页面截图；`proxy` 截图改为运行页）
- Update: `tests/zz-visual.spec.ts-snapshots/` 下的 `proxy-*`、`settings-*`，新增 `sessions-*`、`usage-*`（明暗各一套）
- Modify: `README.md`（功能列表增加"会话轨迹"，以及隐私说明一段）
- Modify: `docs/spec-coverage.md`（如果该文件按 spec 条目登记覆盖情况，就补上本 spec 的条目）

- [ ] **Step 1:** 用 `rg "records\b" src/features/usage` 等方式确认前端不再使用要删除的字段，然后删除后端字段，运行 `npm run gen:types`、`npm run typecheck`。
- [ ] **Step 2:** `npx playwright test zz-visual.spec.ts --update-snapshots`。**逐张打开新截图人工确认**，不能只看命令通过，然后再运行一次不带 `--update-snapshots` 的测试确认稳定。
- [ ] **Step 3: 全量验证：**`cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace`、`npm run typecheck`、`npm run lint`、`npm run check:theme`、`npm run build`、`npm run test:ui`。全部通过。再执行性能基线 `cargo test -p yi-core --release --test proxy_pipeline capture_overhead -- --ignored`，把 p50 / p95 写进附录 B。
- [ ] **Step 4: 真机冒烟：**`npm run tauri dev`，用 codex 和 claude-code 各跑一个带工具调用的任务，确认：
  - 运行页能实时看到请求；
  - 会话页能看到对应会话，回合和工具配对正确；
  - 用量页的数字与运行页一致；
  - 关闭采集后不再产生新步骤。

  把结果记录到附录 B。
- [ ] **Step 5:** 不提交。向用户报告变更的文件清单和验证结果。

---

## 附录 A：会话标识抓包结论（任务 1 填写）

| 客户端 | 会话标识位置与格式 | 回合标识 | 前缀链能否串起全部步骤 | 备注 |
| --- | --- | --- | --- | --- |
| codex | | | | |
| claude-code | | | | |
| opencode | | | | |
| pi | | | | |
| deepseek-harness | | | | |

## 附录 B：真机冒烟记录（任务 21 填写）

| 检查项 | codex | claude-code |
| --- | --- | --- |
| 运行页实时显示 | | |
| 会话归并正确 | | |
| 回合与工具配对 | | |
| 子 Agent 线程 | | |
| 用量一致 | | |
| 关闭采集生效 | | |
