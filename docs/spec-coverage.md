# 设计规格覆盖对照

本文档把 `docs/superpowers/specs/2026-10-25-yi-token-design.md` 的每个章节映射到
实现计划的任务与验证位置，便于审阅时逐项核对。任务编号对应
`docs/superpowers/plans/2026-10-25-yi-token-implementation.md`。验证位置写的是
仓库中实际存在的测试文件与测试函数；`*.rs` 内联单元测试标注所在模块。

## 章节映射

| 规格章节 | 内容 | 任务 | 验证 |
| --- | --- | --- | --- |
| §1 | 目标、范围、三项扩展、不包含项 | 1、3、22–24 | README 功能说明；`proxy_auto.rs`、`backup.rs` |
| §2 D1–D15 | 从零实现、范围、前端不变、数据不兼容、workspace、技术栈、备份扩展名、Auto 约束、终端身份、模型名、候选引擎、thinking、备份合并、与 yi-llm 共存 | 1、2、3、8、9、19–24 | `core/tests/terminal_*.rs`、`backup.rs`、`proxy_auto.rs`；视觉基线 |
| §3.1 | 目录结构 | 1 | workspace 与 `core/src/*` 目录 |
| §3.2 | 依赖规则（core 不依赖 tauri、proxy 走 RouteSource、services 门面） | 1、2、7、8、16 | `core/Cargo.toml`、`core/src/proxy/resolve.rs`、`src-tauri/src/commands.rs` |
| §3.3 | rusqlite + r2d2、WAL、外键、`unsafe_code = "forbid"` | 2、4 | `Cargo.toml`、`core/src/storage/mod.rs` |
| §3.4 | 数据目录、数据库、日志、模型目录变量 | 2、5、7 | `core/src/paths.rs`、`core/tests/logging_level.rs`、`core/tests/catalog.rs` |
| §4.1 | `user_version` 与迁移同事务 | 4 | `core/tests/storage_repos.rs`：`migration_sets_user_version_1`、`migration_is_atomic` |
| §4.2 | 各表结构与约束 | 4 | `core/src/storage/migrations.rs`；`core/tests/storage_repos.rs` |
| §4.3 | JSON 列原则 | 4 | schema 与 `core/src/domain/` |
| §4.4 | 前端 ID 由 number 改为 string | 7、9 | `ts-rs` 生成类型 `src/generated/`；`tests/mock-backend.ts` |
| §4.5 | 有界队列、批量写入、终端聚合 | 6、7 | `core/src/storage/usage_writer.rs`（内联）；`core/tests/services_basic.rs` |
| §5.1 | 全局与终端 HTTP 路由、请求体上限、鉴权头处理、未知 client | 16、17 | `core/tests/proxy_resolve.rs`（含 `large_body_is_not_rejected`）；`core/tests/proxy_pipeline.rs`（含 `large_body_reaches_upstream_through_client_route`） |
| §5.2 | 上游协议选择、直通/转换、未知字段与内容块、SSE 签名透传、`store` 拒绝 | 10–14、17 | `core/src/protocol/`（内联）；`core/tests/oracle.rs`；`core/tests/proxy_pipeline.rs` |
| §5.3 | SSE 按字节分帧、统一解析用量、断连取消 | 10–12、17 | `core/src/protocol/sse.rs`（`sse_decoder_handles_every_split_point`）；`proxy_pipeline.rs`（`client_disconnect_cancels_upstream`） |
| §5.4 | 上游错误透传与本地错误状态码 | 15、17 | `core/tests/proxy_lifecycle.rs`（`conflict_maps_to_409`）；`core/tests/proxy_pipeline.rs` |
| §5.5 | `thinking` 兜底推理强度与能力判断 | 13 | `core/src/protocol/prepare.rs`、`core/src/protocol/thinking.rs`（内联）；`core/tests/oracle.rs` |
| §5.6 | 路由快照热更新、监听重启、优雅关闭 | 15、16、18、22 | `core/tests/proxy_resolve.rs`；`core/tests/proxy_lifecycle.rs` |
| §5.7 | 监控采样、日志滚动与尾部读取、运行时调级 | 18 | `core/tests/proxy_monitor.rs`；`core/tests/logging_level.rs` |
| §6.1 | 五个客户端、协议与配置文件 | 20 | `core/tests/terminal_clients.rs`；`core/tests/snapshots/` |
| §6.2 | proxy / direct 两种模式 | 19、20、21、23 | `core/tests/terminal_clients.rs`、`terminal_service.rs` 快照与回滚 |
| §6.3 | 所有权标识与只改 yi-token 条目 | 20、21 | `core/tests/terminal_clients.rs` 中“用户手写内容保留”“yi-llm 条目保留”快照 |
| §6.4 | Doc 抽象、原子提交、备份回滚、预览脱敏 | 19 | `core/tests/terminal_commit.rs`；`core/src/terminal/commit.rs`、`doc.rs`（内联） |
| §7.1 | Auto 定位、候选、终端独立、配置不改写 | 22、23 | `core/tests/proxy_auto.rs`；`tests/terminal-auto.spec.ts` |
| §7.2 | Auto 请求流程、409、用量字段 | 22 | `core/tests/proxy_auto.rs`（`auto_without_engine_is_409`）；`core/tests/proxy_resolve.rs` |
| §7.3 | Codex 模型元数据与切换提示 | 23 | `core/tests/terminal_clients.rs` 快照（`codex_auto_metadata_*`）；`tests/terminal-auto.spec.ts` |
| §8.1 | `.yitoken` 信封、PBKDF2、AES-GCM、AAD、口令校验 | 24 | `core/tests/backup.rs`：`wrong_passphrase_fails`、`tampered_ciphertext_fails`、`tampered_header_fails`、`short_passphrase_rejected` |
| §8.2 | 备份载荷字段 | 24 | `core/tests/backup.rs` 往返用例 |
| §8.3 | 全量校验后单事务合并、冲突回滚、跳过无效终端引用、不改写文件 | 24 | `core/tests/backup.rs`：`short_code_conflict_rolls_back` |
| §9.1 | 23 个原有命令、2 个 Auto 命令、错误结构 | 8、18、21–24 | `src-tauri/src/lib.rs`；`core/tests/services_basic.rs`（如 `save_provider_duplicate_short_code_is_conflict`） |
| §9.2 | `ts-rs` 类型生成、仅改数据接入层、Auto 复用现有样式 | 9、23 | `npm run typecheck`；视觉基线 |
| §10.1 B1 | SSE 多字节 UTF-8 跨包切分 | 10、17 | `core/src/protocol/sse.rs`（`sse_decoder_handles_multibyte_split`）；`proxy_pipeline.rs`（`stream_chinese_split_across_chunks`） |
| §10.1 B2 | 迁移与版本号同事务 | 4 | `core/tests/storage_repos.rs`（`migration_is_atomic`） |
| §10.1 B3 | 未知 Anthropic 内容块保留 | 11、14 | `core/src/protocol/anthropic/`（内联）；`core/tests/oracle.rs` |
| §10.1 B4 | `encrypted_content` 透传 | 10、17 | `core/tests/proxy_pipeline.rs`（`encrypted_content_passthrough`） |
| §10.1 B5 | 本地错误映射到正确状态码 | 15、17 | `core/tests/proxy_lifecycle.rs`（`conflict_maps_to_409`）；`proxy_pipeline.rs` |
| §10.1 B6 | `thinking` 在代理路径生效 | 13、14 | `core/src/protocol/prepare.rs`（内联）；`core/tests/oracle.rs` 白名单 |
| §10.2 单元 | core 内存 SQLite、协议、路由、终端测试 | 4、10–17、19–24 | `cargo test --workspace` |
| §10.2 基准 | 请求侧语义比对（规格原文：42 字段 × 37 边界 × 3 上游；Task 14 按 yi-llm 实测值域为 32，见实施记录） | 14 | `core/tests/oracle.rs` |
| §10.2 集成 | wiremock、三协议互转/直通、流式、Auto | 14、17、22 | `core/tests/proxy_auto.rs`、`proxy_lifecycle.rs`、`proxy_pipeline.rs`、`proxy_resolve.rs`、`proxy_monitor.rs` |
| §10.2 终端快照 | `insta` 保留用户与 yi-llm 内容 | 20 | `core/tests/terminal_clients.rs`；`core/tests/snapshots/` |
| §10.2 备份 | 往返、错误口令、篡改、冲突回滚 | 24 | `core/tests/backup.rs` |
| §10.2 界面 | Playwright 与 yi-llm 截图对齐 | 25 | `tests/zz-visual.spec.ts`；基线说明 `tests/visual-baseline/README.md` |
| §10.2 验收 | fmt、clippy、test、tsc、lint | 26 | 下方命令 |
| §11 第 1 项 | Auto 界面入口位置 | 23 | 终端页面“路由模式 / 当前引擎” |
| §11 第 2 项 | JSON/JSONC/YAML 不保留注释 | 19 | 终端快照明确记录该行为 |

## 有意行为差异

以下差异来自规格扩展，不是回归：

- Auto 路由与 `route_mode` 字段、`set_active_engine` / `get_active_engines` 命令。
- 代理路径上 `thinking` 作为兜底推理强度生效。
- `.yitoken` 备份包含终端配置与 `log_level`，导入只写数据库。
- `ModelMapping.id` 与 `UsageRecord.id` 在前端为 string。
- 视觉基线中隐藏 Auto 新增的“路由模式”步骤，其余布局与 yi-llm v0.1.8 对齐。

## 验收命令

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm run typecheck
npm run lint
npm run check:theme
npm run test:ui
npm run tauri build
```
