# Prompt Pocket 全量修复 + 健壮性升级计划

47 个审查问题全修 + 质量门建设。按依赖关系分 7 批，每批跑验证后再进下一批。

## 批次 0：存档
- `git add -A && git commit`（当前 clean + REVIEW 报告），建立回滚点。

## 批次 1：Rust 数据安全（C1/H1/H5/M1/M2/M3/M5）

**store.rs**
1. 新增 `write_atomic(path, content)` helper：写 `xxx.tmp` → `fs::rename` 覆盖（同目录保证同文件系统）。替换全部 `fs::write`：save_prompt、rename_prompt、create_prompt、reorder_category、save_category_order。
2. **C1** `save_prompt`：调换顺序——先原子写新文件成功，再删旧文件。
3. **H5** `delete_prompt`：改为移入 `.trash/{stem}_{timestamp}.md`（提取 sync.rs 的备份逻辑为 `store::move_to_trash`，sync.rs 复用）。需要 root 参数 → 签名改 `delete_prompt(root, abs)`，lib.rs 调用点同步改。
4. **M2** `rename_category` 合并分支：同名文件不覆盖，改名为 `{stem}-{n}.md` 再移入；删目录失败不中断。
5. 新增测试：原子写产生完整文件、delete 进 trash 可恢复、rename_category 不覆盖同名文件。

**lib.rs**
6. **H1** `resolve_abs` 改 `Result<PathBuf, String>`：canonicalize 成功但不以 root 开头 → Err；不存在（Err）→ 手工规范化（展开 `.`/`..`，不依赖文件存在）后校验 starts_with，失败 → Err。所有调用点（read/save/rename/delete/reveal）改 `?` 传播。新增逃逸测试（`../`、绝对路径注入）。
7. **M3** upload_all/download_all：check-and-set 合并进单次锁（`if *s { return Err } *s = true`）；syncing 复位用 guard 结构（Drop 里复位）防 panic 卡死。
8. **M5** `save_cloud_config`：keyring 写失败降级——config.json 只存非密字段，返回 `Ok` 但 last_error 记录"密码未持久化"；不整体失败。
9. **L2** `resolve_local_dir`/`resolve_config_file`：app_config_dir 失败时在日志记录并 fallback（保持启动不崩，但不再完全静默——eprintln + 启动后 sync_status 可见）。

## 批次 2：同步模块对账（H2/H3/H4/M4/M6）

**新增 tombstone 机制**
10. store.rs：`delete_prompt`、`rename_prompt`、`save_prompt`（重命名删旧路径时）、`rename_category` 时往 `.sync_deleted.json` 写入 `{ rel: { size, deletedAt } }`（读旧文件 size 后记录）。sync.rs 提供读写函数（原子写）。

**sync.rs**
11. **H2** push：遍历 tombstone → 对每条发远程 DELETE → 成功则从清单移除 + 清 sync_meta 残留。报告加 `deleted_remote` 计数。
12. **H3** 白名单对齐：`is_trash_or_hidden_rel` 放行 `.order.json`、`.category-order.json`（其余 `.` 开头仍过滤）；push 的 `is_order` 判断覆盖两个文件。
13. **H4** pull：① `need_download` 判据改为"本地不存在 OR size 不同 OR（tombstone 命中且远程 size ≠ tombstone.size，说明远程有新版本）"；tombstone 命中且 size 相同 → 跳过不复活。② 下载覆盖已存在文件前先把旧文件移入 `.trash/`。③ 报告提示"N 个因大小相同未比对"。
14. **M6** URL 编码：所有 `format!("/{root}/{rel}")` 拼接前逐段 percent-encode（手写 encode 函数，与现有 decode 对称，空格/`#`/`?`/`%` 转义，保留 `/`）。
15. **M4** lib.rs：save_prompt/rename_prompt/delete_prompt/reorder 命令入口检查 syncing，true → 返回"正在同步中"错误（前端已有 syncing 状态展示）。
16. 更新 sync.rs 顶部架构注释（与实际实现对齐）。新增测试：tombstone 读写、白名单放行、URL 编码对称性。

## 批次 3：前端选中状态重构 + 编辑保护（C2/C3/H6/H7/H9/H10 + 竞态）

**App.svelte（核心重构）**
17. **C3** 选中状态单源化：
    - 删除 `App.svelte:488-500` 的持续 effect。`selectedIndex` 改为 `$derived(visiblePrompts.findIndex(p => p.path === selectedPath))`（只读派生）。
    - 键盘 ↑↓：基于当前 derived index ±1 → 写 `selectedPath`（钳位）；Enter 用 derived index 取项。
    - `refresh()` 后统一 reconcile：selectedPath 仍在可见列表 → 不动；不在 → 选同位置相邻项（复用 removePromptFromList 逻辑）。
    - doSave：保存后 selectedPath = saved.path，refresh 后 reconcile 自然保住选中（不再被劫持）。doCreate：显式 selectedPath = 新项，无需动 index。
18. **C2** 脏检查：新增 `loadedSnapshot`（加载/保存成功时记录 body/title/category/copyMode）；`isDirty = $derived(editorMode === "edit" && 字段与快照不同)`。列表 onselect、键盘 ↑↓、sync-finished 触发的 loadPromptContent 前检查 isDirty → `await ask(...)` 确认丢弃（用 `@tauri-apps/plugin-dialog` 的 ask，顺带修 macOS confirm 失效；doDelete/onCtxDelete 的 confirm 一并换掉）。
19. **HIGH-5** loadPromptContent 加自增 token，响应落地前比对 `token === latestToken && path === selectedPath`。
20. **H6** 保存改分类：`SaveRequest`（types.ts + Rust）加 `category` 字段；`save_prompt` 若 category 与当前目录不同 → 先按 rename_prompt 的移动逻辑算目标目录（重名加序号），原子写新文件、删旧、记 tombstone。doSave 传 `editingCategory`。
21. **H10** Esc 穿透：renameDialog/catRenameDialog backdrop、Settings、新建分类输入框的 Esc 处理全部加 `e.stopPropagation()`；window handler 开头检查 `e.defaultPrevented`。
22. **编辑态 Esc**：window handler 中 editorMode === "edit" 且无弹窗时，Esc 先触发 oncancel（退出编辑）而非直接 hideWindow。

**Editor.svelte**
23. **H9** renderRich effect 内显式读 `body`（`const _b = body;` 并纳入依赖），切 prompt 后 mermaid/KaTeX/高亮正常渲染。
24. 预览链接拦截（LOW-5）：只放行 `^https?:` 给 openUrl，锚点/相对路径直接忽略。

**PromptList.svelte**
25. **H7** 声明 `onmounted?: (fn: (i: number) => void) => void` prop，onMount 时回传 `(i) => itemEls[i]?.scrollIntoView({ block: "nearest" })`；`{#each}` 块用索引收集 itemEls。键盘选中项高亮（selectedIndex === i 加 `.kbd-active` 样式，因 selectedPath 即键盘选中，直接用现有 active 样式即可——verify 后定）。
26. **M8** 拖拽加固：pointerdown 时对 handle `setPointerCapture`（或在 move 里 `(e.buttons & 1) === 0` 时 `finishPointerDrag(false)`——两者都做，防卡死 + 防陈旧提交）。CategoryTabs 同改。

**ContextMenu.svelte**
27. **M10** 菜单挂载后测量，越出视口则向左/上翻转（调整 style left/top）；删 `.arrow` 重复 CSS。

**api.ts**
28. 删 savePrompt 双发字段死代码（只发 camelCase，后端 rename_all 已统一）。

## 批次 4：渲染管线本地化 + CSP（H8/LOW-1/LOW-2）

29. `npm i mermaid katex highlight.js` + `npm i -D @tauri-apps/plugin-dialog`（批次 3 的 ask 用）。
30. renderers 改本地动态 import（Vite 自动分包，保持按需加载）：
    - mermaid.ts：`const { default: mermaid } = await import("mermaid")`
    - katex.ts：`await import("katex")` + `import "katex/dist/katex.min.css"`（CSS 静态打进 bundle）
    - highlight.ts：`await import("highlight.js/lib/common")` + 主题 CSS 静态 import
    - loadRemote.ts 整个删除；renderers/index.ts 签名不变（renderRich 接口不动）。
    - highlight 标记顺序修正（LOW-3）：先 highlightElement 成功后再打 data-rendered。
31. tauri.conf.json CSP 收紧：`script-src 'self'`；`style-src 'self' 'unsafe-inline'`；`font-src 'self' data:`；`connect-src 'self'`；img-src 不变。
32. markdown.ts（LOW-1）：walkTokens 加 `token.type === "image"` 协议白名单（同 link）。
33. markdown.test.mjs 补图片协议注入用例。

## 批次 5：质量门 + 测试补全（M9/M12 + 类型修复）

34. 修存量类型错误：markdown.ts:36/91/111（highlight 回调对 mermaid 提前 return `string`、token 断言过 unknown）、loadRemote.ts:82（随删除消失）、App.svelte onmounted ×2（随 H7 修好）。
35. package.json：`"check": "svelte-check --tsconfig ./tsconfig.json && tsc --noEmit"`、`"test": "node --experimental-strip-types --test src/lib/*.test.mjs"`；README 补 Node 22 要求。
36. CI：test job 在单测前加 `npm run check` 步骤（三平台同）。
37. tsconfig：加 `"noEmit": true`，删 `checkJs` 死配置。
38. Cargo.toml：删未使用的 `dirs = "5"`。
39. 新增测试：i18n 缺 key 回退；markdown 图片协议；Rust 侧批次 1/2 的测试（resolve_abs 逃逸、原子写、trash、tombstone、URL 编码、rename_category 不覆盖）。

## 批次 6：UX/收尾（LOW 批量）

40. autofocus：重命名标题、分类重命名、新建分类、编辑器新分类 4 个输入框（`use:action` 或 onMount + tick）。
41. 未捕获 rejection 补 catch：App.svelte:193/517/719。
42. `translateReorderDisabledReason` 改 reason code：reorder.ts 返回 `"needTwo" | "singleCategory" | "searchDisabled" | null`，App 直接映射 i18n key（不再用中文当中间格式）；reorder.test.mjs 同步。
43. M11 search.ts 注释改为"标题/分类/标签"（与实现一致）。
44. LOW-4 reorder.ts：`movePathOrder` 委托 `moveCategoryOrder` 去重。
45. M5（拖拽手势期 refresh）：PromptList/CategoryTabs 加 `ondragstart`/`ondragend` 回调，App 手势期挂起 refresh（复用 reorderInFlight/pendingRefresh 机制）。
46. lib.rs 平台提示 README 补：macOS 辅助功能权限、Linux/Wayland 模拟粘贴限制、Ctrl+Alt+P 冲突说明。
47. doReorder 并发防护（LOW）：reorderInFlight 时直接忽略二次拖拽提交。

## 每批验证门禁（不过不进下一批）
- `cargo test`（src-tauri）全绿
- `npm test` 全绿
- `npx tsc --noEmit` + `npx svelte-check` 0 错误（批次 5 起硬性 0）
- 批次 4 后加 `npm run build` 验证打包
- 批次 3/4 涉及 UI 行为变更，提供手测清单（tauri:dev 冒烟：编辑→切换、保存→选中、新建→编辑、Esc、拖拽、渲染）

## 明确不做
- 不 bump 版本号、不发版（修完豆哥验证后再说）
- 不做架构级重写（同步仍手动双向、存储仍 Markdown 文件）
- 正文搜索不实现（M11 只改注释，功能是独立需求）
- REVIEW-2026-08-08.md 留在工作区不提交（或随批次 0 一起提交，豆哥可后续删）
