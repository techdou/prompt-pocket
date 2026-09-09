// ── 前后端数据形状契约 ──
// Rust 端结构体全部 serde(rename_all = "camelCase")，invoke 返回的运行时
// 形状是 camelCase（copyMode / absPath / lastSync）；本文件的前端类型约定
// snake_case（copy_mode / abs_path），翻译只发生在 api.ts 的 normalize* 层。
// 因此：新增 command 的返回必须经过 normalize 再进组件，不得绕过 normalize
// 直接消费 invoke 结果——类型标注说有 copy_mode、运行时却是 undefined，
// 编译期查不出来。
export type CopyMode = "markdown" | "plain";

export interface PromptMeta {
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
