# Prompt Pocket 升级验收记录

日期：2026-10-04（与上游 main 整合后的复验）。执行环境：macOS，Node.js 26.10.0，Rust 1.94.0。

## 已完成的功能

- 安全文件替换，保存、改名、删除与同步覆盖前的恢复快照；快照失败阻止危险操作，恢复不覆盖已有文件。
- 全文检索、命中片段与高亮、收藏、最近使用和使用次数；本地偏好在重启后保留。
- 模板变量、默认值、多行填写、缺失值检查、原样输出预览、创建副本与插入已有片段。
- 680×500 快速调用与 960×640 整理模式；最小窗口 640×440，设置页操作按钮固定在底部。
- 异步加载身份校验、未保存草稿提示、加载/保存期间禁止复制、分类与正文一并保存。
- 可配置全局快捷键及失败回滚；复制结果区分已复制、已发送粘贴和粘贴失败。
- 手动同步使用共同内容基线；等长内容变化可识别，冲突保留双方，本地独有文件不删除，两个排序文件参与同步。
- 提取搜索、模板、偏好、编辑加载状态、纯文本转换与恢复模块，增加统一验证入口，整合到 Windows/macOS/Linux 三平台 CI。

## 上游整合

整合基线为 `origin/main` 的 `969c6bb`，保留 GitHub 存档、开机自启动、本地 Mermaid/KaTeX/highlight.js 渲染、Ctrl+S/Shift+Enter、内存新建草稿与拖动刷新保护。

- GitHub 内容读取及条件写使用同一版本的 blob SHA；WebDAV 使用强 ETag。共同比对使用精确内容，按账号/仓库/分支/路径隔离。
- 保留多目标删除确认、空分类标记、分类安全合并和排序迁移；补充原生 canonical 路径与本地库别名兼容回归。
- Tauri JavaScript API 固定在 Rust 锁定版本兼容的次版本范围，避免桌面构建的版本不一致错误。
- 将原独立 Verify workflow 合并进上游 CI：前端测试/类型/构建、Rust 格式/测试/严格 Clippy及桌面打包覆盖三平台；Release 保留 tag / 手动发布触发，并使用锁定 Rust 依赖。
- 首次合并 CI 的 macOS/Linux 测试与 Clippy 通过；Windows 暴露旧路径测试只从预期值去掉原生前缀的问题。改为精确比较两侧 canonical 路径，不修改生产路径校验或放宽越界保护；最终结果以对应提交的 Actions 记录为准。

## 本地检查

| 检查 | 结果 |
| --- | --- |
| `npm run verify` | 90 项测试通过；Svelte/TypeScript 检查 0 错误、0 警告；Vite 6.4.3 生产构建成功 |
| `cargo test --manifest-path src-tauri/Cargo.toml --locked` | 102 项 Rust 测试通过，覆盖存储失败保护、恢复、路径、同步和原生行为判断 |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --locked --all-targets -- -D warnings` | 通过 |
| `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check` | 通过 |
| `npm run tauri:build -- --debug --no-bundle` | macOS 桌面调试构建成功，未运行用户真实提示词库 |
| `npm audit` | 当前锁定的依赖审计结果为 0 漏洞 |
| 生产资源检查 | 开发预览的示例文案与模拟系统操作代码未进入生产 JavaScript |
| `git diff --check` | 通过 |

## 界面回归

在 `http://127.0.0.1:1420/?preview` 使用内存示例数据验证，未访问真实 WebDAV 或用户本地库：

- 搜索仅出现在正文里的“行动计划”，返回项目复盘并高亮正文片段。
- 收藏筛选与浏览器重载后的收藏保留；模板使用后出现在最近使用。
- 18 项列表连续方向键选择至末项，选中项位于可见滚动区域内。
- 模板必填值为空时不能调用；填入多行文本后预览正确，调用后原稿仍含变量。
- 编辑正文和分类后保存，两者同时更新；历史版本可预览并恢复为文件。
- 快捷键设置的界面保存流程，中英文即时切换，以及设置弹窗 Tab 焦点循环。
- 合并后再次验证正文搜索、GitHub 存档设置切换、开机自启动/快捷键/恢复控件同时存在；均使用内存示例数据。
- 640×440 英文设置页、680×500 紧凑/模板界面与 960×640 整理界面无主要操作遮挡。
- 反复点击当前筛选、全部与最近之间切换，当前同一提示词仍能调用和编辑。

回归发现并修复了同一提示词切换筛选后加载标记未重置的问题，以及重复点击当前筛选时错误清空选择的问题。

截图（均为示例数据）：

- [紧凑检索](../screenshots/upgrade/quick-search.png)
- [整理模式](../screenshots/upgrade/manage.png)
- [模板填写](../screenshots/upgrade/template.png)
- [最小窗口英文设置](../screenshots/upgrade/settings-small.png)

## 验证边界

- 合并后的三平台测试与 Windows/macOS 调试安装包构建已在 GitHub Actions 通过。Linux 默认 Gzip RPM 打包长时间无后续输出，改为 Tauri 支持的 Zstd 级别 3，保留 `targets: all` 并重新运行三平台验证。

- Windows 条件编译、原窗口识别、快捷键实际注册和粘贴注入尚未在 Windows 实机执行；已加入 Windows/macOS/Linux CI，运行结果以对应提交的 GitHub Actions 记录为准。
- 本地模拟 WebDAV 验证了完整/不完整清单、失败下载、等长更新、排序、冲突及条件上传；真实坚果云服务的 ETag 支持情况需用测试库验证。缺少强 ETag 时实现会拒绝覆盖已有远程文件。
- 浏览器示例的复制与快捷键是模拟返回，不能作为真实剪贴板或系统快捷键的实机证据。
- 未保存状态与加载保护已检查；浏览器自动化未完整覆盖原生确认框的“保留草稿/放弃草稿”两条路径，需桌面实机回归。
- 本地功能验收产物为源码与调试可执行文件；安装包由 CI 验证，未创建新的版本标签或 Release。提交仅包含源码、项目文档和示例截图，不包含用户提示词库、恢复记录或私密配置。

设计依据见 [DESIGN.md](../../DESIGN.md)，实施计划见 [plan.md](plan.md)，验收条件见 [acceptance.md](acceptance.md)。
