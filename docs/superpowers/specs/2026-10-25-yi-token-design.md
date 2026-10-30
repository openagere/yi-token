# yi-token 设计文档

- 日期：2026-10-25
- 状态：已实施（v0.2.0）。测试对照见 `docs/spec-coverage.md`。
- 参考实现：`D:\Github\yi-llm`（v0.1.8，`main` 分支）；另参考其 `refactor/backend-rust` 分支的缺陷修复与测试用例

## 1. 目标与范围

从零实现一个本地多协议 LLM 网关与终端管理桌面应用 **yi-token**。功能与 yi-llm v0.1.8 对齐，后端 Rust 代码按本文档重新设计。

**包含**
- Tauri 桌面应用，前端 React，界面、样式、组件与 yi-llm 保持一致。
- 本地代理：三种入站协议（Anthropic Messages、OpenAI Chat、OpenAI Responses），之间互相转换；同协议直通。
- 服务商、标准模型、模型的管理；内置模型目录；模型用量统计；日志与监控。
- 五个终端客户端的配置写入（codex、claude-code、opencode、pi、deepseek-harness），支持代理模式与直连模式。
- 加密导入导出，文件扩展名 `.yitoken`。

**相对 v0.1.8 的扩展**
- 新增 Auto 路由模式（第 7 节）。
- 服务商 `thinking` 字段真正生效（第 5.5 节；v0.1.8 中该字段在代理路径上被清空，实际无效）。
- 备份载荷新增终端配置与设置（第 8 节）。

**不包含**
- 不兼容 yi-llm 的任何数据：不读取 v0.1.8 的 SQLite 数据库，不导入 `.yillm` 备份。
- 不做故障自动切换。上游报错时原样返回，由用户手动切换引擎。
- 不改变前端的视觉样式。
- 不引入托盘、开机自启、自动更新、单实例等 v0.1.8 没有的插件。

## 2. 已确定的决策

| # | 决策 | 结论 |
| --- | --- | --- |
| D1 | 起点 | 从零实现，不复制 yi-llm 的后端代码；yi-llm 仅作行为参考 |
| D2 | 范围 | 完整复刻 v0.1.8 的功能，外加第 1 节列出的三项扩展 |
| D3 | 前端 | 样式、组件、布局不变；只调整数据接入层以匹配新的后端协议 |
| D4 | 数据兼容 | 不兼容旧数据库与旧备份 |
| D5 | 后端结构 | Cargo workspace：`core`（纯 Rust）+ `src-tauri`（Tauri 壳） |
| D6 | 技术栈 | 沿用 Tauri 3 alpha、React 19、Vite 8；Rust 依赖沿用 rusqlite、axum、reqwest 等 |
| D7 | 备份扩展名 | `.yitoken` |
| D8 | Auto 切换触发 | 用户手动切换，不做自动故障转移 |
| D9 | Auto 作用范围 | 每个终端独立选择当前引擎 |
| D10 | 终端身份识别 | 按路径 `/clients/{client}` 识别，不引入令牌；配置中的 API Key 字段写固定占位值 `yi-token` |
| D11 | Auto 模型名 | 终端配置中只暴露一个模型 `Auto` |
| D12 | Auto 候选引擎 | 所有已启用服务商下的全部模型 |
| D13 | `thinking` | 作为兜底推理强度生效；请求显式携带推理参数时以请求为准 |
| D14 | 备份导入 | 合并，不覆盖；终端配置只入库，不自动改写终端文件 |
| D15 | 与 yi-llm 共存 | 数据目录、所有权标识、环境变量名全部与 yi-llm 区分，两者可同机安装互不干扰 |

## 3. 架构

### 3.1 目录结构

```
yi-token/
  Cargo.toml              # workspace 根，成员：core、src-tauri
  models/catalog.json     # 内置模型目录（含 schema）
  core/                   # yi-core：纯 Rust 库，不依赖 Tauri
    src/
      domain/             # Provider、Model、StandardModel、Protocol、Client、Capabilities 等类型与校验
      storage/            # SQLite 连接池、迁移、各表 repo
      protocol/           # 协议转换：Anthropic / OpenAI Chat / Responses，直通预处理
      proxy/              # axum 路由、上游转发、SSE、用量统计、监控
      terminal/           # 终端配置规格与写入
      catalog/            # 模型目录加载与热更新
      backup/             # 加密导入导出（.yitoken）
      services/           # 对外门面
      error.rs
  src-tauri/              # 应用 crate，保留 Tauri 默认约定（tauri.conf.json 在此）
    src/
      lib.rs              # Tauri Builder，注册命令
      commands.rs         # #[tauri::command] 薄封装
      state.rs            # 把 services 装入 Tauri State
  src/                    # React 前端
```

### 3.2 依赖规则

- `core` 不依赖 `tauri`，可独立 `cargo test -p yi-core`。
- `src-tauri` 只依赖 `core::services`，不直接访问 `storage`、`protocol` 等内部模块。
- `proxy` 不直接依赖 `storage`，通过 `RouteSource` trait 获取路由快照，由 `storage` 实现。
- `services` 是唯一的公开门面。

### 3.3 技术约束

- SQLite 使用 `rusqlite`（bundled）+ `r2d2` 连接池，启用 WAL、外键约束。同步调用通过 `spawn_blocking` 在 `services` 层包装。
- 行映射用 `serde_rusqlite` + 显式 Row 结构。不引入 ORM。
- 代码中禁止 `unsafe`（`unsafe_code = "forbid"`）。

### 3.4 运行时目录与环境变量

| 项 | 取值 |
| --- | --- |
| 数据目录 | `$YI_TOKEN_HOME`，未设置时为 `~/.yi-token` |
| 数据库 | `<数据目录>/yi-token.db` |
| 日志 | `<数据目录>/logs/yi-token.log`，按天滚动 |
| 模型目录覆盖 | `$YI_TOKEN_MODEL_CATALOG` 指向的文件，未设置时用内置 `models/catalog.json` |

Tauri 应用标识符也与 yi-llm 不同，但数据目录不依赖应用标识符。

## 4. 数据层

### 4.1 版本管理

- `PRAGMA user_version` 从 1 起算，迁移用 `rusqlite_migration`，每个迁移与版本号在同一事务中提交。
- 不保留 legacy 升级路径。

### 4.2 表结构

所有 ID 为 TEXT（UUID v4，由后端生成）。时间为 UTC 毫秒整数。

| 表 | 说明 |
| --- | --- |
| `providers` | `id` PK；`type`（原生协议）；`name`；`short_code` 唯一（`COLLATE NOCASE`）；`base_url`、`api_key`、`enabled`、`thinking`（`minimal`/`low`/`medium`/`high`/`xhigh`，可空）、`is_default`、`sort_order`；`extra` 为 JSON |
| `provider_protocols` | `(provider_id, protocol)` 复合主键，外键级联删除；原生协议必含 |
| `standard_models` | `id` PK；`(name, protocol)` 唯一；`brand`；`capabilities` 为 JSON |
| `models` | `id` PK；`provider_id` 外键级联删除；`standard_model_id` 外键 RESTRICT；`exposed_name` 唯一（固定为 `{upstream_model}({short_code})`）；`upstream_model`；`capabilities` 为 JSON；`non_standard` 布尔 |
| `terminal_proxy_profiles` | `client` PK；`protocol`（多协议客户端由用户选择）；`route_mode` ∈ {`fixed`, `auto`}；`models` JSON（`[{provider_id, model}]`，仅 fixed 使用）；`default_model` |
| `terminal_direct_profiles` | `client` PK；`provider_id` 外键级联删除；`model_source` ∈ {`native`, `provider`}；`model`（可空） |
| `terminal_engines` | `client` PK；`model_id` 外键 → `models`，`ON DELETE SET NULL`。Auto 当前引擎；引擎模型被删后该终端回到"未选择" |
| `settings` | 单行，`CHECK (id = 1)`；`host`（默认 `127.0.0.1`）、`port`（默认 `11435`）、`log_level`（默认 `info`） |
| `usage` | `id` PK；`requested_at` 建索引；`client`（可空，全局路由为空）；`route_mode`；`provider_name`、`protocol`、`model`；`input_tokens`、`output_tokens`、`cached_tokens`、`reasoning_tokens` |

### 4.3 JSON 列原则

- 只有无固定结构或整体读写的内容用 JSON：`capabilities`、`extra`、`terminal_proxy_profiles.models`。
- 其余字段都是具体列，能加索引和约束。

### 4.4 与前端的类型差异

- v0.1.8 中 `ModelMapping.id`、`UsageRecord.id` 为 number，本设计改为 string。这是明确的前端契约变更，由 `ts-rs` 生成的类型在编译期暴露所有受影响位置。
- 其余 ID（服务商、标准模型）在 v0.1.8 中已是 string，不变。

### 4.5 用量写入

- 代理通过有界队列（容量 4096）把用量记录交给独立写入线程，按批（256 条）落库。队列满时丢弃并记 warn 日志，不阻塞请求。
- 聚合维度：按日、按模型、按日×模型；新增按终端（`client`）。

## 5. 协议层与代理层

### 5.1 HTTP 路由

| 路由 | 说明 |
| --- | --- |
| `GET /health` | 健康检查 |
| `GET /v1/models`、`POST /v1/messages`、`/v1/chat/completions`、`/v1/responses`、`/responses` | 全局路由：按 `exposed_name` 精确匹配，要求服务商支持入站协议；匹配不到时交给默认服务商，模型名原样传递 |
| `GET /clients/{client}/v1/models`、`POST /clients/{client}/v1/{messages,chat/completions,responses}`、`/clients/{client}/responses` | 终端路由：只按该终端的代理配置解析（fixed 或 auto） |

- 所有推理路由不设请求体大小上限（Codex 等客户端请求体可能远超 2 MiB）。
- 本地代理不做鉴权。入站的 `authorization`、`x-api-key`、cookie 头被剥离，替换为服务商凭据。
- 未知 `client` 返回 404。

### 5.2 上游协议选择与转换

- **上游协议选择**：服务商支持入站协议时使用入站协议（直通）；否则使用服务商的原生协议（转换）。
- **直通**：请求体原样转发，仅经过预处理（模型名替换、`thinking` 兜底、非标准模型的兼容适配）。响应原样回传，同时扫描 SSE 提取用量。
- **转换**：经过 Responses 中间形态。每种协议只实现"到中间形态"和"从中间形态出来"两个方向，覆盖全部 3×3 组合。
- 入站请求解析为类型化结构体。未知字段通过 `#[serde(flatten)]` 保留，用于 `reasoning_content`、namespace tools、`custom_tool_call` 等厂商扩展。
- 未知内容块（如 Anthropic 的 `server_tool_use`、`citations_delta`）使用兜底变体保留，不丢弃。
- 上游 SSE 中的 `encrypted_content` 等签名字段原样透传。
- 转换到非 Responses 上游时，拒绝 `store=true` 与 `previous_response_id`（返回 400，提示该上游不支持服务端会话状态）。

### 5.3 流式处理

- 用 `eventsource-stream` 分帧。先按字节缓冲，完整事件后再做 UTF-8 解码，避免多字节字符跨 TCP 包切分时被替换成 U+FFFD。
- 下游输出使用 axum 的 `Sse` 类型。
- 用量提取与转发共用同一次解析，不重复扫描 SSE。
- 客户端断开时取消上游请求。

### 5.4 上游请求与错误

- 统一的上游 HTTP 模块：发送、超时、代理设置。
- 上游错误为结构体 `UpstreamError { status, body }`，原样返回上游状态码与响应体。
- 本地错误映射：校验错误 → 400；未找到 → 404；冲突（如 Auto 未选择引擎）→ 409；其余 → 500。

### 5.5 `thinking` 兜底推理强度

仅当请求未携带任何推理参数时生效：

| 上游协议 | 写入字段 | 映射 |
| --- | --- | --- |
| Anthropic | `thinking.budget_tokens` | low 4096 / medium 10000 / high 32000 / xhigh 64000，并限制在 `max_tokens` 以下；`minimal` 不开启 |
| OpenAI Chat | `reasoning_effort` | 直接使用强度值 |
| Responses | `reasoning.effort` | 直接使用强度值 |

模型能力（`capabilities`）声明不支持推理时不写入。

### 5.6 路由状态与生命周期

- 路由快照用 `arc-swap` 原子替换。服务商、模型、终端配置、Auto 引擎、模型目录变更后立即生效，无需重启代理。
- 只有修改 `host` / `port` 时重启监听。启停通过 `oneshot` 通道优雅关闭。

### 5.7 监控与日志

- 监控：每 5 秒采样一次，保留 721 个点（约 1 小时）的时序数据；另按（客户端，协议）统计路由数据。数据由请求日志中间件喂入。
- 日志：`tracing` 输出到按天滚动的日志文件，级别（error/warn/info/debug/trace）可运行时调整。`get_logs` 返回日志文件尾部（上限 512 KiB / 1000 行）。

## 6. 终端配置

### 6.1 客户端清单

| 客户端 | 协议 | 配置文件 |
| --- | --- | --- |
| `codex` | responses | `$CODEX_HOME` 或 `~/.codex/config.toml`（TOML），另写模型元数据 `yi-token-models.json` |
| `claude-code` | anthropic | `$CLAUDE_CONFIG_DIR` 或 `~/.claude/settings.json`（JSON） |
| `opencode` | openai_chat、responses、anthropic | `$OPENCODE_CONFIG`，否则 `$XDG_CONFIG_HOME` 或 `~/.config` 下的 `opencode/opencode.json(c)`（JSONC） |
| `pi` | openai_chat、responses、anthropic | `~/.pi/agent/models.json` 与 `settings.json`（JSON） |
| `deepseek-harness` | openai_chat、responses、anthropic | `$DSH_HOME` 或 `~/.dsh/profiles/{desktop,web}/cordis.patch.yml` 与 `.credentials.yaml`（YAML） |

具体字段在实现计划阶段逐个对照 yi-llm 的 `terminal/clients/*` 确认。

### 6.2 两种模式

- **proxy**：配置指向 `http://{host}:{port}/clients/{client}{协议后缀}`，API Key 写 `yi-token`。路由方式为 `fixed` 或 `auto`（第 7 节）。
- **direct**：直接写入服务商的真实 `base_url` 与 API Key，不经过代理。`model_source = native` 保留客户端自带的模型选择；`provider` 写入该服务商的全部上游模型，启动模型取 `model`，未设置时取第一个。

### 6.3 所有权标识（与 yi-llm 区分）

| 场景 | yi-llm 使用 | yi-token 使用 |
| --- | --- | --- |
| proxy 模式 provider id | `yi` | `yitoken` |
| direct 模式 provider id | `yi_direct_<id>` | `yitoken_direct_<id>` |
| direct 模式名称前缀 | `yi-llm direct:` | `yi-token direct:` |
| deepseek 环境变量引用 | `YI_LLM_PROXY`、`YI_LLM_DIRECT_*` | `YI_TOKEN_PROXY`、`YI_TOKEN_DIRECT_*` |

**硬性约束**：只修改和删除带 yi-token 标识的条目。用户手写的内容和 yi-llm 写入的条目必须原样保留。

### 6.4 写入机制

- 每个客户端由声明式 `ClientSpec` 描述：配置路径、格式、需要写入的字段。
- 统一文档抽象 `enum Doc { Toml(DocumentMut), Json(Value), Yaml(Value) }`。TOML 经 `toml_edit` 编辑，保留注释与格式；JSON / JSONC / YAML 按值读写，注释不保留（与 yi-llm 一致，见第 11 节）。
- 统一提交流程（proxy 与 direct 共用）：生成计划 → 备份原文件到同目录 `<name>.bak-XXXX` → 临时文件写入后原子替换（保留文件权限）→ 任一步失败按逆序回滚，并且不保存数据库中的配置。
- 预览与真实写入共用渲染函数，仅对 API Key 等敏感值脱敏。

## 7. Auto 路由模式

### 7.1 定位

- Auto 是 proxy 模式下的路由方式，与 `fixed` 并列。直连模式不受影响。
- 终端配置中只暴露一个模型 `Auto`。切换引擎时终端配置文件不改写，终端的模型列表保持不变。
- 候选引擎：所有已启用服务商下的全部模型。每个终端独立选择，存于 `terminal_engines`。
- 切换引擎只影响之后到达的新请求；请求开始时锁定路由快照，进行中的请求不受影响。

### 7.2 请求流程

1. 从路径取出 `client`，读取其代理配置。
2. `fixed`：按配置的模型列表解析请求中的模型名。
3. `auto`：读取当前引擎。未选择，或引擎所属服务商已停用时，返回 409，提示"请先在 yi-token 中为该终端选择当前引擎"。
4. 请求体中的模型名替换为引擎的上游模型名。
5. 按第 5.2 节选择上游协议，直通或转换。
6. 上游报错原样返回，不切换、不重试。用量记录写入 `client`、`route_mode = auto` 以及实际服务商与模型。

### 7.3 模型元数据

- 终端会根据模型名读取元数据（上下文长度、推理能力等）。Codex 的 `yi-token-models.json` 中，`Auto` 条目使用**应用配置时**当前引擎的能力参数；尚未选择引擎时用模型目录的默认值。
- 之后切换引擎不重写该文件，元数据可能与实际引擎不一致。这是 Auto 的已知代价，界面上切换引擎时提示"如上下文长度差异较大，可重新应用终端配置"。

## 8. 备份（.yitoken）

### 8.1 格式

- 信封：`{ format: "yi-token.backup", version: 1, kdf: { algorithm: "pbkdf2-hmac-sha256", iterations, salt }, cipher: { algorithm: "aes-256-gcm", nonce }, payload }`，`deny_unknown_fields`。
- PBKDF2-HMAC-SHA256，默认 600,000 次，导入接受 10,000 至 10,000,000；salt 16 字节，nonce 12 字节。
- 不含 `payload` 的信封头 JSON 作为 AAD，篡改信封头会导致解密失败。
- 口令至少 8 个字符。派生密钥与口令使用 `zeroize` 清零。

### 8.2 载荷

`{ exported_at, app_version, standard_models, providers（含模型与 API Key）, terminal_proxy_profiles, terminal_direct_profiles, terminal_engines, settings（仅 log_level） }`。

### 8.3 导入

- 先完整解密并校验整个载荷，全部通过后在一个事务中写入。任何一步失败都不留下半导入的数据。
- 合并语义：
  - 服务商、标准模型、模型按 `id` 插入或更新；备份中没有的记录保持不动。
  - `short_code` 或 `exposed_name` 与现有其他记录冲突时整体失败，提示冲突项。
  - 终端配置与引擎按 `client` 插入或更新，只写数据库，**不改写终端配置文件**，由用户手动点击"应用"。
  - 引用了不存在的服务商或模型的终端条目跳过，并在结果中列出。

## 9. 接口层与前端契约

### 9.1 命令层

- `commands.rs` 只做参数转换、调用 `services`、映射错误。
- 沿用 v0.1.8 的 23 个命令，名称与参数不变：
  - 服务商：`list_providers`、`save_provider`、`delete_provider`、`test_provider`、`export_provider_config`、`import_provider_config`
  - 模型：`list_standard_models`、`get_model_catalog_info`、`save_standard_model`、`delete_standard_model`
  - 设置：`get_settings`、`save_settings`
  - 代理：`get_proxy_status`、`get_proxy_runtime`、`get_proxy_monitor`、`control_proxy`
  - 日志与用量：`get_logs`、`get_usage`
  - 终端：`get_terminal_profiles`、`preview_terminal_config`、`apply_terminal_config`、`preview_terminal_direct_config`、`apply_terminal_direct_config`
- 新增：`set_active_engine(client, model_id | null)`、`get_active_engines()`。路由模式作为终端代理配置的字段，经现有 `preview_terminal_config` / `apply_terminal_config` 提交。
- 错误结构 `{ code, message }`，`code` 取值沿用：`validation`、`not_found`、`conflict`、`database`、`io`、`upstream`、`internal`。错误消息保持中文。

### 9.2 前端

- 后端 DTO 用 `ts-rs` 生成 TypeScript 类型到 `src/generated/`，字段变化在编译期暴露。
- 前端只改数据接入层（各 feature 的 `api.ts`、类型导入）以及 4.4 节的 ID 类型变更。组件与样式不动。
- Auto 所需的界面入口（路由模式切换、当前引擎选择与展示）复用现有组件与样式，不引入新的视觉元素。

## 10. 质量保障

### 10.1 必修缺陷（v0.1.8 已知问题，每项需有回归测试）

| # | 问题 | 对应设计 |
| --- | --- | --- |
| B1 | SSE 中多字节 UTF-8 跨包切分产生 U+FFFD | 5.3 |
| B2 | 迁移与版本号不在同一事务提交 | 4.1 |
| B3 | 未知 Anthropic 内容块被丢弃 | 5.2 |
| B4 | Responses `encrypted_content` 丢失 | 5.2 |
| B5 | 所有本地错误都返回 HTTP 500 | 5.4 |
| B6 | 服务商 `thinking` 在代理路径上被清空 | 5.5 |

### 10.2 测试

- **core 单元测试**：数据层使用内存 SQLite；协议转换、路由选择、终端规格各自独立测试。
- **请求侧行为基准**：基于 yi-llm `refactor/backend-rust` 分支的 `protocol_oracle` 用例集（42 个字段 × 37 个边界值 × 3 种上游），重新导出完整输出，与 yi-token 做**语义比对**（接受或拒绝、生成的上游 JSON），不比对哈希。B6 等有意的行为差异列入白名单。
- **集成测试**：`wiremock` 模拟上游，覆盖三种协议互转与直通、流式输出（含中文跨包切分）、Auto 切换（切换后新请求走新引擎，进行中的请求不变）、Auto 未选择引擎返回 409。
- **终端配置快照**：`insta`，包含"用户手写内容保留"与"yi-llm 条目保留"的用例。
- **备份**：导出后导入的往返测试；错误口令、篡改密文、篡改信封头、`short_code` 冲突的失败测试，并断言失败后数据库无变化。
- **界面一致性**：复用 yi-llm 的 Playwright 配置，与 yi-llm 截图对比，确保样式未变化。
- **验收命令**：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test`，前端加 `tsc` 与 lint。

## 11. 待实现计划阶段确认的事项

1. Auto 界面入口的具体位置（对照终端页面确定）。
2. JSON / JSONC / YAML 写入时注释是否保留（当前设计为不保留，与 yi-llm 一致）。
