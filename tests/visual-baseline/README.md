# 视觉基线

`tests/zz-visual.spec.ts` 用 Playwright 的 `toHaveScreenshot` 比较 yi-token 与
yi-llm v0.1.8（`main` = `b61e613`，2026-10-22）的界面。基线图片位于
`tests/zz-visual.spec.ts-snapshots/*-win32.png`。

## 覆盖范围

浅色 / 深色各比较以下页面：

- 连接列表
- 模型管理
- 代理运行
- 模型用量
- 终端管理 · 直连接入
- 终端管理 · 代理接入（固定模型）
- 设置

比较时固定 `Date.now()`，让运行时长、图表横轴、用量日期和“更新于”时间保持一致；
截图禁用 CSS 动画。模型目录路径因产品名不同而被 mask。

## 允许的差异

- 产品名文案（yi-llm → yi-token）。
- Auto 路由控件。代理接入页新增的“路由模式”步骤在截图前隐藏，并把下一个步骤的
  分隔间距归零，因此除该控件外固定模型页必须与 v0.1.8 一致。
- 加密备份弹窗的文案（`.yitoken`、服务商 / 终端配置说明）未纳入像素比较，
  行为由 `tests/workspace.spec.ts` 覆盖。

除此之外的差异都视为视觉回归，应修复前端而不是更新基线。

## 重新生成基线

基线必须来自 yi-llm v0.1.8，不能从 yi-token 自身生成。参考仓库只读，使用
`git archive` 取一份临时副本，不要 checkout 或添加 worktree：

```powershell
$repo = (Get-Location).Path
$ref = Join-Path $env:TEMP "yi-llm-visual-baseline"
New-Item -ItemType Directory -Path $ref -Force | Out-Null
git -C D:\Github\yi-llm archive --format=zip main -o "$ref.zip"
Expand-Archive "$ref.zip" $ref -Force
New-Item -ItemType Junction -Path "$ref\node_modules" -Target "$repo\node_modules"
Copy-Item tests\zz-visual.spec.ts "$ref\tests\zz-visual.spec.ts" -Force
```

在临时副本中启动 Vite 并更新快照（需要 Node.js 20+）：

```powershell
$node = "C:\Program Files\nodejs\node.exe"
Push-Location $ref
& $node "$ref\node_modules\@playwright\test\cli.js" `
  test tests/zz-visual.spec.ts --update-snapshots --reporter=line
Pop-Location
Copy-Item "$ref\tests\zz-visual.spec.ts-snapshots\*" `
  "tests\zz-visual.spec.ts-snapshots\" -Force
```

再运行 `npm run test:ui` 验证。

只有 Playwright 版本、字体或操作系统渲染发生变化时才重新生成；功能改动不应
直接刷新基线。
