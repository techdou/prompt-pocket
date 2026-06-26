# Prompt Pocket

> 轻量级提示词管理工具 —— 全局快捷键秒唤，Markdown 存储，云盘同步。

按 `Ctrl+Alt+P` 从任意应用唤出，搜索 → 选中 → `Enter` 复制，回到原应用粘贴即用。

## 特性

- ⚡ **秒唤秒用**：全局快捷键 `Ctrl+Alt+P` 随时唤出/隐藏 spotlight 窗口，失焦自动收起
- 🗂 **分类清晰**：一 prompt 一 Markdown 文件，文件夹即分类，YAML frontmatter 存标签/时间戳
- ☁️ **云同步**：内置设置界面，一键把数据目录指向你的 OneDrive / iCloud / 坚果云同步文件夹，配置自动持久化
- 🔍 **模糊搜索**：标题 / 分类 / 标签 / 正文实时检索，键盘全程可达
- ✍️ **Markdown 编辑 + 预览**：所见即所得的预览，源码编辑模式，支持富文本与纯文本复制
- 🪶 **极致轻量**：Tauri + Rust 后端，安装包仅 1.6MB，内存占用约 37-70MB，CPU 近乎 0（常驻不退出）
- 🌗 **自适应深浅色**：跟随系统主题

## 云同步

点顶栏齿轮 ⚙ →「提示词存储目录」→「选择…」，把目录指向你的云盘同步文件夹即可。提示词会以 Markdown 文件存进去，云盘客户端自动同步到所有设备。

默认目录是 `~/Documents/PromptPocket/`（OneDrive / iCloud 常已接管此目录）。

## 技术栈

| 层 | 技术 | 说明 |
|---|---|---|
| 桌面框架 | [Tauri v2](https://tauri.app) | Rust 后端，跨平台原生，安装包小、内存低 |
| 全局快捷键 | `tauri-plugin-global-shortcut` | 注册 Ctrl+Alt+P |
| 前端 | Svelte 5 + Vite + TypeScript | 响应式 UI，运行时轻 |
| 存储 | Markdown 文件 + YAML frontmatter | 无数据库，云同步友好 |

## 数据格式

```
~/Documents/PromptPocket/
├── 写作/
│   ├── 改写润色.md
│   └── 周报模板.md
└── 编程/
    └── 代码审查.md
```

每个 `.md` 文件：

```markdown
---
title: 改写润色
tags: [写作, 润色]
copy_mode: markdown
created: 2026-06-27T00:00:00Z
updated: 2026-06-27T00:00:00Z
---

请把下面这段文字改写得**更简洁**…
```

## 快捷键

| 操作 | 快捷键 |
|---|---|
| 全局唤出 / 隐藏 | `Ctrl+Alt+P` |
| 新建提示词 | `Ctrl+N` |
| 聚焦搜索框 | `Ctrl+F` |
| 上下选择 | `↑` / `↓` |
| 复制选中并隐藏 | `Enter` |
| 隐藏窗口 | `Esc` |

## 开发

前置依赖：[Node.js](https://nodejs.org) 18+、[Rust](https://rustup.rs) 1.77+。

```bash
npm install        # 安装前端依赖
npm run tauri:dev  # 启动开发模式（热重载）
npm run tauri:build  # 打包发布版本
```

首次启动会在 `~/Documents/PromptPocket/` 写入示例 prompt。

## 设计原则

- **本地优先、零后端**：数据在你自己的磁盘和云盘，不上传任何服务器
- **一文件一 prompt**：避免数据库在云同步时的冲突损坏
- **纯工具定位**：不内置 AI，专注"存得整齐、找得快、复制得爽"

## License

MIT
