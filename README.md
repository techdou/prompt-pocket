# Prompt Pocket

<p align="center">
  <img src="docs/screenshots/list.png" alt="Prompt Pocket main window" width="720" />
</p>

Prompt Pocket 是一个轻量级桌面提示词管理工具。它像系统搜索一样用 `Ctrl+Alt+P` 秒唤，选中提示词后按 `Enter` 即可复制；如果唤出前焦点在输入框里，还会自动粘贴回原输入框。

官网：<https://techdou.github.io/prompt-pocket/>

## 适合谁

- 经常在 ChatGPT、浏览器、IDE、文档编辑器之间复用提示词的人
- 想把提示词保存为本地 Markdown 文件，而不是交给第三方服务的人
- 想要一个随叫随到、用完自动隐藏的提示词口袋的人

## 核心特性

| 能力 | 说明 |
| --- | --- |
| 全局秒唤 | `Ctrl+Alt+P` 从任意应用唤出或隐藏 |
| 智能复制 / 粘贴 | `Enter` 写剪贴板；唤出前在输入框时自动粘贴 |
| Markdown 存储 | 一条提示词一个 `.md` 文件，文件夹就是分类 |
| 富 Markdown 预览 | 支持 GFM 表格、任务列表、代码块；Mermaid、KaTeX、highlight.js 按需加载 |
| 提示词排序 | 在单个分类里拖动列表项左侧手柄，顺序写入 `.order.json` |
| 分类排序 | 横向拖动分类标签手柄，顺序写入 `.category-order.json` |
| 手动 WebDAV 同步 | 通过坚果云上传 / 下载，避免自动同步误覆盖 |
| 安全凭据存储 | WebDAV 应用密码保存到系统凭据库，不写入明文 JSON |
| 托盘与单实例 | 快捷键被占用时仍可从托盘打开；重复启动会唤醒已有窗口 |
| 轻量桌面壳 | Tauri v2 + Rust 后端，不使用 Electron |

## 安装

从 GitHub Releases 下载 Windows 安装包：

- `Prompt Pocket_2.0.1_x64-setup.exe`：推荐，普通安装器
- `Prompt Pocket_2.0.1_x64_en-US.msi`：MSI 安装包
- `prompt-pocket.exe`：release 构建出的可执行文件

首次启动会显示主窗口；之后默认隐藏到后台，可用快捷键或托盘打开。

## 快速使用

1. 在任意应用的输入框里放好光标
2. 按 `Ctrl+Alt+P` 唤出 Prompt Pocket
3. 搜索或用方向键选中提示词
4. 按 `Enter`

结果：

- 如果唤出前焦点在输入框：写入剪贴板，并自动粘贴回原输入框
- 如果唤出前焦点不在输入框：只写入剪贴板，不模拟粘贴

Windows 上输入框识别优先使用 UI Automation（用户界面自动化）识别现代输入框，失败时回退到传统 caret（文本光标）检测。

## 快捷键

| 操作 | 快捷键 |
| --- | --- |
| 全局唤出 / 隐藏 | `Ctrl+Alt+P` |
| 新建提示词 | `Ctrl+N` |
| 聚焦搜索框 | `Ctrl+F` |
| 上下选择 | `↑` / `↓` |
| 复制选中项 | `Enter` |
| 隐藏窗口 | `Esc` |

## 数据结构

默认数据目录：

```text
Windows: %APPDATA%/com.promptpocket.app/PromptPocket/
macOS:   ~/Library/Application Support/com.promptpocket.app/PromptPocket/
Linux:   ~/.config/com.promptpocket.app/PromptPocket/
```

目录示例：

```text
PromptPocket/
├── 写作/
│   ├── 改写润色.md
│   └── 周报模板.md
├── 编程/
│   └── 代码审查.md
├── .order.json          # 每个分类内的提示词排序
└── .category-order.json # 分类排序
```

提示词文件格式：

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

## 拖拽排序

提示词排序和分类排序都使用 Pointer Events（指针事件），不依赖浏览器原生 Drag and Drop（拖放 API）。原因很简单：Tauri/WebView2 里原生拖放容易被桌面壳、窗口拖动和系统事件链路干扰。

- 提示词排序：只在单个分类视图可用，搜索结果和多分类「全部」视图会禁用排序
- 分类排序：「全部」固定首位不可拖，其他分类可横向重排
- 写盘策略：前端先乐观更新，再调用 Rust 后端写入排序 JSON
- 竞态保护：排序写盘期间如果同步完成触发刷新，会延迟到写盘后再刷新，避免顺序被旧文件冲掉

## 富 Markdown 预览

预览分两层：

- 离线内置：GitHub Flavored Markdown（GFM），包括表格、引用、删除线、任务列表、代码块
- 联网增强：检测到对应语法时，按需从 CDN 加载 Mermaid、KaTeX、highlight.js

安全处理：

- raw HTML 一律转义显示
- `javascript:` 等危险链接会替换为 `#`
- Mermaid / KaTeX 占位元素会分别做文本转义和属性转义，避免属性注入
- CDN 加载失败时降级显示源码，不影响核心阅读和复制

## 坚果云同步

Prompt Pocket 通过坚果云 WebDAV 手动同步。

配置步骤：

1. 登录坚果云
2. 打开「账户信息 → 安全选项 → 第三方应用管理」
3. 添加应用并生成应用密码
4. 在 Prompt Pocket 设置中填写账号、应用密码和远程目录
5. 选择「上传到坚果云」或「下载到本地」

同步规则：

- 上传：把本地提示词推送到云端，不删除云端已有文件
- 下载：以云端为准拉取到本地，并清理云端已删除的本地文件
- `.trash`、隐藏目录和 `.sync_meta.json` 会被过滤
- 应用密码保存到系统凭据库；旧版本明文 JSON 中的密码会在读取时迁移出去

## 开发

前置依赖：

- Node.js 18+
- Rust 1.77+
- Tauri v2 平台工具链：<https://v2.tauri.app/start/prerequisites/>

安装依赖：

```bash
npm install
```

开发运行：

```bash
npm run tauri:dev
```

生产构建：

```bash
npm run tauri:build
```

## 验证命令

```bash
node --experimental-strip-types --test src/lib/*.test.mjs
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run tauri:build
```

## GitHub Pages

落地页位于 `docs/index.html`，截图资源位于 `docs/screenshots/`。

GitHub Pages 配置为 `main` 分支的 `/docs` 目录。推送到 `main` 后，Pages 会按仓库配置重新发布。

本地预览：

```bash
cd docs
python -m http.server 8010
```

## 技术栈

| 层 | 技术 |
| --- | --- |
| 桌面壳 | Tauri v2 |
| 后端 | Rust |
| 前端 | Svelte 5 + Vite + TypeScript |
| Markdown | marked + marked-highlight |
| 富内容增强 | Mermaid / KaTeX / highlight.js CDN 按需加载 |
| 快捷键 | tauri-plugin-global-shortcut |
| 剪贴板 | tauri-plugin-clipboard-manager |
| 托盘 | Tauri tray icon |
| 单实例 | tauri-plugin-single-instance |
| 凭据存储 | keyring + 系统凭据库 |
| 云同步 | reqwest_dav + 坚果云 WebDAV |
| 数据格式 | Markdown + YAML frontmatter |

## License

[Apache License 2.0](LICENSE)
