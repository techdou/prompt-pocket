# Prompt Pocket

<p align="center">
  <img src="docs/screenshots/list.png" alt="Prompt Pocket 主界面" width="720" />
</p>

> 全局秒唤的提示词口袋：`Ctrl+Alt+P` 从任意应用唤出，搜索 → 选中 → `Enter` 复制或自动粘贴。一条提示词一个 Markdown 文件，文件夹即分类，坚果云手动同步。

**官网**：<https://techdou.github.io/prompt-pocket/>

---

## 适合谁

- 经常在 ChatGPT、IDE、浏览器输入框、文档编辑器之间复用提示词的人
- 想用本地 Markdown 文件管理提示词，而不是把数据交给第三方服务的人
- 希望提示词工具像系统搜索一样：随叫随到，用完自动消失的人

## 核心能力

| 能力 | 说明 |
| --- | --- |
| 全局秒唤 | `Ctrl+Alt+P` 从任意应用唤出 Prompt Pocket |
| 智能复制 / 粘贴 | `Enter` 写入剪贴板；唤出前焦点在输入框时自动粘贴回原输入框 |
| Markdown 存储 | 一条提示词一个 `.md` 文件，文件夹即分类，便于备份、搜索、迁移 |
| 富 Markdown 渲染 | 预览支持 GFM 全语法（表格 / 引用 / 删除线 / 任务列表），Mermaid 图表、KaTeX 公式、代码高亮按需从 CDN 加载 |
| 双向拖拽排序 | 提示词与分类都支持原生拖拽重排，顺序持久化到本地、随云同步 |
| 键盘优先 | `Ctrl+F` 搜索、`↑` / `↓` 选择、`Enter` 复制，全流程可不碰鼠标 |
| 手动云同步 | 通过坚果云 WebDAV 上传 / 下载，避免自动同步误覆盖 |
| 轻量桌面壳 | Tauri v2 + Rust 后端，不使用 Electron |

## 使用流程

1. 在任意应用的输入框里放好光标
2. 按 `Ctrl+Alt+P` 唤出 Prompt Pocket
3. 搜索或用 `↑` / `↓` 选中提示词
4. 按 `Enter`

结果：

- **唤出前焦点在输入框**：提示词写入剪贴板，并自动粘贴到原输入框
- **唤出前焦点不在输入框**：只写入剪贴板，不模拟粘贴

输入框识别在 Windows 上优先使用 UI Automation 识别现代输入框，失败时回退到传统 caret 检测。

## 快捷键

| 操作 | 快捷键 |
| --- | --- |
| 全局唤出 / 隐藏 | `Ctrl+Alt+P` |
| 新建提示词 | `Ctrl+N` |
| 聚焦搜索框 | `Ctrl+F` |
| 上下选择 | `↑` / `↓` |
| 复制选中项 | `Enter` |
| 隐藏窗口 | `Esc` |

## 数据格式

本地数据默认保存在：

```text
%APPDATA%/com.promptpocket.app/PromptPocket/        # Windows
~/Library/Application Support/com.promptpocket.app/ # macOS
~/.config/com.promptpocket.app/                     # Linux
```

目录结构示例：

```text
PromptPocket/
├── 写作/
│   ├── 改写润色.md
│   └── 周报模板.md
├── 编程/
│   └── 代码审查.md
├── .order.json          # 提示词排序（按分类）
└── .category-order.json # 分类排序
```

每个提示词文件都是普通 Markdown：

```markdown
---
title: 改写润色
copy_mode: markdown
created: 2026-06-27T00:00:00Z
updated: 2026-06-27T00:00:00Z
---

请把下面这段文字改写得更简洁、专业：

> 待改写内容
```

### 富 Markdown 渲染说明

预览采用**分层渲染**，兼顾离线可用与体积：

- **离线（打包进应用）**：GFM 全语法——标题、表格、引用、分割线、删除线、任务列表、代码块。raw HTML 一律转义，`javascript:` 等危险协议链接会被替换为 `#`，无需额外 sanitizer。
- **联网（按需 CDN）**：检测到对应语法才加载
  - ` ```mermaid ` 代码块 → 加载 [Mermaid](https://mermaid.js.org/) 渲染流程图 / 时序图
  - `$...$` / `$$...$$` → 加载 [KaTeX](https://katex.org/) 渲染数学公式
  - ` ```js ` 等带语言代码块 → 加载 [highlight.js](https://highlightjs.org/) 高亮

加载失败或离线时，对应内容退化为源码展示，绝不白屏。核心内容（GFM）始终离线可用，保证「秒唤」体验。

### 拖拽排序说明

- **提示词排序**：在单个分类视图下，按住列表项左侧 `⠿` 手柄拖动，落点指示线实时跟随。顺序写入 `.order.json`。
- **分类排序**：按住分类标签上的 `⠿` 手柄横向拖动。「全部」标签固定首位不可拖。顺序写入 `.category-order.json`。
- 两种顺序文件都会随云同步自动上传。

搜索结果、「全部」视图下的提示词列表会自动禁用排序，避免隐藏项错序。

## 坚果云同步

Prompt Pocket 通过 WebDAV 连接坚果云。

**配置方式**：

1. 登录坚果云 → 账户信息 → 安全选项 → 第三方应用管理
2. 添加应用并生成**应用密码**（不是登录密码）
3. 在 Prompt Pocket 的设置里填写账号、应用密码、远程目录
4. 用「上传到坚果云」或「下载到本地」手动同步

**同步策略**：

- 上传：把本地提示词推送到云端，不删除云端已有文件
- 下载：以云端为准拉取到本地，并清理云端已删除的本地文件
- `.trash`、隐藏目录和 `.sync_meta.json` 会被过滤，避免备份或内部元数据混入列表

## 开发

**前置依赖**：Node.js 18+、Rust 1.77+，以及 [Tauri v2 要求的平台工具链](https://v2.tauri.app/start/prerequisites/)。

```bash
npm install
npm run tauri:dev    # 开发
npm run tauri:build  # 打包
```

测试：

```bash
node --experimental-strip-types --test src/lib/reorder.test.mjs src/lib/api.test.mjs
cargo test --manifest-path src-tauri/Cargo.toml
```

## GitHub Pages

落地页位于 `docs/index.html`，截图资源位于 `docs/screenshots/`。GitHub Pages 配置为 `main` 分支的 `/docs` 目录；推送到 `main` 后自动重新发布。

本地预览：

```bash
cd docs && python -m http.server 8010
```

## 技术栈

| 层 | 技术 |
| --- | --- |
| 桌面 | Tauri v2 |
| 后端 | Rust |
| 前端 | Svelte 5 + Vite + TypeScript |
| Markdown | marked + 分层 CDN（Mermaid / KaTeX / highlight.js） |
| 快捷键 | `tauri-plugin-global-shortcut` |
| 剪贴板 | `tauri-plugin-clipboard-manager` |
| 输入框识别 | Windows UI Automation + legacy caret fallback |
| 键盘注入 | `enigo` |
| 云同步 | `reqwest_dav` + 坚果云 WebDAV |
| 数据 | Markdown + YAML frontmatter |

## License

[Apache License 2.0](LICENSE)
