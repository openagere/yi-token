# 通用 API 设置设计

日期：2026-11-12

## 背景

代理除了 `/clients/<client>/...` 终端入口，还有不带终端标识的全局入口
`/v1/messages`、`/v1/chat/completions`、`/v1/responses` 与 `/v1/models`。
用量和轨迹里把它记为 `api`。任何支持自定义 Base URL 的工具（脚本、SDK、
Cherry Studio 一类的聊天客户端、IDE 插件）都可以接这个入口，但界面上没有
地方说明怎么接、能用哪些模型，也不能控制开放范围：全部服务商模型都对外暴露，
未知模型名还会落到默认服务商。

## 目标

在设置页新增“通用 API”分区，作为其他工具接入 yi-token 的唯一说明与配置处：

1. 展示接入信息：Base URL、三种协议的完整端点、模型列表端点、API Key 说明，
   以及可直接复制的调用示例。
2. 展示支持的模型：启用服务商下的模型，显示对外模型名、上游模型、服务商与协议。
3. 配置开放范围：
   - 开放全部模型（默认，与现有行为一致），或只开放勾选的模型；
   - 未匹配到对外模型名时，是否转发到默认服务商（默认开启，与现有行为一致）。

不做：入站鉴权（本地代理当前不校验 Key，保持不变）、按协议限制模型、备份中
包含这组设置（模型 id 跨机器导入时会重建，选择列表意义不大）。

## 数据

`settings` 行新增三列（迁移 2）：

| 列 | 类型 | 默认 | 含义 |
| --- | --- | --- | --- |
| `api_restrict_models` | INTEGER | 0 | 1 表示只开放 `api_model_ids` |
| `api_model_ids` | TEXT(JSON 数组) | `[]` | 开放的模型 id |
| `api_default_fallback` | INTEGER | 1 | 未知模型名是否转发到默认服务商 |

领域类型 `GlobalApiSettings { restrict_models, model_ids, default_fallback }`，
导出为 TS 类型。保存时去重、丢弃不存在的模型 id。

## 路由

- 路由快照携带 `GlobalApiSettings`，保存后调用 `routes_changed` 立即生效。
- `resolve_global`：
  1. 名字命中对外模型名：未限制或在开放列表中时转发；限制且不在列表中时返回
     404“通用 API 未开放模型 X”。
  2. 未命中：`default_fallback` 开启且存在默认服务商时按上游模型名转发，否则 404。
- `GET /v1/models` 只列出开放的模型。
- 终端入口不受影响。

## 接口

- `get_api_settings() -> GlobalApiSettings`
- `save_api_settings(settings: GlobalApiSettings)`

模型列表复用前端已有的 `list_providers`。

## 界面

设置页标签：通用 / 代理监听 / 通用 API / 会话轨迹。“通用 API”分区三块：

1. 接入地址：Base URL（`http://host:port/v1`，随监听设置变化）；表格列出
   OpenAI Chat、Responses、Anthropic Messages、模型列表四行，各自的 Base URL
   与完整端点，均可复制；API Key 说明“填任意非空值，转发时使用服务商密钥”；
   代理未运行时给出提示。附 curl 示例（使用第一个开放模型）。
2. 开放模型：两个选项卡片“全部模型 / 仅选中的模型”；按服务商分组的模型列表，
   限制模式下每行有复选框，并提供全选 / 清空；停用的服务商不出现，列表下方说明。
3. 默认服务商转发：开关，并显示当前默认服务商名称（无默认服务商时说明不可用）。

2、3 共用一个保存按钮与“未保存 / 已保存”状态，与代理监听分区一致。

## 测试

- core：限制模式下未开放模型 404、开放模型可解析、关闭转发后未知名 404、
  `/v1/models` 过滤、设置读写与去重。
- Playwright：打开分区看到端点与模型；切到限制模式勾选并保存，刷新状态保持。
