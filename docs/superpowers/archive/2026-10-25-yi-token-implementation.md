> 历史实施计划（已完成，仅作记录）。复选框保留原始状态，不代表未完成；当前行为以 `docs/spec-coverage.md` 与代码为准。

# yi-token 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 从零实现 yi-token：功能对齐 yi-llm v0.1.8 的本地多协议 LLM 网关与终端管理桌面应用，并新增 Auto 路由、生效的 `thinking` 兜底和扩展备份。

**Architecture:** Cargo workspace 分为纯 Rust 的 `core`（domain / storage / catalog / protocol / proxy / terminal / backup / services）与 Tauri 壳 `src-tauri`（只依赖 `core::services`）。代理基于 axum，同协议直通，跨协议经 Responses 中间形态转换。前端从 yi-llm 原样拷入，只改数据接入层。

**Tech Stack:** Rust 2024 edition、Tauri 3.0.0-alpha、axum 0.8、reqwest 0.12（rustls）、tokio、rusqlite 0.40（bundled）+ r2d2 + rusqlite_migration + serde_rusqlite、arc-swap、eventsource-stream、toml_edit、serde_json / json5 / serde_norway、aes-gcm + pbkdf2 + zeroize、tracing + tracing-appender、ts-rs；测试用 wiremock、insta、tempfile。前端 React 19、Vite 8、TanStack Query、Playwright。

**Spec:** `docs/superpowers/specs/2026-10-25-yi-token-design.md`（执行者必须同时阅读）

> **关于本计划的写法：** 按用户要求，本计划不附完整代码。每个任务给出文件、接口签名、行为要求、测试用例（名称与断言要点）、验证命令和提交信息。行为细节以 spec 为准，spec 未写明的细节以 yi-llm v0.1.8 的对应文件为准（见每个任务的"参考"）。

## 如何阅读 yi-llm 参考代码

yi-llm 当前检出的是 `refactor/backend-rust` 分支，**不要切换或修改它**。读取 v0.1.8 用：

```bash
git -C D:/Github/yi-llm show main:src-tauri/src/<path>
git -C D:/Github/yi-llm ls-tree -r --name-only main -- <dir>
```

读取重构分支（oracle 用例、缺陷修复）用 `refactor/backend-rust:<path>`。只参考行为，不复制后端代码（spec D1）。前端代码、`models/catalog.json`、图标资源、Playwright 测试可以直接拷贝（spec D3）。

## Global Constraints

- 数据目录 `$YI_TOKEN_HOME`，默认 `~/.yi-token`；数据库 `yi-token.db`；日志 `logs/yi-token.log`，按天滚动；模型目录覆盖变量 `$YI_TOKEN_MODEL_CATALOG`。
- Tauri identifier `com.yi-token.app`，productName `yi-token`，与 yi-llm 不同。
- 默认监听 `127.0.0.1:11435`，默认日志级别 `info`。
- 所有 ID 为 TEXT（UUID v4，后端生成）；时间为 UTC 毫秒整数。
- 终端所有权标识：proxy provider id `yitoken`；direct provider id `yitoken_direct_<id>`；direct 名称前缀 `yi-token direct:`；deepseek 环境变量 `YI_TOKEN_PROXY`、`YI_TOKEN_DIRECT_*`；proxy 模式 API Key 固定为 `yi-token`；Codex 模型元数据文件 `yi-token-models.json`。
- 只修改和删除带 yi-token 标识的终端条目；用户手写内容和 yi-llm 条目必须原样保留。
- 备份：扩展名 `.yitoken`，`format = "yi-token.backup"`，`version = 1`，PBKDF2-HMAC-SHA256 默认 600,000 次，接受 10,000–10,000,000，salt 16 字节，nonce 12 字节，AES-256-GCM，信封头为 AAD，口令 ≥ 8 字符。
- 错误结构 `{ code, message }`，code ∈ `validation | not_found | conflict | database | io | upstream | internal`，消息为中文。
- HTTP 映射：validation→400，not_found→404，conflict→409，upstream→原样状态码与响应体，其余→500。
- 推理路由不设请求体上限。
- 沿用 v0.1.8 的 23 个 Tauri 命令（名称与参数不变），新增 `set_active_engine`、`get_active_engines`。
- `thinking` 映射：Anthropic `budget_tokens` low 4096 / medium 10000 / high 32000 / xhigh 64000（低于 `max_tokens`，`minimal` 不开启）；Chat `reasoning_effort`；Responses `reasoning.effort`；请求已带推理参数时不覆盖。
- 不兼容 yi-llm 数据；不做故障自动切换；不改前端视觉样式。
- `unsafe_code = "forbid"`；验收：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、`npm run typecheck`、`npm run lint`。

## Review Focus

1. **超大请求体**：Codex 发送 >2 MiB（例如 8 MiB）的 `/clients/codex/v1/responses` 请求，应正常转发而不是 413。测试加在 Task 16。
2. **流中途客户端断开**：下游在 SSE 中途断开，上游请求应被取消，用量最多记录一次，监控计数不卡在"进行中"。测试加在 Task 17。
3. **端口被占用**：启动代理时端口已被占用，应返回中文 `io` 错误，`get_proxy_status` 为 false，应用其余功能可用，修改端口后可以成功启动。测试加在 Task 15。
4. **终端配置文件损坏**：目标 JSON/TOML/YAML 无法解析时，应返回校验错误并且不覆盖原文件（不生成"修复后"的空配置）；文件不存在时应新建。测试加在 Task 19。
5. **删除正在被引用的服务商**：删除某服务商后，以它的模型为 Auto 引擎的终端回到"未选择"并返回 409；以它为 direct 配置的终端条目随之删除；fixed 配置中引用它的模型在下次解析时返回 404 而不是 panic。测试加在 Task 22。

---

## 文件结构总览

```
yi-token/
  Cargo.toml                         # workspace；[workspace.lints] unsafe_code = "forbid"
  rustfmt.toml
  package.json, vite.config.ts, tsconfig*.json, eslint.config.js, playwright.config.mjs, index.html
  models/catalog.json, catalog.schema.json, README.md     # 从 yi-llm 拷贝
  core/
    Cargo.toml                       # crate 名 yi-core
    src/lib.rs
    src/error.rs                     # CoreError、ErrorCode
    src/paths.rs                     # 数据目录、数据库、日志、目录文件路径解析
    src/domain/{mod,protocol,client,provider,model,standard_model,capabilities,thinking,terminal,settings,usage,names}.rs
    src/storage/{mod,pool,migrations,providers,standard_models,models,terminal,engines,settings,usage,usage_writer,route_source}.rs
    src/catalog/{mod,file,sync}.rs
    src/protocol/
      mod.rs
      ir.rs                          # Responses 中间形态（请求、输出项、流事件）
      anthropic/{mod,types,inbound,outbound,stream}.rs
      chat/{mod,types,inbound,outbound,stream}.rs
      responses/{mod,types,stream}.rs
      prepare.rs                     # 模型名替换、store/previous_response_id 校验、thinking 兜底
      thinking.rs
      compatibility.rs               # non_standard 兼容规则
      sse.rs                         # 字节缓冲 SSE 分帧与编码
      usage.rs                       # 从响应/流中提取用量
    src/proxy/
      mod.rs
      server.rs                      # ProxyServer、ProxyHandle、生命周期
      router.rs                      # 路由表
      resolve.rs                     # 全局 / fixed / auto 解析
      pipeline.rs                    # 一次请求的完整处理
      upstream.rs                    # reqwest 客户端、头处理、错误
      headers.rs
      errors.rs                      # CoreError → HTTP 响应
      monitor.rs
      request_log.rs                 # 中间件
    src/terminal/
      mod.rs
      doc.rs                         # Doc 抽象（Toml / Json / Yaml）
      spec.rs                        # ClientSpec、ClientCtx
      paths.rs                       # 各客户端配置文件定位
      clients/{mod,codex,claude_code,opencode,pi,deepseek_harness}.rs
      plan.rs                        # TerminalPlan、FileChange、预览脱敏
      commit.rs                      # 备份、原子写入、回滚
    src/backup/{mod,crypto,payload,import}.rs
    src/logging.rs                   # tracing 初始化、运行时调级、读取日志尾部
    src/services/{mod,providers,models,settings,proxy,usage,logs,terminal,engines,backup,dto}.rs
    tests/                           # 集成测试（wiremock 等）
      common/mod.rs
      storage_*.rs, proxy_*.rs, terminal_*.rs, backup_*.rs, oracle.rs
  src-tauri/
    Cargo.toml, build.rs, tauri.conf.json, capabilities/default.json, icons/
    src/{main,lib,commands,state}.rs
  src/                               # 前端（拷自 yi-llm main:src）
    generated/                       # ts-rs 输出
  tests/                             # Playwright（拷自 yi-llm main:tests）
```

## 阶段与任务一览

| 阶段 | 任务 | 交付 |
| --- | --- | --- |
| 0 骨架 | 1 | workspace、前端拷入、空壳应用可启动 |
| 1 领域与存储 | 2–6 | domain 类型、SQLite 迁移与 repo、模型目录、用量写入器 |
| 2 门面与前端接入 | 7–9 | services、Tauri 命令、ts-rs 与前端数据层；服务商/模型/设置可用 |
| 3 协议层 | 10–14 | 中间形态、三协议进出、SSE、prepare / thinking / 兼容、oracle 比对 |
| 4 代理层 | 15–18 | 生命周期、路由与解析、管线与直通、监控日志用量、代理命令 |
| 5 终端配置 | 19–21 | Doc 与提交、五个客户端、proxy/direct 预览与应用 |
| 6 Auto | 22–23 | 引擎存储与解析、命令与前端入口 |
| 7 备份 | 24 | `.yitoken` 导出导入 |
| 8 验收 | 25–26 | 界面一致性、全量验收与发布配置 |

工作分支：在 `main` 上创建 `feat/implementation`（或按 superpowers:using-git-worktrees 建 worktree）。每个任务结束都要满足：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 通过；涉及前端的任务另需 `npm run typecheck`、`npm run lint` 通过。

---

## 阶段 0：骨架

### Task 1: Workspace 与应用空壳

**Files:**
- Create: `Cargo.toml`、`rustfmt.toml`、`.gitignore`、`core/Cargo.toml`、`core/src/lib.rs`、`src-tauri/{Cargo.toml,build.rs,tauri.conf.json,capabilities/default.json}`、`src-tauri/src/{main,lib}.rs`、`src-tauri/icons/*`
- Copy from yi-llm main: `src/**`、`tests/**`、`models/**`、`scripts/check-theme-tokens.mjs`、`index.html`、`package.json`、`vite.config.ts`、`tsconfig.json`、`tsconfig.node.json`、`eslint.config.js`、`playwright.config.mjs`、`public/**`、`LICENSE`
- Modify（拷入后）：`package.json`（name `yi-token`，version `0.1.0`，description）、`tauri.conf.json`

**Interfaces:**
- Produces: crate `yi-core`（`core/`），crate `yi-token`（`src-tauri/`，lib 名 `yi_token_lib`）；workspace 级依赖版本表 `[workspace.dependencies]`，后续任务只在成员 crate 中写 `xxx.workspace = true`。

- [ ] **Step 1:** 用 `git -C D:/Github/yi-llm archive main <paths> | tar -x -C D:/Github/yi-token` 拷入上面列出的前端与资源文件；不要拷贝 `src-tauri/src`、`dist`、`node_modules`、`target`。
- [ ] **Step 2:** 编写 workspace `Cargo.toml`：`members = ["core", "src-tauri"]`，`resolver = "3"`，`[workspace.package] edition = "2024"`，`[workspace.lints.rust] unsafe_code = "forbid"`，`[workspace.lints.clippy] all = "warn"`；在 `[workspace.dependencies]` 中写入 Tech Stack 列出的依赖，版本取 yi-llm `main:src-tauri/Cargo.toml` 中的同名依赖版本。
- [ ] **Step 3:** `tauri.conf.json` 以 yi-llm 的为模板：`productName = "yi-token"`，`identifier = "com.yi-token.app"`，`version = "0.1.0"`，窗口参数（1120×760、最小 960×640、`decorations: false`、`shadow: true`）保持不变；`frontendDist = "../dist"`。
- [ ] **Step 4:** `src-tauri/src/lib.rs` 只建一个不注册任何命令的 `tauri::Builder`；`main.rs` 调用 `yi_token_lib::run()`。`core/src/lib.rs` 暂时只有模块声明占位与一个 `pub const VERSION`。
- [ ] **Step 5:** 在前端 `src/shared/api/tauri.ts` 保持原样（后续 Task 9 再改）。把界面中出现的产品名 "yi-llm" 改为 "yi-token"（只改 i18n 文案和 `AppLogo` alt 文本，不改样式）；用 `grep -rn "yi-llm\|yillm" src` 确认只剩下需要在 Task 9 / 24 处理的 API 调用与 `.yillm` 扩展名。
- [ ] **Step 6:** 运行 `npm install`、`npm run typecheck`、`npm run lint`、`cargo build`、`cargo clippy --all-targets -- -D warnings`，全部通过。运行 `npm run tauri dev`，确认窗口能打开（页面数据为空、调用报错可以接受）。
- [ ] **Step 7:** 提交 `chore: scaffold workspace and import yi-llm frontend`。

---

## 阶段 1：领域与存储

### Task 2: 错误类型与路径

**Files:**
- Create: `core/src/error.rs`、`core/src/paths.rs`
- Test: 同文件内 `#[cfg(test)] mod tests`

**Interfaces:**
- Produces:
  - `pub enum ErrorCode { Validation, NotFound, Conflict, Database, Io, Upstream, Internal }`，serde 序列化为 snake_case。
  - `pub enum CoreError { Validation(String), NotFound(String), Conflict(String), Database(String), Io(String), Upstream(UpstreamError), Internal(String) }`，`impl CoreError { pub fn code(&self) -> ErrorCode; pub fn message(&self) -> String }`，实现 `Serialize` 为 `{ code, message }`。
  - `pub struct UpstreamError { pub status: u16, pub body: bytes::Bytes, pub content_type: Option<String> }`；`message()` 返回"上游返回 {status}：{body 前 2000 字节的 UTF-8 有损文本}"。
  - `From<rusqlite::Error>`/`From<r2d2::Error>` → Database；`From<std::io::Error>` → Io；`From<serde_json::Error>` → Validation。**不**为 `String` 实现 `From`（避免 yi-llm 中一切字符串都变成 Validation 的隐式行为）。
  - `pub type CoreResult<T> = Result<T, CoreError>;`
  - `pub struct AppPaths { pub home: PathBuf, pub db: PathBuf, pub log_dir: PathBuf, pub catalog_override: Option<PathBuf> }`，`impl AppPaths { pub fn from_env() -> CoreResult<Self>; pub fn under(home: PathBuf) -> Self }`。

- [ ] **Step 1:** 写测试 `error_serializes_code_and_message`（`CoreError::Conflict("x")` 序列化为 `{"code":"conflict","message":"x"}`）、`upstream_message_truncates_body`、`paths_respect_yi_token_home`（设置 `YI_TOKEN_HOME` 为临时目录，`db` 为 `<home>/yi-token.db`，`log_dir` 为 `<home>/logs`）、`paths_default_to_dot_yi_token`（未设置时 `home` 以 `.yi-token` 结尾）、`catalog_override_from_env`。环境变量测试用 `serial_test` 或在单个测试内设置后恢复。
- [ ] **Step 2:** 运行 `cargo test -p yi-core error paths`，确认失败。
- [ ] **Step 3:** 实现。
- [ ] **Step 4:** 测试通过。
- [ ] **Step 5:** 提交 `feat(core): error type and app paths`。

### Task 3: 领域类型

**Files:**
- Create: `core/src/domain/{mod,protocol,client,provider,model,standard_model,capabilities,thinking,terminal,settings,usage,names}.rs`

**参考:** `main:src-tauri/src/domain/*.rs`、`main:src/features/*/types.ts`（字段名与 serde 名必须与前端一致）。

**Interfaces:**
- Produces（全部 `#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]`，`#[ts(export, export_to = "../../src/generated/")]`）：
  - `enum Protocol { Anthropic, OpenaiChat, Responses }`，serde 名 `anthropic` / `openai_chat` / `responses`；`fn as_str()`，`FromStr`，`fn all() -> [Protocol; 3]`。
  - `enum Client { Codex, ClaudeCode, Opencode, Pi, DeepseekHarness }`，serde 名 `codex` / `claude-code` / `opencode` / `pi` / `deepseek-harness`；`fn protocols(&self) -> &'static [Protocol]`（codex 仅 Responses，claude-code 仅 Anthropic，其余三种全部）；`fn default_protocol()`。
  - `enum Thinking { Minimal, Low, Medium, High, Xhigh }`。
  - `struct Capabilities`：字段与 yi-llm `domain/capabilities.rs` 相同（`input_modalities`、`output_modalities`、`context_window`、`max_output_tokens`、`effort: Effort { support, levels, default }`），方法 `validate(protocol)`、`efforts_for(client, upstream)`、`supports_reasoning() -> bool`。
  - `struct Provider { id, provider_type: Protocol (serde "type"), name, short_code, base_url, api_key, enabled, thinking: Option<Thinking>, is_default, sort_order, extra: serde_json::Value, protocols: Vec<Protocol> }`，`fn validate(&self) -> CoreResult<()>`（`short_code` 仅字母数字与 `-`、1–32 字符；`base_url` 必须是 http/https；`protocols` 必须包含 `provider_type`）。
  - `struct Model { id, provider_id, standard_model_id, upstream_model, exposed_name, capabilities, non_standard }`；前端 DTO `ModelMapping`（serde 字段 `id`、`route_id`=exposed_name、`name`=upstream_model 等，照 yi-llm types.ts）放在 `services::dto`（Task 7）。
  - `fn exposed_name(upstream_model: &str, short_code: &str) -> String`（`names.rs`），格式 `{upstream}({short_code})`。
  - `struct StandardModel { id, name, protocol, brand, capabilities }`。
  - `enum RouteMode { Fixed, Auto }`；`struct ProxyProfile { client, protocol, route_mode, models: Vec<ProfileModel { provider_id, model }>, default_model: Option<String> }`；`enum ModelSource { Native, Provider }`；`struct DirectProfile { client, provider_id, model_source, model: Option<String> }`；`fn ProxyProfile::validate(&self)`（protocol 必须属于 `client.protocols()`；fixed 时 models 非空且 default_model ∈ models）。
  - `struct Settings { host, port: u16, log_level: LogLevel }`，`enum LogLevel { Error, Warn, Info, Debug, Trace }`，`Settings::default()` 为 `127.0.0.1:11435 info`。
  - `struct UsageEvent { requested_at, client: Option<Client>, route_mode: Option<RouteMode>, provider_name, protocol, model, input_tokens, output_tokens, cached_tokens, reasoning_tokens }`。
  - `pub const AUTO_MODEL: &str = "Auto";`

- [ ] **Step 1:** 写测试：`protocol_serde_names`、`client_serde_names_and_protocols`、`provider_validate_rejects_bad_short_code`、`provider_validate_requires_native_protocol`、`exposed_name_format`（`exposed_name("gpt-5", "oa") == "gpt-5(oa)"`）、`proxy_profile_rejects_protocol_not_supported_by_client`（codex + anthropic 报 Validation）、`proxy_profile_auto_allows_empty_models`、`capabilities_validate_matches_yi_llm`（从 yi-llm `domain/capabilities.rs` 的单测中挑 3 个用例转写断言）。
- [ ] **Step 2:** 运行失败。
- [ ] **Step 3:** 实现。中文错误消息措辞照搬 yi-llm 对应校验。
- [ ] **Step 4:** 测试通过；`cargo test -p yi-core export_bindings` 生成 `src/generated/*.ts`（ts-rs 默认测试），确认文件出现。
- [ ] **Step 5:** 提交 `feat(core): domain types`。

### Task 4: SQLite 连接池、迁移与 repo

**Files:**
- Create: `core/src/storage/{mod,pool,migrations,providers,standard_models,models,terminal,engines,settings,usage}.rs`
- Test: `core/tests/storage_repos.rs`

**参考:** `main:src-tauri/src/db/**`（字段默认值、排序规则、`short_code` 唯一校验的中文提示）。

**Interfaces:**
- Produces:
  - `pub struct Db { pool: r2d2::Pool<SqliteConnectionManager> }`；`Db::open(path: &Path) -> CoreResult<Db>`；`Db::open_in_memory() -> CoreResult<Db>`（共享缓存 URI，测试用）；`Db::with_conn<T>(&self, f: impl FnOnce(&Connection) -> CoreResult<T>) -> CoreResult<T>`；`Db::with_tx<T>(&self, f: impl FnOnce(&Transaction) -> CoreResult<T>)`。
  - 每个连接执行 `PRAGMA journal_mode=WAL; foreign_keys=ON; busy_timeout=5000`。
  - 迁移：`rusqlite_migration::Migrations`，v1 一次建全部表（spec 4.2），含索引 `usage(requested_at)`、`usage(client)`，`providers.short_code COLLATE NOCASE UNIQUE`，`models.exposed_name UNIQUE`，`standard_models UNIQUE(name, protocol)`，`terminal_engines.model_id REFERENCES models(id) ON DELETE SET NULL`，`terminal_direct_profiles.provider_id ... ON DELETE CASCADE`，`settings` 插入默认行。
  - repo 函数（`&Connection` 或 `&Transaction` 为第一参数的自由函数，按模块分组）：
    - `providers::{list, get, upsert(provider, models: &[Model]), delete, set_default}`；`upsert` 在同一事务里重写 `provider_protocols` 与该服务商的 `models`（按 `upstream_model` 匹配保留已有 model id）。
    - `standard_models::{list, upsert, delete, replace_catalog_entries}`。
    - `models::{list, list_by_provider, get}`。
    - `terminal::{list_proxy, get_proxy, save_proxy, list_direct, get_direct, save_direct}`。
    - `engines::{list, get, set(client, Option<model_id>)}`。
    - `settings::{get, save}`。
    - `usage::{insert_batch(&[UsageEvent]), query(UsageQuery) -> UsageReport}`；`UsageQuery { start_ms, end_ms, model: Option<String> }`；`UsageReport` 字段与 yi-llm `domain/usage.rs` 的报表结构一致（records、按日、按模型、按日×模型），新增按 `client` 聚合 `by_client`。
  - 冲突映射：唯一约束失败映射为 `CoreError::Conflict`，中文消息（如"简称 xx 已被服务商 yy 使用"）；`delete` 标准模型被模型引用时映射为 Conflict（RESTRICT）。

- [ ] **Step 1:** 写集成测试（`core/tests/storage_repos.rs`，每个测试用 `Db::open_in_memory()`）：
  - `migration_sets_user_version_1`；`migration_is_atomic`（向迁移注入一个故意失败的 v2 时 `user_version` 仍为 1 —— 用 `Migrations::new(vec![v1, bad])` 的测试辅助函数，覆盖 B2）。
  - `provider_roundtrip_with_protocols_and_models`；`short_code_unique_case_insensitive`（`OA` 与 `oa` 冲突 → Conflict）；`exposed_name_unique`。
  - `upsert_keeps_model_id_for_same_upstream`。
  - `delete_provider_cascades_models_and_direct_profile_and_nulls_engine`（Review Focus 5 的存储部分）。
  - `delete_standard_model_in_use_is_conflict`。
  - `settings_default_row`；`proxy_profile_roundtrip`；`engine_set_and_clear`。
  - `usage_query_aggregates_by_day_model_and_client`。
- [ ] **Step 2:** 运行 `cargo test -p yi-core --test storage_repos`，失败。
- [ ] **Step 3:** 实现。行映射用 `serde_rusqlite::from_rows` 到 `*Row` 结构，再转换为 domain 类型；JSON 列用 `serde_json` 字符串。
- [ ] **Step 4:** 测试通过。
- [ ] **Step 5:** 提交 `feat(core): sqlite storage with migrations and repos`。

### Task 5: 模型目录

**Files:**
- Create: `core/src/catalog/{mod,file,sync}.rs`
- Test: `core/tests/catalog.rs`

**参考:** `main:src-tauri/src/catalog/{file,service}.rs`、`main:models/catalog.schema.json`。

**Interfaces:**
- Produces:
  - `pub struct CatalogFile { pub version: String, pub entries: Vec<StandardModel> }`；`catalog::file::load(path: Option<&Path>) -> CoreResult<CatalogFile>`（无覆盖时用 `include_str!("../../../models/catalog.json")`）。
  - `pub struct CatalogInfo { pub source: String, pub path: Option<String>, pub version: String, pub count: usize, pub loaded_at: i64 }`（字段对照前端 `get_model_catalog_info` 的类型）。
  - `catalog::sync::apply(tx, &CatalogFile) -> CoreResult<()>`：按 `(name, protocol)` 插入或更新内置条目，不删除用户自建条目。
  - `pub struct CatalogWatcher { path, last_mtime }`，`fn changed(&mut self) -> bool`（用 mtime 判断；只在有覆盖路径时生效）。

- [ ] **Step 1:** 测试：`builtin_catalog_parses`（条目数 > 0，所有条目 `capabilities.validate` 通过）；`override_path_is_used`；`sync_is_idempotent`；`sync_preserves_user_entries`；`watcher_detects_mtime_change`。
- [ ] **Step 2–4:** 失败 → 实现 → 通过。
- [ ] **Step 5:** 提交 `feat(core): model catalog loading and sync`。

### Task 6: 用量写入器

**Files:**
- Create: `core/src/storage/usage_writer.rs`
- Test: 同文件 tests

**Interfaces:**
- Produces: `pub struct UsageSink { tx: std::sync::mpsc::SyncSender<UsageEvent> }`，`impl UsageSink { pub fn record(&self, e: UsageEvent) }`（`try_send`，满时 `tracing::warn!` 后丢弃）；`pub fn spawn_usage_writer(db: Db) -> (UsageSink, UsageWriterHandle)`，容量 4096，每批最多 256 条或 500 ms 刷新一次；`UsageWriterHandle::shutdown(self)` 刷完剩余记录后退出。

- [ ] **Step 1:** 测试：`records_are_flushed_in_batches`（写 600 条，shutdown 后数据库 600 条）；`full_queue_drops_without_blocking`（用一个阻塞的假 Db 或容量 1 的构造函数，`record` 立即返回）。
- [ ] **Step 2–4:** 失败 → 实现 → 通过。
- [ ] **Step 5:** 提交 `feat(core): batched usage writer`。

---

## 阶段 2：门面与前端接入

### Task 7: services 门面（服务商、模型、设置、用量、日志）

**Files:**
- Create: `core/src/services/{mod,dto,providers,models,settings,usage,logs}.rs`、`core/src/logging.rs`
- Test: `core/tests/services_basic.rs`

**参考:** `main:src-tauri/src/service/*.rs`、`main:src-tauri/src/commands/*.rs`、`main:src-tauri/src/app/logging.rs`、`main:src/features/{providers,models,usage,proxy}/types.ts`。

**Interfaces:**
- Consumes: Task 2–6。
- Produces:
  - `pub struct Services { … }`，`Services::start(paths: AppPaths) -> CoreResult<Arc<Services>>`：打开数据库、同步目录、启动用量写入器、初始化日志；代理在 Task 18 接入。
  - `Services::for_test(home: &Path) -> Arc<Services>`。
  - `dto.rs`：前端可见的视图类型（`ProviderView`（含 `models: Vec<ModelMapping>`）、`ModelMapping`、`StandardModelView`、`SettingsView`、`UsageReportView`、`CatalogInfo`），全部 `ts_rs::TS` 导出；字段名逐个对照 yi-llm 前端 `types.ts`，**唯一变化**：`ModelMapping.id`、`UsageRecord.id` 为 string（spec 4.4）。
  - 异步方法（内部 `tokio::task::spawn_blocking`）：
    - `list_providers() -> Vec<ProviderView>`；`save_provider(ProviderInput, Vec<ModelInput>) -> ProviderView`（新建时后端生成 UUID；忽略前端传入的空 id）；`delete_provider(id)`。（`test_provider` 依赖上游客户端，在 Task 17 实现、Task 18 注册命令，本任务不涉及。）
    - `list_standard_models()`、`get_model_catalog_info()`、`save_standard_model()`、`delete_standard_model(id)`。
    - `get_settings()`、`save_settings(host, port, log_level)`（保存后调用 `logging::set_level`；代理重启在 Task 18 接入）。
    - `get_usage(UsageQueryInput { days, model, start_date, end_date }) -> UsageReportView`（日期为本地日期字符串 `YYYY-MM-DD`，转换规则照 yi-llm `service/usage.rs`）。
    - `get_logs() -> LogsView`（日志文件尾部，上限 512 KiB / 1000 行）。
  - `logging::init(log_dir, level) -> LoggingGuard`；`logging::set_level(LogLevel)`（`tracing_subscriber::reload`）；`logging::tail(log_dir, max_bytes, max_lines) -> Vec<String>`。
  - 每次写操作后调用 `self.routes_changed()`（本任务为空实现，Task 16 接入路由快照刷新）。

- [ ] **Step 1:** 测试：`save_provider_generates_uuid_and_lists`；`save_provider_duplicate_short_code_is_conflict`；`save_settings_changes_log_level`；`get_usage_by_date_range`；`logs_tail_respects_line_limit`；`dto_field_names_match_frontend`（把 `ProviderView` 序列化后断言包含 yi-llm 前端使用的全部键：从 `main:src/features/providers/types.ts` 抄出键列表）。
- [ ] **Step 2–4:** 失败 → 实现 → 通过。
- [ ] **Step 5:** 提交 `feat(core): services facade for providers, models, settings, usage, logs`。

### Task 8: Tauri 命令层

**Files:**
- Create: `src-tauri/src/{commands,state}.rs`；Modify: `src-tauri/src/lib.rs`

**参考:** `main:src-tauri/src/lib.rs`、`main:src-tauri/src/commands/*.rs`（参数名、可选参数、返回类型）。

**Interfaces:**
- Consumes: `yi_core::services::Services`、`yi_core::error::CoreError`。
- Produces: `state.rs` 中 `pub struct AppState(pub Arc<Services>)`；`lib.rs` 在 `setup` 中调用 `Services::start(AppPaths::from_env()?)` 并 `manage`；`commands.rs` 注册本阶段可用的命令：`list_providers`、`save_provider`、`delete_provider`、`list_standard_models`、`get_model_catalog_info`、`save_standard_model`、`delete_standard_model`、`get_settings`、`save_settings`、`get_logs`、`get_usage`。命令错误类型直接返回 `CoreError`（已实现 `Serialize`）。`export_provider_config` / `import_provider_config` 的文件对话框用 `rfd`，放在 Task 24。

- [ ] **Step 1:** 在 `src-tauri` 写一个不启动窗口的单测 `commands_delegate_to_services`：用 `Services::for_test` 构造，直接调用命令函数体（把命令函数写成接收 `&Services` 的内部函数 + 薄 `#[tauri::command]` 包装，测试调用内部函数），断言 `save_provider` 后 `list_providers` 返回 1 条。
- [ ] **Step 2–4:** 失败 → 实现 → 通过。
- [ ] **Step 5:** 提交 `feat(app): tauri commands for providers, models, settings, usage, logs`。

### Task 9: 前端数据层接入

**Files:**
- Modify: `src/features/{providers,models,usage,proxy,terminal}/types.ts`、`src/features/*/api.ts`（仅类型导入）、`src/shared/api/tauri.ts`、`tsconfig.json`（加 `src/generated` 路径）、`package.json`（加脚本 `gen:types` = `cargo test -p yi-core export_bindings`）
- Create: `src/generated/index.ts`（汇总导出）

**Interfaces:**
- Consumes: Task 3、7 生成的 TS 类型。
- Produces: 前端 `types.ts` 中与后端 DTO 对应的类型改为 `export type { X } from "../../generated"` 重导出；仅前端使用的类型（表单草稿、选择状态）保留在原文件。

- [ ] **Step 1:** 运行 `npm run gen:types`；`npm run typecheck`，记录全部类型错误。
- [ ] **Step 2:** 逐个改为重导出生成类型；`ModelMapping.id` / `UsageRecord.id` 由 number 改为 string 后，修正使用处（比较、Map key、`Number(...)` 转换）。只改类型与数据转换代码，不改 JSX 结构和 className。
- [ ] **Step 3:** `src/shared/api/tauri.ts` 的错误解析保持 `{ code, message }`，确认 code 联合类型与后端 `ErrorCode` 一致（改为引用生成类型）。
- [ ] **Step 4:** `npm run typecheck`、`npm run lint` 通过；`npm run tauri dev` 手动验证：新建服务商、添加模型、编辑标准模型、修改日志级别、查看用量页（空数据）均正常。
- [ ] **Step 5:** 提交 `feat(ui): wire frontend types to generated backend DTOs`。

---

## 阶段 3：协议层

> 本阶段全部是纯函数，不依赖 storage 和网络。行为参考 `main:src-tauri/src/protocol/**`，缺陷修复参考 `refactor/backend-rust:src-tauri/src/protocol/**`。

### Task 10: Responses 中间形态与 SSE 分帧

**Files:**
- Create: `core/src/protocol/{mod,ir,sse}.rs`、`core/src/protocol/responses/{mod,types,stream}.rs`
- Test: 同文件 tests

**Interfaces:**
- Produces:
  - `ir::Request`：Responses 请求的类型化结构（`model`、`input: Vec<InputItem>`、`instructions`、`tools: Vec<Tool>`、`tool_choice`、`reasoning: Option<Reasoning { effort, summary }>`、`max_output_tokens`、`temperature`、`top_p`、`stream`、`store`、`previous_response_id`、`include`、`metadata`、`#[serde(flatten)] extra: Map<String, Value>`）。`InputItem` / `ContentPart` / `Tool` 为带 `#[serde(tag = "type")]` 的枚举，每个都有 `#[serde(other)]` 无法保留原文，所以用 `Unknown(Value)` 变体 + 自定义反序列化（先按 `type` 匹配已知变体，失败则存原始 `Value`）。
  - `ir::Response`（非流式）与 `ir::StreamEvent`（Responses 流事件枚举，含 `Unknown { event: String, data: Value }`）。`encrypted_content`、`signature` 等字段作为普通字段保留（B4）。
  - `sse::Decoder`：`fn push(&mut self, chunk: &[u8]) -> Vec<SseFrame>`，`SseFrame { event: Option<String>, data: String }`；在完整帧（空行）出现前只缓冲字节，完整后再 UTF-8 解码（B1）。基于 `eventsource-stream` 的 `EventStream` 实现或自行实现均可，但必须满足测试。
  - `sse::encode(event: Option<&str>, data: &str) -> Bytes`。
  - `responses::stream::Collector`：把 Responses 流事件聚合为 `ir::Response`（供非流式客户端使用）。

- [ ] **Step 1:** 测试：`unknown_input_item_roundtrips`（未知 `type` 的输入项序列化后与原 JSON 相等）；`extra_fields_are_preserved`；`encrypted_content_survives_roundtrip`；`sse_decoder_handles_multibyte_split`（把 `data: 你好\n\n` 的字节在"你"的第 2 个字节处切开分两次 push，得到 `你好`，无 U+FFFD）；`sse_decoder_handles_crlf_and_comments`；`collector_builds_response_from_events`（用 yi-llm `main:src-tauri/src/protocol/responses/sse.rs` 测试里的事件序列）。
- [ ] **Step 2–4:** 失败 → 实现 → 通过。
- [ ] **Step 5:** 提交 `feat(protocol): responses IR and byte-buffered SSE decoder`。

### Task 11: Anthropic 进出与流转换

**Files:**
- Create: `core/src/protocol/anthropic/{mod,types,inbound,outbound,stream}.rs`
- Test: 同文件 tests

**参考:** `main:src-tauri/src/protocol/client.rs`（anthropic → responses 的 `normalize_body` 部分）、`main:src-tauri/src/protocol/translate/anthropic.rs`、`main:src-tauri/src/protocol/upstream/anthropic/**`、`main:src-tauri/src/protocol/respond.rs`。

**Interfaces:**
- Produces:
  - `anthropic::inbound::to_ir(body: Value) -> CoreResult<ir::Request>`（Anthropic Messages 请求 → IR）。
  - `anthropic::outbound::from_ir(req: &ir::Request, ctx: &OutboundCtx) -> CoreResult<Value>`（IR → Anthropic 上游请求体）。`OutboundCtx { upstream_model: String, capabilities: Capabilities, thinking: Option<Thinking> }`（定义在 `protocol/mod.rs`，三种协议共用）。
  - `anthropic::stream::UpstreamToIr`：`fn on_frame(&mut self, frame: SseFrame) -> CoreResult<Vec<ir::StreamEvent>>`（Anthropic 上游流 → IR 事件）；`fn usage(&self) -> Usage`。
  - `anthropic::stream::IrToClient`：`fn on_event(&mut self, ev: &ir::StreamEvent) -> Vec<Bytes>`（IR 事件 → Anthropic 客户端 SSE）；以及非流式 `anthropic::outbound::response_to_client(resp: &ir::Response) -> Value` 与 `anthropic::inbound::response_to_ir(body: Value) -> ir::Response`。
  - 未知内容块（`server_tool_use`、`web_search_tool_result`、`citations_delta` 等）保留为 `Unknown`，同协议路径原样输出，跨协议路径记录 `debug` 日志后跳过（B3）。
  - `pub struct Usage { input_tokens, output_tokens, cached_tokens, reasoning_tokens }`（定义在 `protocol/usage.rs`）。

- [ ] **Step 1:** 测试（JSON 夹具放在 `core/tests/fixtures/anthropic/`，从 yi-llm main 的单测/夹具中整理）：`messages_with_tools_to_ir`、`system_blocks_become_instructions`、`tool_result_pairs_preserved`、`ir_to_messages_with_thinking_budget`（与 Task 13 的 thinking 映射一致）、`stream_text_and_tool_use_to_ir`、`stream_unknown_block_preserved`（B3）、`stream_usage_extracted`、`ir_events_to_anthropic_sse_order`（`message_start` → `content_block_start` → … → `message_stop`）。
- [ ] **Step 2–4:** 失败 → 实现 → 通过。
- [ ] **Step 5:** 提交 `feat(protocol): anthropic inbound/outbound and stream translation`。

### Task 12: OpenAI Chat 进出与流转换

**Files:**
- Create: `core/src/protocol/chat/{mod,types,inbound,outbound,stream}.rs`
- Test: 同文件 tests

**参考:** `main:src-tauri/src/protocol/translate/chat_completions.rs`、`main:src-tauri/src/protocol/upstream/openai_chat/**`、`main:src-tauri/src/protocol/client.rs`（chat → responses 部分）；v0.1.7→v0.1.8 的修复提交 `c3651b4`（"keep chat tool messages paired and record chat stream usage"）。

**Interfaces:**
- Produces（与 Task 11 对称）：`chat::inbound::{to_ir, response_to_ir}`、`chat::outbound::{from_ir, response_to_client}`、`chat::stream::{UpstreamToIr, IrToClient}`。
  - 保留 `reasoning_content`（厂商扩展）；工具调用消息与 `tool` 消息保持成对（缺失配对时报 Validation，消息照 yi-llm）。
  - 流式上游请求自动加 `stream_options: { include_usage: true }`。

- [ ] **Step 1:** 测试：`chat_messages_to_ir`、`tool_messages_stay_paired`、`reasoning_content_preserved`、`ir_to_chat_sets_reasoning_effort`、`stream_tool_call_deltas_assembled`、`stream_usage_from_final_chunk`、`ir_events_to_chat_sse_with_done`（最后输出 `data: [DONE]`）。
- [ ] **Step 2–4:** 失败 → 实现 → 通过。
- [ ] **Step 5:** 提交 `feat(protocol): openai chat inbound/outbound and stream translation`。

### Task 13: prepare、thinking 与兼容规则

**Files:**
- Create: `core/src/protocol/{prepare,thinking,compatibility,usage}.rs`
- Test: 同文件 tests

**参考:** `main:src-tauri/src/protocol/{prepare,compatibility,schema}.rs`。

**Interfaces:**
- Produces:
  - `pub struct PrepareCtx { pub inbound: Protocol, pub upstream: Protocol, pub upstream_model: String, pub capabilities: Capabilities, pub thinking: Option<Thinking>, pub non_standard: bool }`。
  - `prepare::passthrough(body: &mut Value, ctx: &PrepareCtx) -> CoreResult<()>`：替换 `model`；`non_standard` 时调用 `compatibility::adapt`；调用 `thinking::apply_raw`。
  - `prepare::validate_for_translation(req: &ir::Request, upstream: Protocol) -> CoreResult<()>`：上游非 Responses 时，`store == Some(true)` 或存在 `previous_response_id` → Validation（中文提示"该上游不支持服务端会话状态（store / previous_response_id）"）。
  - `thinking::has_reasoning(protocol, body: &Value) -> bool`：Anthropic 看 `thinking`，Chat 看 `reasoning_effort`，Responses 看 `reasoning.effort`。
  - `thinking::apply_raw(protocol, body: &mut Value, thinking: Option<Thinking>, caps: &Capabilities)`：仅在 `!has_reasoning && caps.supports_reasoning()` 时写入；Anthropic 预算按映射表并 `min(budget, max_tokens - 1)`，`max_tokens` 不足 1024 时不开启；`Minimal` 对 Anthropic 不开启，对 Chat/Responses 写 `minimal`。
  - `compatibility::adapt(body: &mut Value, protocol: Protocol) -> CompatibilityReport`：规则照 yi-llm（Responses 下移除 `namespace` / `web_search` / `web_search_preview` 工具和 `message.annotations` include 等）。

- [ ] **Step 1:** 测试：`thinking_fills_anthropic_budget`（medium、`max_tokens` 20000 → `budget_tokens` 10000）；`thinking_budget_clamped_below_max_tokens`；`thinking_respects_client_value`（请求已有 `reasoning_effort: low`，服务商 high → 仍为 low）；`thinking_skipped_when_model_has_no_reasoning`；`thinking_minimal_anthropic_disabled`；`store_rejected_for_chat_upstream`；`previous_response_id_allowed_for_responses_upstream`；`compatibility_drops_namespace_tools_only_when_non_standard`；`passthrough_replaces_model_and_keeps_unknown_fields`。
- [ ] **Step 2–4:** 失败 → 实现 → 通过。
- [ ] **Step 5:** 提交 `feat(protocol): request preparation, thinking fallback, compatibility rules`。

### Task 14: 协议门面与 oracle 语义比对

**Files:**
- Modify: `core/src/protocol/mod.rs`
- Create: `core/tests/oracle.rs`、`core/tests/fixtures/oracle/`（导出的语料）、`scripts/export-oracle.md`（导出步骤说明）

**Interfaces:**
- Produces（`protocol/mod.rs`）：
  - `pub fn to_ir(inbound: Protocol, body: Value) -> CoreResult<ir::Request>`（Responses 入站直接反序列化）。
  - `pub fn build_upstream(req: &ir::Request, upstream: Protocol, ctx: &OutboundCtx) -> CoreResult<Value>`。
  - `pub fn choose_upstream(inbound: Protocol, provider_protocols: &[Protocol], native: Protocol) -> (Protocol, bool /*passthrough*/)`。
  - `pub fn stream_translator(inbound: Protocol, upstream: Protocol) -> Box<dyn StreamTranslator + Send>`，`trait StreamTranslator { fn on_frame(&mut self, f: SseFrame) -> CoreResult<Vec<Bytes>>; fn finish(&mut self) -> Vec<Bytes>; fn usage(&self) -> Usage; }`；同协议时实现为"原样转发 + 提取用量"。
  - `pub fn translate_response(inbound: Protocol, upstream: Protocol, body: Value) -> CoreResult<(Value, Usage)>`（非流式）。

- [ ] **Step 1:** 导出语料：在 yi-llm 中**不修改分支**，用 `git -C D:/Github/yi-llm worktree add ../yi-llm-oracle refactor/backend-rust`，在该 worktree 运行 `cargo test --test protocol_oracle`，取得 `target/protocol_oracle_full.txt`；写一个一次性脚本把每行解析为 `{ case_id, inbound, upstream, field, value, outcome: "accepted" | "rejected", upstream_body?: Value }` 的 JSONL，存入 `core/tests/fixtures/oracle/request_pipeline.jsonl`。把步骤写进 `scripts/export-oracle.md`。完成后 `git worktree remove ../yi-llm-oracle`。
- [ ] **Step 2:** 写 `core/tests/oracle.rs`：逐条执行 `to_ir` → `validate_for_translation` → `build_upstream`（或直通路径的 `prepare::passthrough`），比较"接受/拒绝"以及上游 JSON（`serde_json::Value` 相等，忽略键顺序）；不比较错误文案。维护 `ALLOWED_DIFFS: &[(&str /*case_id 前缀*/, &str /*原因*/)]`，至少包含 B6（thinking 现在会生效）。测试输出不匹配用例的汇总（前 50 条）。
- [ ] **Step 3:** 运行 `cargo test -p yi-core --test oracle`，记录失败条目，逐类修复 Task 10–13 的实现，直到只剩白名单中的差异。每类修复单独提交。
- [ ] **Step 4:** 测试 `choose_upstream_prefers_passthrough`（服务商支持入站协议 → 直通）、`choose_upstream_falls_back_to_native`。
- [ ] **Step 5:** 提交 `test(protocol): semantic oracle against yi-llm request pipeline`。

---

## 阶段 4：代理层

### Task 15: 代理生命周期

**Files:**
- Create: `core/src/proxy/{mod,server,errors}.rs`
- Test: `core/tests/proxy_lifecycle.rs`

**参考:** `main:src-tauri/src/proxy/{lifecycle,errors}.rs`。

**Interfaces:**
- Produces:
  - `pub struct ProxyHandle`：`async fn start(&self, addr: SocketAddr) -> CoreResult<()>`、`async fn stop(&self)`、`async fn restart(&self, addr: SocketAddr) -> CoreResult<()>`、`fn status(&self) -> ProxyStatus { running: bool, addr: Option<SocketAddr>, started_at: Option<i64>, last_error: Option<String> }`。
  - 内部：`TcpListener::bind` 失败 → `CoreError::Io("端口 {port} 已被占用或无法监听：{err}")`；`axum::serve(...).with_graceful_shutdown(oneshot)`。
  - `errors.rs`：`impl IntoResponse for CoreError`，按 Global Constraints 的 HTTP 映射；非 upstream 错误的响应体为 `{ "error": { "type": code, "message": message } }`；upstream 错误原样返回状态码、`content-type` 与 body（B5）。
  - `ProxyHandle::new(router_factory: impl Fn() -> axum::Router + Send + Sync + 'static)`（Task 16 提供真实 router）。

- [ ] **Step 1:** 测试：`start_stop_restart`（端口用 `127.0.0.1:0` 改为先绑一个临时端口取号再释放）；`start_on_busy_port_returns_io_error`（先用 `std::net::TcpListener` 占用端口，`start` 返回 Io，`status().running == false`，释放后 `start` 成功——Review Focus 3）；`validation_error_maps_to_400`、`conflict_maps_to_409`、`upstream_error_passthrough_status_and_body`。
- [ ] **Step 2–4:** 失败 → 实现 → 通过。
- [ ] **Step 5:** 提交 `feat(proxy): lifecycle and error responses`。

### Task 16: 路由快照、路由表与解析

**Files:**
- Create: `core/src/proxy/{router,resolve}.rs`、`core/src/storage/route_source.rs`
- Test: `core/tests/proxy_resolve.rs`

**参考:** `main:src-tauri/src/proxy/{router,routing}.rs`、`main:src-tauri/src/terminal/resolve.rs`。

**Interfaces:**
- Produces:
  - `pub trait RouteSource: Send + Sync { fn snapshot(&self) -> CoreResult<RouteSnapshot>; }`（定义在 `proxy/resolve.rs`）；`storage::route_source::DbRouteSource { db: Db }` 实现它。
  - `pub struct RouteSnapshot { providers: HashMap<String, ProviderRoute>, by_exposed: HashMap<String, (provider_id, model_id)>, default_provider: Option<String>, proxy_profiles: HashMap<Client, ProxyProfile>, engines: HashMap<Client, Option<String /*model_id*/>>, models: HashMap<String, Model> }`；`ProviderRoute { provider: Provider, models_by_upstream: HashMap<String, Model> }`；只包含 `enabled` 的服务商。
  - `pub struct Routes { current: ArcSwap<RouteSnapshot>, source: Arc<dyn RouteSource> }`，`fn refresh(&self) -> CoreResult<()>`、`fn load(&self) -> Arc<RouteSnapshot>`。
  - `pub enum Scope { Global, Client(Client) }`；`pub struct Resolved { pub provider: Provider, pub model: Option<Model>, pub upstream_model: String, pub route_mode: Option<RouteMode> }`。
  - `resolve::resolve(snap: &RouteSnapshot, scope: Scope, inbound: Protocol, requested_model: &str) -> CoreResult<Resolved>`：
    - Global：`by_exposed` 精确匹配且服务商支持 inbound；否则交给默认服务商并原样使用模型名；都没有 → NotFound("未找到模型 {name}，且没有默认服务商")。
    - Client + fixed：在该终端 `models` 中按 exposed_name 或 `{provider_id, model}` 匹配；未配置 → NotFound("终端 {client} 尚未配置代理")。
    - Client + auto：见 Task 22（本任务对 auto 返回 `Internal("auto 未实现")` 并在 Task 22 替换；测试只覆盖 Global 与 fixed）。
  - `router::build(state: ProxyState) -> axum::Router`：注册 spec 5.1 的全部路由；所有 POST 推理路由 `DefaultBodyLimit::disable()`；`/clients/{client}` 中未知 client → 404。`ProxyState { routes: Arc<Routes>, http: UpstreamClient, usage: UsageSink, monitor: Arc<Monitor> }`。
  - `GET /v1/models` 与 `/clients/{client}/v1/models` 的响应形状照 yi-llm `handlers/models.rs`（auto 时只列出 `Auto`，在 Task 22 补）。
  - `Services::routes_changed()` 改为调用 `routes.refresh()`。

- [ ] **Step 1:** 测试：`global_exact_match`；`global_requires_protocol_support`；`global_falls_back_to_default_provider`；`disabled_provider_excluded`；`fixed_profile_resolves_model`；`client_without_profile_is_not_found`；`refresh_swaps_snapshot`（修改数据库后 `refresh`，新解析使用新数据，旧 `Arc<RouteSnapshot>` 不变）；`unknown_client_path_is_404`（用 `tower::ServiceExt::oneshot` 调 router）；`large_body_is_not_rejected`（8 MiB JSON 发到 `/clients/codex/v1/responses`，返回码不是 413——Review Focus 1，此时上游可用 wiremock）。
- [ ] **Step 2–4:** 失败 → 实现 → 通过。
- [ ] **Step 5:** 提交 `feat(proxy): route snapshot, router and resolution`。

### Task 17: 请求管线、上游客户端与直通

**Files:**
- Create: `core/src/proxy/{pipeline,upstream,headers}.rs`
- Test: `core/tests/proxy_pipeline.rs`、`core/tests/common/mod.rs`

**参考:** `main:src-tauri/src/proxy/{pipeline,handlers/*,usage_scan}.rs`、`main:src-tauri/src/protocol/upstream/{client,headers}.rs`。

**Interfaces:**
- Consumes: Task 13–16。
- Produces:
  - `upstream::UpstreamClient`（包装 `reqwest::Client`，连接超时 30 s，无总超时，读空闲超时 300 s，遵循系统代理设置）；`async fn send(&self, provider: &Provider, protocol: Protocol, path: &str, body: Bytes, stream: bool) -> CoreResult<reqwest::Response>`；非 2xx → `CoreError::Upstream(UpstreamError { status, body, content_type })`。
  - `headers::build(provider, protocol) -> HeaderMap`：Anthropic 用 `x-api-key` + `anthropic-version: 2023-06-01`；OpenAI 用 `authorization: Bearer`；剥离入站 `authorization`、`x-api-key`、`cookie`、`host`、`content-length`；透传 `anthropic-beta`、`openai-beta` 等白名单头（照 yi-llm `headers.rs`）。
  - `pipeline::handle(state: &ProxyState, scope: Scope, inbound: Protocol, headers: HeaderMap, body: Bytes) -> Response`：
    1. 解析 JSON，取 `model` 和 `stream`。
    2. `resolve` 得到 `Resolved`（请求开始即持有快照，Auto 锁定由此保证）。
    3. `choose_upstream`；直通 → `prepare::passthrough`；转换 → `to_ir` → `validate_for_translation` → `build_upstream`（`OutboundCtx` 带上 thinking）。
    4. 发送；流式 → 包装上游字节流：`sse::Decoder` → `StreamTranslator` → axum `Body::from_stream`（直接输出已编码字节，`content-type: text/event-stream`）；非流式 → `translate_response`。
    5. 流结束或出错时通过 `UsageSink` 记录一次用量；用 `Drop` 守卫保证客户端断开时也只记录一次，并且 drop 上游 `reqwest::Response` 以取消请求。
  - `test_provider` 的实现（`Services::test_provider`）：用提供的服务商与模型发一个最小非流式请求，返回 `TestResult { ok, latency_ms, message }`；照 yi-llm `api/providers.rs` 中 test 的请求体。

- [ ] **Step 1:** `core/tests/common/mod.rs`：启动 wiremock 上游、`Services::for_test`、插入服务商（三种协议各一个）、启动代理在随机端口，返回 `TestEnv { base_url, mock, services }`。
- [ ] **Step 2:** 测试矩阵（全部经真实 HTTP）：
  - `passthrough_anthropic_to_anthropic_preserves_unknown_fields`（上游收到的 body 与入站相同，仅 `model` 被替换）。
  - 3×3 转换各一条非流式用例：`translate_{in}_to_{up}_non_stream`（9 个，可用宏生成），断言上游收到的协议形状与下游收到的协议形状。
  - 3 条流式用例：`stream_responses_from_anthropic`、`stream_anthropic_from_chat`、`stream_chat_from_responses`。
  - `stream_chinese_split_across_chunks`（wiremock 返回的流在中文字符中间分块——用自定义 `Respond` 输出分块 body；下游文本无 U+FFFD，B1）。
  - `encrypted_content_passthrough`（B4）。
  - `upstream_429_returned_verbatim`（B5）。
  - `auth_headers_replaced`（入站带 `authorization: Bearer user`，上游收到的是服务商 key）。
  - `usage_recorded_once_for_stream`。
  - `client_disconnect_cancels_upstream`（下游读到第一帧后 drop 连接；上游 mock 的流在 1 秒后才结束，断言用量最多 1 条、监控 in-flight 计数归零——Review Focus 2）。
  - `test_provider_reports_success_and_failure`。
- [ ] **Step 3–4:** 实现 → 通过。
- [ ] **Step 5:** 提交 `feat(proxy): request pipeline with passthrough, translation and streaming`。

### Task 18: 监控、请求日志、代理服务与命令

**Files:**
- Create: `core/src/proxy/{monitor,request_log}.rs`、`core/src/services/proxy.rs`
- Modify: `core/src/services/{mod,settings,providers}.rs`、`src-tauri/src/commands.rs`
- Test: `core/tests/proxy_monitor.rs`

**参考:** `main:src-tauri/src/proxy/{monitor,request_log,diagnostics}.rs`、`main:src-tauri/src/service/proxy.rs`、`main:src/features/proxy/types.ts`。

**Interfaces:**
- Produces:
  - `Monitor`：5 秒一个采样桶，环形保留 721 个；每个桶记录请求数、错误数、in-flight、平均/最大延迟、token 数（字段照 yi-llm `monitor.rs` 与前端类型）；按 `(Option<Client>, Protocol)` 的路由统计。`fn snapshot(minutes: Option<u32>) -> MonitorView`。
  - `request_log` 中间件：记录方法、路径、状态码、延迟到 `tracing::info!`，并更新 `Monitor`。
  - `Services`：`get_proxy_status() -> bool`、`get_proxy_runtime() -> ProxyRuntimeView`、`get_proxy_monitor(minutes) -> MonitorView`、`control_proxy(action: "start" | "stop" | "restart")`、`test_provider(...)`。`Services::start` 在启动时自动启动代理；启动失败只记录 `last_error`，不阻止应用启动。`save_settings` 在 host/port 变化时 `restart`。
  - 命令注册：`get_proxy_status`、`get_proxy_runtime`、`get_proxy_monitor`、`control_proxy`、`test_provider`。

- [ ] **Step 1:** 测试：`monitor_buckets_rotate_after_721`（注入可控时钟）；`monitor_route_stats_by_client_protocol`；`save_settings_port_change_restarts_proxy`；`app_starts_when_port_busy`（`Services::start` 成功，`get_proxy_runtime().last_error` 非空）。
- [ ] **Step 2–4:** 失败 → 实现 → 通过。
- [ ] **Step 5:** 手动：`npm run tauri dev`，代理页显示运行状态、监控图、日志；用 `curl` 向 `/v1/chat/completions` 发请求能在日志和用量页看到。
- [ ] **Step 6:** 提交 `feat(proxy): monitor, request log, proxy service and commands`。

---

## 阶段 5：终端配置

### Task 19: Doc 抽象、计划与提交

**Files:**
- Create: `core/src/terminal/{mod,doc,spec,paths,plan,commit}.rs`
- Test: 同文件 tests 与 `core/tests/terminal_commit.rs`

**参考:** `main:src-tauri/src/terminal/{apply,changes,plan}.rs`、`refactor/backend-rust:src-tauri/src/terminal/{commit,render}.rs`。

**Interfaces:**
- Produces:
  - `pub enum DocFormat { Toml, Json, Jsonc, Yaml }`；`pub enum Doc { Toml(toml_edit::DocumentMut), Json(serde_json::Value), Yaml(serde_json::Value) }`；`Doc::parse(format, text: &str) -> CoreResult<Doc>`（解析失败 → Validation("无法解析 {path}：{err}，未做任何修改")）；`Doc::empty(format)`；`Doc::render(&self, format) -> String`（JSON 两空格缩进 + 末尾换行；YAML 用 `serde_norway`）。
  - 路径操作：`Doc::get(&self, path: &[&str]) -> Option<Value>`、`Doc::set(&mut self, path: &[&str], value: Value)`、`Doc::remove(&mut self, path: &[&str])`、`Doc::retain_in(&mut self, path: &[&str], keep: impl Fn(&str, &Value) -> bool)`（用于删除带 yi-token 标识的旧条目）。TOML 的 `set` 保留同表其他键的注释与顺序。
  - `pub struct FileChange { pub path: PathBuf, pub format: DocFormat, pub before: Option<String>, pub after: Option<String> /*None 表示删除*/, pub secrets: Vec<String> }`；`pub struct TerminalPlan { pub client: Client, pub changes: Vec<FileChange> }`；`TerminalPlan::preview(&self) -> TerminalPreview`（把 `secrets` 中的值替换为 `••••••`，输出与 yi-llm 前端 `preview_terminal_config` 返回类型一致）。
  - `commit::apply(plan: &TerminalPlan, after_write: impl FnOnce() -> CoreResult<()>) -> CoreResult<AppliedPlan>`：
    1. 对每个已有文件复制到 `<name>.bak-<YYYYMMDDHHMMSS>`（同目录，保留）。
    2. 写入同目录临时文件，复制原文件权限（Unix `set_permissions`；Windows 只读属性），`fs::rename` 原子替换；不存在的父目录创建。
    3. 任一步失败或 `after_write`（保存数据库）失败 → 按逆序用备份恢复已写入的文件，删除新建的文件，返回原错误。
  - `ClientCtx { proxy_base: String /*http://host:port*/, providers: Vec<Provider>, models: Vec<Model>, standard_models: Vec<StandardModel>, active_engine: Option<Model>, home: PathBuf, env: HashMap<String, String> }`（`home` 与 `env` 可注入，便于快照测试）。
  - `trait ClientSpec { fn client(&self) -> Client; fn files(&self, ctx: &ClientCtx) -> Vec<(PathBuf, DocFormat)>; fn render_proxy(&self, docs: &mut [Doc], profile: &ProxyProfile, ctx: &ClientCtx) -> CoreResult<Vec<String> /*secrets*/>; fn render_direct(&self, docs: &mut [Doc], profile: &DirectProfile, ctx: &ClientCtx) -> CoreResult<Vec<String>>; }`；`terminal::build_plan(spec, mode, ctx) -> CoreResult<TerminalPlan>` 读文件 → `Doc::parse`（不存在则 `empty`）→ render → 生成 `FileChange`（内容不变的文件不进计划）。

- [ ] **Step 1:** 测试：`toml_set_preserves_comments`；`json_retain_removes_only_marked_entries`；`parse_error_does_not_write`（Review Focus 4：损坏的 JSON → Validation，文件内容与 mtime 不变）；`missing_file_is_created`；`apply_creates_backup`；`apply_rolls_back_on_db_failure`（`after_write` 返回 Err，所有文件恢复原内容、新建文件被删）；`apply_rolls_back_on_second_file_failure`（第二个目标路径为只读目录）；`preview_masks_secrets`。
- [ ] **Step 2–4:** 失败 → 实现 → 通过。
- [ ] **Step 5:** 提交 `feat(terminal): document abstraction, plan and atomic commit with rollback`。

### Task 20: 五个客户端规格

**Files:**
- Create: `core/src/terminal/clients/{mod,codex,claude_code,opencode,pi,deepseek_harness}.rs`
- Test: `core/tests/terminal_clients.rs`、`core/tests/snapshots/`（insta）

**参考:** `main:src-tauri/src/terminal/clients/*.rs`（字段、路径、协议后缀、模型元数据结构）、`main:src-tauri/src/terminal/direct.rs`、`refactor/backend-rust:src-tauri/src/terminal/snapshots/*.snap`（可作为对照，但标识必须换成 yi-token 的）。

**Interfaces:**
- Produces: 每个客户端一个实现 `ClientSpec` 的单元结构；`clients::spec_for(client: Client) -> &'static dyn ClientSpec`。
  - 共同规则：proxy 模式 base URL `{proxy_base}/clients/{client}{suffix}`（suffix 按协议：Responses/Chat 为 `/v1`，Anthropic 为空，具体以 yi-llm 各客户端为准）；API Key `yi-token`；所有自有条目使用 Global Constraints 中的标识；渲染前先 `retain_in` 删除旧的自有条目，再写入新条目。
  - **codex**：`config.toml` 中 `[model_providers.yitoken]`、顶层 `model_provider` / `model`；`yi-token-models.json` 模型元数据（结构照 yi-llm 的 `yi-models.json`）。direct：`[model_providers.yitoken_direct_<id>]`，name `yi-token direct:{id}:{name}`，`model_source = native` 时不写 `model` 与元数据。
  - **claude-code**：`settings.json` 的 `env.ANTHROPIC_BASE_URL`、`env.ANTHROPIC_AUTH_TOKEN = "yi-token"`、`env.ANTHROPIC_MODEL` 等与 `modelPicker`（照 yi-llm）。direct 写真实 base_url 和 key。
  - **opencode**：`provider.yitoken`（npm 包按协议选择，照 yi-llm）、`model`；direct：`provider.yitoken_direct_<id>`。
  - **pi**：`models.json` 的 provider `yitoken` 与 `settings.json` 默认模型；direct 同理。
  - **deepseek-harness**：`cordis.patch.yml`（desktop 与 web 两个 profile，存在哪个写哪个，都不存在时写 desktop）引用 `${YI_TOKEN_PROXY}`，`.credentials.yaml` 写 `YI_TOKEN_PROXY` 的值；direct 用 `YI_TOKEN_DIRECT_<ID>`。
  - 文件定位规则（`terminal/paths.rs`）照 spec 6.1，环境变量从 `ctx.env` 读取。

- [ ] **Step 1:** 对每个客户端写 insta 快照测试（`ctx.home` 为 tempdir）：
  - `{client}_proxy_fresh`（无原文件）。
  - `{client}_proxy_preserves_user_content`（原文件含用户手写条目与注释；TOML 断言注释仍在）。
  - `{client}_proxy_preserves_yi_llm_entries`（原文件含 yi-llm 的 `yi` / `yi_direct_*` / `yi-llm direct:` / `YI_LLM_PROXY` 条目，渲染后原样存在）。
  - `{client}_proxy_rerender_is_idempotent`（渲染两次结果相同）。
  - `{client}_direct_native` 与 `{client}_direct_provider`（codex、claude-code 两种 source 都测；其余客户端测 provider）。
  - `multi_protocol_clients_use_selected_protocol`（opencode / pi / deepseek 三种协议各一个快照）。
  - `env_overrides_config_path`（`CODEX_HOME`、`CLAUDE_CONFIG_DIR`、`OPENCODE_CONFIG`、`XDG_CONFIG_HOME`、`DSH_HOME`）。
- [ ] **Step 2:** 运行 `cargo insta test -p yi-core --test terminal_clients`，审阅每个新快照：逐项对照 yi-llm 同场景输出（把 yi-llm 标识替换为 yi-token 标识后应一致），确认后 `cargo insta accept`。
- [ ] **Step 3:** 提交 `feat(terminal): client specs for codex, claude-code, opencode, pi, deepseek-harness`。

### Task 21: 终端服务与命令

**Files:**
- Create: `core/src/services/terminal.rs`
- Modify: `src-tauri/src/commands.rs`
- Test: `core/tests/terminal_service.rs`

**参考:** `main:src-tauri/src/service/terminal.rs`、`main:src/features/terminal/types.ts`。

**Interfaces:**
- Produces:
  - `Services::get_terminal_profiles() -> TerminalProfilesView`（每个客户端的 proxy 配置、direct 配置、Auto 引擎；无记录时给默认值；形状对照前端 types.ts，新增 `route_mode` 与 `active_engine` 字段）。
  - `preview_terminal_config(profile: ProxyProfile) -> TerminalPreview`；`apply_terminal_config(profile) -> TerminalApplyResult`（`commit::apply` 的 `after_write` 中保存 `terminal_proxy_profiles` 并 `routes_changed()`）。
  - `preview_terminal_direct_config(profile: DirectProfile)`；`apply_terminal_direct_config(profile)`。
  - 命令注册上述 5 个。

- [ ] **Step 1:** 测试：`apply_proxy_writes_file_and_saves_profile`；`apply_failure_leaves_db_unchanged`；`preview_does_not_touch_disk`；`apply_direct_for_disabled_provider_is_validation`。
- [ ] **Step 2–4:** 失败 → 实现 → 通过。
- [ ] **Step 5:** 手动：在 `YI_TOKEN_HOME` 与 `HOME` 指向临时目录的情况下运行应用，为 codex 应用 proxy 配置，用真实 codex（如已安装）发一个请求经代理成功。
- [ ] **Step 6:** 提交 `feat(terminal): terminal service and commands`。

---

## 阶段 6：Auto

### Task 22: Auto 解析与引擎服务

**Files:**
- Modify: `core/src/proxy/resolve.rs`、`core/src/proxy/router.rs`（`/v1/models` 的 auto 分支）、`core/src/terminal/clients/*.rs`（auto 渲染）
- Create: `core/src/services/engines.rs`
- Test: `core/tests/proxy_auto.rs`、`core/tests/terminal_clients.rs`（追加快照）

**Interfaces:**
- Produces:
  - `resolve` 的 Client + auto 分支：取 `snap.engines[client]`；为 `None`、模型不存在、或其服务商未启用 → `CoreError::Conflict("请先在 yi-token 中为该终端选择当前引擎")`；否则 `Resolved { provider, model: Some(m), upstream_model: m.upstream_model, route_mode: Some(Auto) }`。入站模型名不参与匹配（任何名字都按引擎路由）。
  - `/clients/{client}/v1/models` 在 auto 时只返回 `Auto`。
  - 各客户端 `render_proxy` 在 `route_mode == Auto` 时只写一个模型 `Auto`；codex 的 `yi-token-models.json` 中 `Auto` 条目使用 `ctx.active_engine` 的能力参数，未选择时用模型目录默认值（`standard_models` 中与 `Auto` 无关，用 `Capabilities::default_for(Protocol::Responses)`，定义在 Task 3 的 `capabilities.rs`，本任务补充该函数）。
  - `Services::set_active_engine(client: Client, model_id: Option<String>) -> CoreResult<()>`（模型不存在 → NotFound；模型所属服务商未启用 → Validation；保存后 `routes_changed()`；**不改写终端文件**）；`Services::get_active_engines() -> Vec<ActiveEngineView { client, model_id, exposed_name, provider_name }>`。
  - 用量记录带上 `client` 与 `route_mode`（Task 17 的 pipeline 从 `Resolved` 取值，本任务补测试）。

- [ ] **Step 1:** 测试：
  - `auto_without_engine_is_409`。
  - `auto_routes_to_engine_regardless_of_model_name`（请求 `model: "Auto"` 与 `model: "whatever"` 都到同一上游，上游收到的 model 为引擎 upstream_model）。
  - `auto_cross_protocol`（终端 codex/responses，引擎为 anthropic 服务商）。
  - `auto_switch_affects_only_new_requests`（wiremock 让第一次流式请求慢速输出；中途 `set_active_engine` 改到另一服务商；第一条流仍从原上游完整输出，第二次请求到新上游）。
  - `deleting_engine_provider_resets_to_unselected`（Review Focus 5：删除服务商后 auto 请求返回 409；fixed 配置中引用它的模型返回 404；direct 配置随之删除）。
  - `usage_records_client_and_route_mode`。
  - `set_active_engine_does_not_touch_terminal_files`（应用配置后记录文件 mtime，切换引擎后 mtime 不变）。
  - 快照：`{client}_proxy_auto`（5 个客户端），`codex_auto_metadata_uses_engine_capabilities`，`codex_auto_metadata_without_engine_uses_defaults`。
- [ ] **Step 2–4:** 失败 → 实现 → 通过（`cargo insta review` 审阅新快照）。
- [ ] **Step 5:** 提交 `feat(auto): per-terminal engine routing`。

### Task 23: Auto 命令与前端入口

**Files:**
- Modify: `src-tauri/src/commands.rs`、`src/features/terminal/{api,types,hooks}.ts`、`src/features/terminal/components/TerminalPage.tsx`、`src/features/terminal/useTerminalDrafts.ts`、`src/shared/i18n/locales/{zh-CN,en-US}/terminal.ts`

**Interfaces:**
- Consumes: Task 22 的 `set_active_engine`、`get_active_engines`、`TerminalProfilesView.route_mode`。
- Produces: 命令 `set_active_engine(client, modelId)`、`get_active_engines()`；前端 hooks `useActiveEngines()`、`useSetActiveEngine()`（TanStack Query，成功后 invalidate `terminal-profiles` 与 `active-engines`）。

- [ ] **Step 1:** 确定界面入口位置（spec 第 11 节第 1 项）：阅读 `TerminalPage.tsx` 的现有布局，在代理模式区域中复用现有控件——路由模式用页面已有的分段/选择控件（如 `ProtocolSelector` 同款或 `Select`），"当前引擎"用 `src/shared/ui/Select.tsx`，选项为全部已启用服务商的模型（显示 exposed_name）。auto 时隐藏 fixed 的模型列表编辑区。切换引擎时用现有 `Toast` 显示"已切换，之后的新请求生效"；若新旧引擎 `context_window` 不同，附加提示"如上下文长度差异较大，可重新应用终端配置"（spec 7.3）。把位置决定写进提交说明。
- [ ] **Step 2:** 实现；只使用已有 className 与组件，不新增 CSS 规则。运行 `npm run check:theme`、`npm run typecheck`、`npm run lint`。
- [ ] **Step 3:** 在 `tests/mock-backend.ts` 中加入 `get_active_engines`、`set_active_engine` 的 mock 与 `route_mode` 字段；新增 Playwright 用例 `terminal-auto.spec.ts`：切换到 Auto → 选择引擎 → 显示 Toast；fixed 模型列表隐藏。
- [ ] **Step 4:** `npm run test:ui` 通过。
- [ ] **Step 5:** 提交 `feat(ui): auto route mode and active engine selection`。

---

## 阶段 7：备份

### Task 24: `.yitoken` 导出与导入

**Files:**
- Create: `core/src/backup/{mod,crypto,payload,import}.rs`、`core/src/services/backup.rs`
- Modify: `src-tauri/src/commands.rs`（`export_provider_config`、`import_provider_config`，文件对话框用 `rfd`，过滤器 `yi-token 备份 (*.yitoken)`）、`src/features/providers/components/ProviderConfigTransfer.tsx`（仅扩展名与文案）、i18n 文案
- Test: `core/tests/backup.rs`

**参考:** `main:src-tauri/src/backup/{mod,crypto}.rs`、`main:src-tauri/src/backup/tests.rs`。

**Interfaces:**
- Produces:
  - `crypto::encrypt(plaintext: &[u8], passphrase: &SecretString, header_aad: &[u8], iterations: u32) -> CoreResult<(salt, nonce, ciphertext)>`；`crypto::decrypt(envelope: &Envelope, passphrase: &SecretString) -> CoreResult<Zeroizing<Vec<u8>>>`；派生密钥放 `Zeroizing<[u8; 32]>`。
  - `Envelope { format, version, kdf: Kdf { algorithm, iterations, salt }, cipher: Cipher { algorithm, nonce }, payload }`，`#[serde(deny_unknown_fields)]`；AAD = 去掉 `payload` 后的信封按固定字段顺序序列化的 JSON 字节。
  - `payload::Payload`（spec 8.2）；`backup::export(db, passphrase, title) -> CoreResult<Vec<u8>>`。
  - `import::merge(tx, payload) -> CoreResult<ImportReport { providers: usize, standard_models: usize, models: usize, terminal_profiles: usize, skipped: Vec<String> }>`：在单个事务内按 spec 8.3 合并；冲突 → Conflict 并回滚整个事务。
  - 校验：口令 < 8 → Validation；`format`/`version` 不符 → Validation("不是 yi-token 备份文件或版本不受支持")；迭代次数越界 → Validation；解密失败 → Validation("口令错误或文件已损坏")。
  - 服务：导入成功后 `routes_changed()`；**不改写终端文件**。

- [ ] **Step 1:** 测试：`export_import_roundtrip`（导出 → 清空数据库 → 导入 → 数据一致，含 API Key、终端配置、引擎、log_level；不含 host/port）；`wrong_passphrase_fails`；`tampered_ciphertext_fails`；`tampered_header_fails`（改 `iterations` 或 `format` 中的一个字符 → 解密失败，验证 AAD）；`short_passphrase_rejected`；`iterations_out_of_range_rejected`；`short_code_conflict_rolls_back`（导入前插入一个 `short_code` 相同但 id 不同的服务商；导入失败后数据库与导入前逐表相等）；`merge_keeps_unlisted_providers`；`terminal_entries_with_missing_refs_are_skipped`；`import_does_not_touch_terminal_files`。
- [ ] **Step 2–4:** 失败 → 实现 → 通过。
- [ ] **Step 5:** 手动：应用内导出、导入一次 `.yitoken` 文件。
- [ ] **Step 6:** 提交 `feat(backup): encrypted .yitoken export and merge import`。

---

## 阶段 8：验收

### Task 25: 界面一致性

**Files:**
- Modify: `playwright.config.mjs`（如需调整 baseURL / 端口）、`tests/mock-backend.ts`（与新 DTO 对齐：string ID、新增字段）
- Create: `tests/visual-baseline/README.md`（基线生成步骤）

- [ ] **Step 1:** 在 yi-llm 的临时 worktree（`git -C D:/Github/yi-llm worktree add ../yi-llm-ui main`）中运行 `npm ci && npx playwright test tests/zz-visual.spec.ts --update-snapshots`，得到 v0.1.8 的截图基线；拷入 yi-token 的 `tests/zz-visual.spec.ts-snapshots/`。完成后移除 worktree。
- [ ] **Step 2:** 在 yi-token 运行 `npm run test:ui`。允许的差异只有：产品名文案、Auto 相关控件（只出现在终端页的 auto 状态截图里，fixed 状态截图必须一致）。其余差异视为回归，修复前端改动直到一致。
- [ ] **Step 3:** 提交 `test(ui): visual parity with yi-llm v0.1.8`。

### Task 26: 全量验收与发布配置

**Files:**
- Create: `README.md`（中文，说明与 yi-llm 的关系、数据目录、Auto 用法、共存说明）、`.github/workflows/ci.yml`（可选：fmt、clippy、test、typecheck、lint、test:ui）
- Modify: `src-tauri/tauri.conf.json`（bundle 信息）

- [ ] **Step 1:** 逐节核对 spec：为 spec 的每一节写出对应任务号（写在 PR 描述里）；发现遗漏就补任务后再继续。
- [ ] **Step 2:** 运行完整验收：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`、`npm run typecheck`、`npm run lint`、`npm run check:theme`、`npm run test:ui`、`npm run tauri build`。全部通过，记录输出。
- [ ] **Step 3:** 端到端手动冒烟（在独立的 `YI_TOKEN_HOME` 与 `HOME` 下）：
  1. 新建三种协议的服务商各一个；
  2. codex 以 fixed 应用配置并成功请求；
  3. 切换为 Auto，选引擎 A 请求成功，切到引擎 B 请求成功，用量页按终端显示两条；
  4. claude-code direct 模式应用后配置中是真实 base_url；
  5. 同机已有 yi-llm 写入的 codex 配置在以上操作后仍完整；
  6. 导出 `.yitoken`，删除数据目录，重启后导入，服务商与终端配置恢复。
- [ ] **Step 4:** 提交 `docs: readme and release configuration`。

---

## 自检记录

- **Spec 覆盖：** §1/§2 → Task 1、3；§3 → Task 1、2、7、8；§3.4 → Task 2；§4 → Task 3、4、6；§4.4 → Task 7、9；§5.1 → Task 16；§5.2 → Task 10–14、17；§5.3 → Task 10、17；§5.4 → Task 15、17；§5.5 → Task 13；§5.6 → Task 15、16、18；§5.7 → Task 7、18；§6 → Task 19–21；§7 → Task 22、23；§8 → Task 24；§9.1 → Task 8、18、21、23、24；§9.2 → Task 9、23；§10.1 B1 → Task 10/17，B2 → Task 4，B3 → Task 11，B4 → Task 10/17，B5 → Task 15/17，B6 → Task 13/14；§10.2 → Task 14、17、20、24、25、26；§11 第 1 项 → Task 23 Step 1，第 2 项 → 维持不保留注释（Task 19）。
- **类型一致性：** `Protocol`、`Client`、`RouteMode`、`ProxyProfile`、`DirectProfile`、`Resolved`、`RouteSnapshot`、`UsageEvent`、`UsageSink`、`StreamTranslator`、`OutboundCtx`、`PrepareCtx`、`TerminalPlan`、`ClientCtx` 在定义任务与使用任务中名称一致。
- **Review Focus：** 五项分别落在 Task 16、17、15、19、22 的测试中。
