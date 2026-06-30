// prompt 数据结构，与 Rust 端 Prompt struct 一一对应（serde 自动转 camelCase）
export interface PromptMeta {
  /** 标题，缺省时取文件名（去扩展名） */
  title: string;
  /** 旧版 frontmatter 兼容字段；新文件不再写入 */
  tags?: string[];
  /** 复制时是否先转纯文本：markdown 渲染成纯文本 / 原样 */
  copy_mode: "markdown" | "plain";
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
  copy_mode: "markdown" | "plain";
  body: string;
}
