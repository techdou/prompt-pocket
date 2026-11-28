// ── 前后端数据形状契约 ──
// Rust 端结构体全部 serde(rename_all = "camelCase")，invoke 返回的运行时
// 形状是 camelCase。本文件分两类：
// - Prompt 族（Prompt/PromptMeta/PromptContent/ScanResult/CategoryCount）：
//   前端类型约定 snake_case（copy_mode/abs_path），翻译只发生在 api.ts 的
//   normalize* 层——新增 command 的返回必须过 normalize 再进组件，绕过
//   normalize 直接消费 invoke 结果会拿到 camelCase，类型说有 copy_mode、
//   运行时却是 undefined，编译期查不出来。
// - SyncStatus / CloudConfigView：camelCase 直映 wire，不经 normalize。
export type CopyMode = "markdown" | "plain";

export interface PromptMeta {
  /** 兼容带标签的提示词元数据。 */
  tags?: string[];
  /** 标题，缺省时取文件名（去扩展名） */
  title: string;
  /** 复制时是否先转纯文本：markdown 渲染成纯文本 / 原样 */
  copy_mode: CopyMode;
  /** 创建时间 ISO 字符串 */
  created: string;
  /** 更新时间 ISO 字符串 */
  updated: string;
}

export interface Prompt {
  /** 相对于仓库根的稳定 id（无扩展名路径，正斜杠分隔） */
  id: string;
  /** 显示标题 */
  title: string;
  /** 所属分类（即父文件夹名，根目录则为 "未分类"） */
  category: string;
  /** 文件相对路径，正斜杠分隔，含 .md */
  path: string;
  /** 绝对路径（系统相关分隔符） */
  abs_path: string;
  /** frontmatter 元数据 */
  meta: PromptMeta;
  /** 正文全文（扫描时随列表返回，供内容搜索） */
  body: string;
  /** 在分类内的排序权重（来自 .order.json），undefined 表示未定义 */
  order?: number;
}

/** 扫描结果：prompt 列表 + 分类汇总 */
export interface ScanResult {
  prompts: Prompt[];
  categories: CategoryCount[];
}

export interface CategoryCount {
  name: string;
  count: number;
}

/** 云同步配置（读取时用，密码只返回是否存在） */
export interface CloudConfigView {
  username: string;
  remoteRoot: string;
  enabled: boolean;
  hasPassword: boolean;
  /** 当前激活的同步后端："webdav"（缺省）| "github" */
  provider?: string;
  /** GitHub 存档配置（密钥只返回 hasToken，永不下发明文） */
  ghRepo?: string;
  ghBranch?: string;
  ghPrefix?: string;
  ghEnabled?: boolean;
  hasToken?: boolean;
}

/** 同步状态 */
export interface SyncStatus {
  configured: boolean;
  enabled: boolean;
  lastSync: string | null;
  lastError: string | null;
  syncing: boolean;
}

/** read_prompt 返回：结构化元数据 + 正文 */
export interface PromptContent {
  meta: PromptMeta;
  body: string;
}

/** save_prompt 接收的结构化保存请求（前端表单直接构造） */
export interface SaveRequest {
  title: string;
  copy_mode: CopyMode;
  body: string;
  /** 目标分类：与当前目录不同则保存时移动文件（serde camelCase → category） */
  category?: string;
}

/** 同步一次执行的统计（Rust SyncReport 结构化返回，文案由前端 i18n 拼装） */
export interface SyncReport {
  downloaded: number;
  skipped: number;
  deleted: number;
  uploaded: number;
  deletedRemote: number;
  conflicts: number;
  errors: string[];
}

export interface RecoveryEntry {
  id: string;
  originalPath: string;
  createdAt: string;
  kind: string;
}
/**
 * 唤起会话快照。hasTarget 决定 Enter 是"隐藏并注入回原窗口"还是"仅复制"；
 * generation 标识"这是第几次唤起"，发起粘贴请求时原样带回后端——用户在
 * 第 N 次唤起的窗口里按的 Enter，即使指令晚到一步，也不会被算到第 N+1 次唤起头上。
 */
export interface PasteSessionSnapshot {
  generation: number;
  hasTarget: boolean;
}

/**
 * paste_sent：注入指令已完整发出（SendInput 4 事件全部送达），不代表目标
 * 应用已插入——UIPI / 目标忽略快捷键不可观测。
 * superseded：等待期间发生了新的唤起（用户又按了快捷键），本次未注入，无需提示。
 * paste_failed：自动粘贴失败，按 reason 区分是否还能手动粘贴。
 */
export type PasteFailReason =
  | "hide_failed"
  | "focus_lost"
  | "modifiers_held"
  | "clipboard_changed"
  | "inject_failed";
export type CopyResult =
  | { status: "copied" }
  | { status: "paste_sent" }
  | { status: "superseded" }
  | { status: "paste_failed"; reason: PasteFailReason };
