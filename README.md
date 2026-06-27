# Prompt Pocket

> 轻量级提示词管理工具 —— 全局快捷键秒唤，Markdown 存储，坚果云同步。

按 `Ctrl+Alt+P` 从任意应用唤出，搜索 → 选中 → `Enter` 复制，回到原应用粘贴即用。

## 特性

- ⚡ **秒唤秒用**：全局快捷键 `Ctrl+Alt+P` 随时唤出/隐藏 spotlight 窗口，失焦自动收起
- 🗂 **分类管理**：一 prompt 一 Markdown 文件，文件夹即分类，支持右键新建/重命名
- ☁️ **坚果云同步**：内置设置界面，填入坚果云账号 + 应用密码即可，启动自动拉取、保存自动推送
- 🔍 **模糊搜索**：标题 / 分类 / 正文实时检索，键盘全程可达
- ✍️ **填空式编辑**：标题/分类用表单填写，无需手写 YAML；Markdown 预览
- 🪶 **极致轻量**：Tauri + Rust 后端，安装包 1.6MB，内存约 42MB，CPU 近乎 0
- 🌗 **自适应深浅色**：跟随系统主题

## 坚果云同步（v1.0.0 新增）

应用通过 **WebDAV 协议**直连坚果云，数据自动同步到所有设备。

### 配置步骤

1. **获取应用密码**：登录 [坚果云官网](https://www.jianguoyun.com) → 账户信息 → 安全选项 → 第三方应用管理 → 添加应用 → 生成**应用密码**（不是登录密码）
2. **应用内配置**：点顶栏 **⚙** → 填入坚果云账号 + 应用密码 → 点「测试连接」验证 → 「保存并同步」
3. 完成。此后启动自动拉取最新，编辑保存后自动推送

### 同步机制

- **本地缓存 + 后台同步**：所有读写走本地缓存（瞬间响应），后台异步与坚果云同步
- **启动拉取**：打开应用时静默从坚果云拉取最新到本地
- **保存即推**：编辑保存后立即推送单个文件到坚果云
- 规避坚果云速率限制（免费版每 30 分钟 600 次请求）

> 应用密码明文存储于 `%APPDATA%/com.promptpocket.app/config.json`，个人电脑可接受。

## 快捷键

| 操作 | 快捷键 |
|---|---|
| 全局唤出 / 隐藏 | `Ctrl+Alt+P` |
| 新建提示词 | `Ctrl+N` |
| 聚焦搜索框 | `Ctrl+F` |
| 上下选择 | `↑` / `↓` |
| 复制选中并隐藏 | `Enter` |
| 隐藏窗口 | `Esc` |
| 列表项右键 | 重命名 / 移动分类 / 置顶 / 删除 |
| 分类右键 | 重命名分类 |

## 技术栈

| 层 | 技术 | 说明 |
|---|---|---|
| 桌面框架 | [Tauri v2](https://tauri.app) | Rust 后端，跨平台原生 |
| 全局快捷键 | `tauri-plugin-global-shortcut` | 注册 Ctrl+Alt+P |
| 云同步 | `reqwest_dav` + 坚果云 WebDAV | 本地缓存 + 后台同步 |
| 前端 | Svelte 5 + Vite + TypeScript | 响应式 UI |
| 存储 | Markdown 文件 + YAML frontmatter | 无数据库 |

## 数据格式

本地缓存于 `%APPDATA%/com.promptpocket.app/PromptPocket/`，同步到坚果云 `PromptPocket/` 目录：

```
PromptPocket/
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
copy_mode: markdown
created: 2026-06-27T00:00:00Z
updated: 2026-06-27T00:00:00Z
---

请把下面这段文字改写得更**简洁**…
```

## 开发

前置依赖：[Node.js](https://nodejs.org) 18+、[Rust](https://rustup.rs) 1.77+。

```bash
npm install
npm run tauri:dev    # 启动开发模式（热重载）
npm run tauri:build  # 打包发布版本
```

## License

MIT
