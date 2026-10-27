# yi-token

本地多协议 LLM 网关与终端管理桌面应用。前端、交互和样式对齐 yi-llm
v0.1.8，后端按 `docs/superpowers/specs/2026-10-25-yi-token-design.md` 重新实现为
Rust workspace（`core` + `src-tauri`）。

## 功能

- 本地代理：Anthropic Messages、OpenAI Chat、OpenAI Responses 三种入站协议，
  支持直通与跨协议转换。
- 服务商、标准模型、模型映射、用量统计、监控与日志管理。
- 五个终端客户端的代理 / 直连配置写入。
- Auto 路由：每个终端独立选择当前引擎，切换后新请求立即生效。
- 加密导入导出 `.yitoken`，载荷包含服务商、标准模型、终端配置与日志级别。

## 与 yi-llm 的关系

yi-token 是独立实现，不读取 yi-llm 的数据库，也不导入 `.yillm` 备份。两者
可以同机安装、同时使用，终端配置按所有权标识区分：

| 项 | yi-llm | yi-token |
| --- | --- | --- |
| 数据目录 | `~/.yi-llm` | `~/.yi-token` |
| 模型目录环境变量 | `YI_LLM_MODEL_CATALOG` | `YI_TOKEN_MODEL_CATALOG` |
| 代理 provider id | `yi` | `yitoken` |
| 直连 provider id | `yi_direct_<id>` | `yitoken_direct_<id>` |
| 直连名称前缀 | `yi-llm direct:` | `yi-token direct:` |
| deepseek 环境变量 | `YI_LLM_*` | `YI_TOKEN_*` |
| 备份格式 | `.yillm` | `.yitoken` |

yi-token 只修改和删除带自身标识的终端配置条目；用户手写内容和 yi-llm
写入的条目会原样保留。

## 环境要求

- Node.js 20 或更高版本。
- Rust stable（edition 2024）。
- 平台对应的 Tauri 构建依赖；Windows 需要 WebView2 与 Visual Studio Build
  Tools。
- 运行 Playwright 测试前安装 Chromium：`npx playwright install chromium`。

## 开发与构建

```powershell
npm ci
npm run tauri dev     # 开发模式
npm run tauri build   # 生成安装包，产物在 target/release/bundle/
```

只构建前端可运行 `npm run build`；Tauri 命令需要桌面壳，请使用
`npm run tauri dev`。

## 数据目录与环境变量

| 项 | 取值 |
| --- | --- |
| 数据目录 | `$YI_TOKEN_HOME`，未设置时为 `~/.yi-token`（Windows 为 `%USERPROFILE%\.yi-token`） |
| 数据库 | `<数据目录>/yi-token.db` |
| 日志 | `<数据目录>/logs/yi-token.log.<日期>`，按天滚动 |
| 模型目录覆盖 | `$YI_TOKEN_MODEL_CATALOG` 指向的文件；未设置时使用内置目录 |
| 默认监听 | `127.0.0.1:11435`，可在代理运行页修改 |
| 默认日志级别 | `info`，可在代理运行页运行时调整 |

## Auto 路由

Auto 是代理接入下的路由方式，与固定模型并列；直连模式不受影响。

1. 在终端管理中选择客户端，进入代理接入。
2. 路由模式选择 Auto，终端配置中只暴露一个模型 `Auto`。
3. 在“当前引擎”中选择服务商模型。切换只影响之后到达的新请求，不重写终端
   配置文件，也不会中断进行中的请求。
4. 未选择引擎，或引擎所属服务商已停用时，请求返回 409，并提示先在应用中选择
   当前引擎。
5. Codex 的 `yi-token-models.json` 使用应用配置时的引擎能力。切换引擎后若上下文
   长度差异较大，界面会提示重新应用终端配置。

用量记录会保存终端、`route_mode = auto` 以及实际使用的服务商和模型。

## 终端支持

| 客户端 | 协议 | 配置文件 |
| --- | --- | --- |
| codex | Responses | `$CODEX_HOME` 或 `~/.codex/config.toml`，另写 `yi-token-models.json` |
| claude-code | Anthropic | `$CLAUDE_CONFIG_DIR` 或 `~/.claude/settings.json` |
| opencode | Chat / Responses / Anthropic | `$OPENCODE_CONFIG`，否则 `~/.config/opencode/opencode.json(c)` |
| pi | Chat / Responses / Anthropic | `~/.pi/agent/models.json` 与 `settings.json` |
| deepseek-harness | Chat / Responses / Anthropic | `$DSH_HOME` 或 `~/.dsh/profiles/{desktop,web}/cordis.patch.yml` |

代理模式写入 `http://<host>:<port>/clients/<client>/...` 和固定占位 API Key；
直连模式写入服务商真实的 `base_url` 与 API Key。写入前会备份原文件，失败时
按逆序回滚。

## 模型目录

内置目录为 `models/catalog.json`。应用启动和读取模型前会把目录同步到数据库：
以 `(name, protocol)` 匹配，更新 `brand` 与 `capabilities`，保留数据库 id，因此
Provider 引用不会失效。只在界面创建的标准模型不会被删除，也不会写回目录文件。

设置 `YI_TOKEN_MODEL_CATALOG` 可使用外部共享目录；文件变更会在下一次模型读取或
请求前加载。无效的 JSON、不支持的版本、重复定义或破坏 Provider 引用的修改会被
拒绝，并继续使用上一次有效的数据库缓存。

## 备份

在服务商列表页使用“导入 / 导出”处理 `.yitoken` 文件。加密使用 PBKDF2-HMAC-SHA256
（默认 600,000 次）与 AES-256-GCM，口令至少 8 个字符。

导入采用合并语义：服务商、标准模型、模型和终端配置按 id 插入或更新，备份中没有
的记录保持不变；`short_code` 或 `exposed_name` 冲突时整个事务失败。终端配置只
写入数据库，不会自动改写终端文件，需要用户再次点击“应用”。无法解析的终端引用会
被跳过并在结果中列出。

## 测试与验收

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

界面与 yi-llm v0.1.8 的像素基线位于
`tests/zz-visual.spec.ts-snapshots/`，生成与允许差异见
`tests/visual-baseline/README.md`。规格章节与任务对应关系见
`docs/spec-coverage.md`。

## License

MIT
