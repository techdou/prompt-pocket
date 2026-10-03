# Design

## Source of truth
- Status: Active
- Last refreshed: 2026-10-04
- Primary product surfaces: 快速调用、提示词整理、模板填写、设置与恢复。
- Evidence reviewed: README.md、docs/screenshots/list.png、src/App.svelte、src/app.css、src/lib/Editor.svelte、PromptList.svelte、Settings.svelte。

## Brand
- Personality: 安静、轻巧、清晰。
- Trust signals: 本地 Markdown、明确保存状态、可恢复操作、准确复制反馈。
- Avoid: 大面积装饰、营销式文案、隐藏覆盖行为。

## Product goals
- Goals: 快速找到提示词，少做重复修改，数据可恢复。
- Non-goals: 本次不引入云端账户、模型聊天服务或多人协作。
- Success signals: 从搜索到调用全键盘完成；失败不丢稿；模板可直接生成使用。

## Personas and jobs
- Primary personas: 经常在 AI 工具、浏览器和编辑器间复用提示词的个人用户。
- User jobs: 检索、收藏、填写模板、调用、编辑、备份与恢复。
- Key contexts of use: 工作中短暂唤起；集中整理提示词库。

## Information architecture
- Primary navigation: 搜索、全部/收藏/最近、分类；整理模式入口。
- Core routes/screens: 同一窗口内紧凑调用与双栏整理；模板对话框；设置恢复面板。
- Content hierarchy: 标题与命中片段优先，分类与使用记录次之，详细编辑在整理界面。

## Design principles
- 直接操作有即时反馈；破坏性动作可恢复。
- 键盘与鼠标功能等价，焦点顺序可预测。
- 优先复用现有组件和样式；只增加完成任务所需的控件。
- Tradeoffs: 模板填写增加一步，但只在正文包含变量时出现。

## Visual language
- Color: 延续现有 CSS 蓝色强调与白/浅灰底；错误使用现有错误色。
- Typography: 系统无衬线；正文保持可读行高；代码使用等宽。
- Spacing/layout rhythm: 8px 基准，按钮至少 32px 高。
- Shape/radius/elevation: 沿用现有圆角、边框与轻阴影。
- Motion: 小幅状态过渡，支持 reduced-motion。
- Imagery/iconography: 使用现有文字/简单图标，无新增图片资产。

## Components
- Existing components to reuse: CategoryTabs、PromptList、Editor、Settings、ContextMenu。
- New/changed components: 模板填写弹窗、恢复记录列表、收藏/最近筛选与模式切换。
- Variants and states: 加载、无结果、无收藏、模板待填写、恢复失败、同步部分失败。
- Token/component ownership: src/app.css 管全局基础，组件样式负责局部布局。

## Accessibility
- Target standard: 语义化按钮/表单、可见焦点、可读对比度。
- Keyboard/focus behavior: ↑↓选择、Enter调用、Esc关闭顶层弹窗；输入法组合态不调用；关闭弹窗后回到触发处。
- Contrast/readability: 次要信息仍可读，状态不只依赖颜色。
- Screen-reader semantics: 每个图标按钮有标签，状态用 live region。
- Reduced motion and sensory considerations: 减少动画首选项有效。

## Responsive behavior
- Supported breakpoints/devices: 桌面窗口，最小宽度 640px；管理双栏在窄宽度自动收紧。
- Layout adaptations: 紧凑模式单列表与短预览；整理模式展开详情；弹窗内部滚动。
- Touch/hover differences: 核心操作不依赖 hover。

## Interaction states
- Loading: 正文未对应当前选择时禁用调用。
- Empty: 给出新建或调整搜索的直接入口。
- Error: 保留用户输入和原文件，提供具体原因。
- Success: 区分已复制、已发送粘贴、已保存、已恢复。
- Disabled: 解释缺少必填变量、同步进行中等原因。
- Offline/slow network: 本地编辑搜索调用可用；同步错误不阻塞本地工作。

## Content voice
- Tone: 简短、具体，中英文等价。
- Terminology: 提示词、模板、变量、收藏、最近使用、恢复记录。
- Microcopy rules: 明确影响方向；不用实现细节替代操作说明。

## Implementation constraints
- Framework/styling system: Svelte 5 + TypeScript + Tauri/Rust；沿用 CSS。
- Design-token constraints: 复用 src/app.css，不引入 UI 框架。
- Performance constraints: 搜索在内存完成；正文加载校验请求身份；不阻塞键盘操作。
- Compatibility constraints: 旧 Markdown 可继续读取；中英切换、GitHub/WebDAV 手动同步、开机自启动和本地富文本渲染保留。
- Test/screenshot expectations: 新逻辑有回归测试；预览截图审查主要状态。

## Open questions
- macOS/Linux 自动粘贴的系统级适配与权限引导需要各平台实机验证，当前保证复制可用。
