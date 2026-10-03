export const LANGUAGE_STORAGE_KEY = "prompt-pocket.language";

export const LANGUAGES = ["zh", "en"] as const;
export type Language = (typeof LANGUAGES)[number];

type TranslationValues = Record<string, string | number>;

const zh = {
  "app.loading": "正在加载提示词...",
  "app.searchPlaceholder": "搜索标题或内容...",
  "app.newPrompt": "新建 (Ctrl+N)",
  "app.settings": "设置",
  "app.switchLanguageTitle": "切换到英文",
  "app.switchLanguageAria": "切换语言",
  "app.syncConnected": "已连接坚果云",
  "app.syncStatusAria": "同步状态（点击打开设置）",
  "app.deleteConfirm": "确定删除「{title}」？可在设置的恢复记录中找回。",
  "app.discardChanges": "当前有未保存的修改，切换后将丢失。确定继续？",
  "app.unsavedTitle": "未保存的修改",
  "app.copiedToast": "✓ 已复制",
  "app.renameMoveTitle": "重命名 / 移动分类",
  "app.titleLabel": "标题",
  "app.categoryLabel": "分类",
  "app.untitled": "未命名",
  "app.categoryRenameTitle": "重命名分类",
  "app.newCategoryName": "新分类名",
  "app.renameCategoryAction": "重命名分类",
  "app.resizeWindow": "调整窗口大小：{edge}",

  "app.syncInProgress": "正在同步中，请稍候再编辑",
  "app.syncNotConfigured": "同步未配置：请在设置页填写账号 / 仓库信息",
  "app.lastUploaded": "上次上传：{n} 个文件",
  "app.lastDownloaded": "上次下载：更新 {u}，跳过 {s}，清理 {d}",

  "common.cancel": "取消",
  "common.close": "关闭",
  "common.confirm": "确定",
  "common.uncategorized": "未分类",

  "category.all": "全部",
  "category.add": "新建分类",
  "category.namePlaceholder": "分类名",
  "category.dragSort": "拖拽排序",

  "prompt.dragSort": "拖拽排序",
  "prompt.moreActions": "更多操作",
  "prompt.empty": "还没有提示词",
  "prompt.emptyHint": "按 Ctrl+N 新建第一条",
  "prompt.emptySearch": "没有匹配「{query}」的提示词",
  "prompt.emptySearchHint": "换个关键词，或按 Esc 清空搜索",

  "vars.title": "填写变量",
  "vars.hint": "替换提示词中的占位符后复制；留空的变量保留原样。",
  "vars.copyRaw": "复制原文",
  "vars.copy": "复制",

  "editor.emptyTitle": "选中一条提示词查看详情",
  "editor.emptyHint": "或按 Ctrl+N 新建",
  "editor.edit": "编辑",
  "editor.reveal": "显示文件",
  "editor.delete": "删除",
  "editor.save": "保存",
  "editor.saveTitle": "保存 (Ctrl+S)",
  "editor.copyTitle": "复制 (Enter)",
  "editor.copyAria": "复制提示词",
  "editor.copyLabel": "复制",
  "editor.copyModeHint": "当前复制模式：{mode}。按 Shift+Enter 临时用另一模式复制。",
  "editor.modeMarkdown": "Markdown",
  "editor.modePlain": "纯文本",
  "editor.titleLabel": "标题",
  "editor.titlePlaceholder": "给这条提示词起个名字",
  "editor.categoryLabel": "分类",
  "editor.newCategoryName": "新分类名",
  "editor.addCategory": "+ 新建分类",
  "editor.add": "添加",
  "editor.bodyLabel": "正文",
  "editor.bodyPlaceholder": "在这里写提示词内容...支持 Markdown 语法",

  "context.rename": "重命名...",
  "context.moveToCategory": "移动到分类",
  "context.delete": "删除...",

  "settings.title": "同步设置",
  "settings.language": "界面语言",
  "settings.languageHint": "语言偏好会保存在本机，并立即应用到界面。",
  "settings.languageZh": "中文",
  "settings.languageEn": "English",
  "settings.autostart": "开机自启动",
  "settings.autostartHint": "登录系统后自动驻留后台，全局快捷键随时可用。",
  "settings.autostartFailed": "自启动设置失败：{error}",
  "settings.provider": "同步后端",
  "settings.providerWebdav": "坚果云",
  "settings.providerGithub": "GitHub 存档",
  "settings.statusSyncing": "正在同步...",
  "settings.statusNotConfigured": "未配置",
  "settings.statusError": "同步出错",
  "settings.statusWaiting": "已就绪",
  "settings.account": "坚果云账号",
  "settings.accountPlaceholder": "你的坚果云登录邮箱 / 手机号",
  "settings.appPassword": "应用密码",
  "settings.help": "如何获取？",
  "settings.passwordSaved": "✓ 已保存（无需重复输入）",
  "settings.editPassword": "修改",
  "settings.passwordPlaceholder": "在坚果云官网生成的应用密码",
  "settings.passwordHintBefore": "应用密码会本地加密保存，下次上传/下载无需重复输入。不是登录密码，需在",
  "settings.passwordHintLink": "坚果云官网 → 账户信息 → 安全选项 → 第三方应用管理",
  "settings.passwordHintAfter": "中添加应用生成。",
  "settings.remoteRoot": "远程存储路径",
  "settings.remoteRootHint": "提示词会存在坚果云的这个文件夹下。",
  "settings.ghRepo": "GitHub 仓库",
  "settings.ghRepoPlaceholder": "owner/repo，如 techdou/prompts",
  "settings.ghRepoHint": "填你已有的仓库（建议私有仓库）；仓库需已初始化（有至少一个提交）。",
  "settings.ghToken": "访问令牌（PAT）",
  "settings.ghTokenPlaceholder": "fine-grained PAT",
  "settings.ghTokenHintBefore": "令牌保存在系统凭据管理器，不落配置文件明文。建议创建 fine-grained PAT，只授权这一个仓库的 Contents 读写权限，在",
  "settings.ghTokenHintLink": "GitHub → Settings → Personal access tokens",
  "settings.ghTokenHintAfter": "创建。",
  "settings.ghBranch": "分支",
  "settings.ghBranchHint": "留空使用 main。",
  "settings.ghPrefix": "仓库内路径前缀",
  "settings.ghPrefixHint": "留空表示仓库根目录；填子目录名（如 archive）则存到该目录下。",
  "settings.fillGhRepo": "请填写仓库（owner/repo）",
  "settings.fillGhToken": "请填写访问令牌",
  "settings.ghTokenKeepReserved": "该令牌值是系统保留占位符，请换一个令牌",
  "settings.ghTestOk": "✓ 连接成功！仓库可访问且有写权限",
  "settings.manualSync": "手动同步",
  "settings.uploading": "上传中...",
  "settings.upload": "↑ 上传到{target}",

  "settings.syncUploaded": "上传完成：{n} 个文件",
  "settings.syncDeletedRemote": "云端删除 {n} 个",
  "settings.syncFailedCount": "{n} 个失败",
  "settings.syncDownloaded": "下载完成：更新 {u}，跳过 {s}，清理 {d}",
  "settings.downloading": "下载中...",
  "settings.download": "↓ 下载到本地",
  "settings.syncHint": "上传和下载均为手动操作。分歧内容保留冲突副本；覆盖前保留备份，本地独有文件保留。",
  "settings.testing": "测试中...",
  "settings.testConnection": "测试连接",
  "settings.saving": "保存中...",
  "settings.saveConfig": "保存配置",
  "settings.testOk": "✓ 连接成功！账号和应用密码有效",
  "settings.connectionFailed": "连接失败：{error}",
  "settings.fillUsername": "请填写坚果云账号",
  "settings.passwordKeepReserved": "该密码值是系统保留占位符，请换一个密码",
  "settings.fillPassword": "请填写应用密码",
  "settings.configSaved": "✓ 配置已保存",

  "reorder.needTwoPrompts": "至少需要 2 条提示词才能排序",
  "reorder.singleCategory": "切到单个分类后可拖拽排序",
  "reorder.searchDisabled": "搜索结果不支持拖拽排序",  "view.quick": "快速调用",
  "view.manage": "整理提示词",
  "view.all": "全部",
  "view.favorites": "收藏",
  "view.recent": "最近使用",
  "view.favorite": "加入收藏",
  "view.unfavorite": "取消收藏",
  "view.keyboard": "↑ ↓ 选择 · Enter 调用 · Esc 收起",
  "editor.loading": "正在读取…",
  "editor.saving": "正在保存…",
  "editor.unsaved": "尚未保存",
  "editor.leaveConfirm": "当前修改尚未保存。放弃这些修改？",
  "editor.copyMode": "复制格式",
  "editor.markdown": "Markdown 原文",
  "editor.plain": "纯文本",
  "editor.duplicate": "创建副本",
  "editor.insertSnippet": "插入常用片段",
  "editor.chooseSnippet": "选择一条提示词作为片段…",
  "editor.templateHint": "用 {{字段}} 添加变量，用 {{字段|默认值}} 设置默认值。",
  "app.pastedToast": "✓ 已复制并发送粘贴",
  "app.pasteFailedToast": "已复制；自动粘贴未完成，请手动粘贴",
  "template.raw": "复制原文",
  "template.title": "填写模板",
  "template.hint": "填写本次内容，原模板保持不变。",
  "template.preview": "本次生成结果",
  "template.apply": "使用此内容",
  "template.missing": "请填写所有变量",
  "settings.syncConflicts": "{n} 个冲突已保留为副本",
  "settings.hotkey": "全局快捷键",
  "settings.hotkeyHint": "例如 Ctrl+Alt+P。保存失败时仍保留原快捷键。",
  "settings.saveHotkey": "保存快捷键",
  "settings.hotkeySaved": "快捷键已更新",
  "settings.recovery": "恢复记录",
  "settings.recoveryHint": "删除和覆盖前的内容保存在本机。恢复时若已有同名文件，会创建副本。",
  "settings.recoveryEmpty": "还没有恢复记录",
  "settings.restore": "恢复为文件",
  "settings.restored": "已恢复：{path}",
  "settings.recoveryHistory": "历史版本",
  "settings.recoveryDeleted": "已删除",
  "settings.recoverySync": "同步备份",
  "settings.viewRecovery": "查看内容",
  "settings.transferConfirm": "下载会应用云端变更，并为覆盖内容保留恢复记录。继续下载？",
  "settings.fillCredentials": "请填写账号和应用密码",
} as const;

const en: Record<keyof typeof zh, string> = {
  "app.loading": "Loading prompts...",
  "app.searchPlaceholder": "Search titles or content...",
  "app.newPrompt": "New (Ctrl+N)",
  "app.settings": "Settings",
  "app.switchLanguageTitle": "Switch to Chinese",
  "app.switchLanguageAria": "Switch language",
  "app.syncConnected": "Nutstore connected",
  "app.syncStatusAria": "Sync status (click to open settings)",
  "app.deleteConfirm": 'Delete "{title}"? You can restore it from Settings → Recovery.',
  "app.discardChanges": "You have unsaved changes that will be lost. Continue?",
  "app.unsavedTitle": "Unsaved Changes",
  "app.copiedToast": "✓ Copied",
  "app.renameMoveTitle": "Rename / Move Category",
  "app.titleLabel": "Title",
  "app.categoryLabel": "Category",
  "app.untitled": "Untitled",
  "app.categoryRenameTitle": "Rename Category",
  "app.newCategoryName": "New category name",
  "app.renameCategoryAction": "Rename category",
  "app.resizeWindow": "Resize window: {edge}",

  "app.syncInProgress": "A sync is in progress — try again in a moment",
  "app.syncNotConfigured": "Sync is not configured (see Settings)",
  "app.lastUploaded": "Last upload: {n} file(s)",
  "app.lastDownloaded": "Last download: {u} updated, {s} skipped, {d} cleaned",

  "common.cancel": "Cancel",
  "common.close": "Close",
  "common.confirm": "OK",
  "common.uncategorized": "Uncategorized",

  "category.all": "All",
  "category.add": "New category",
  "category.namePlaceholder": "Category name",
  "category.dragSort": "Drag to reorder",

  "prompt.dragSort": "Drag to reorder",
  "prompt.moreActions": "More actions",
  "prompt.empty": "No prompts yet",
  "prompt.emptyHint": "Press Ctrl+N to create one",
  "prompt.emptySearch": 'No prompts matching "{query}"',
  "prompt.emptySearchHint": "Try another keyword, or press Esc to clear the search",

  "vars.title": "Fill in variables",
  "vars.hint": "Placeholders are substituted before copying; empty fields keep the original text.",
  "vars.copyRaw": "Copy as-is",
  "vars.copy": "Copy",

  "editor.emptyTitle": "Select a prompt to view details",
  "editor.emptyHint": "Or press Ctrl+N to create one",
  "editor.edit": "Edit",
  "editor.reveal": "Show file",
  "editor.delete": "Delete",
  "editor.save": "Save",
  "editor.saveTitle": "Save (Ctrl+S)",
  "editor.copyTitle": "Copy (Enter)",
  "editor.copyAria": "Copy prompt",
  "editor.copyLabel": "Copy",
  "editor.copyModeHint": "Copy mode: {mode}. Shift+Enter copies with the other mode temporarily.",
  "editor.modeMarkdown": "Markdown",
  "editor.modePlain": "Plain text",
  "editor.titleLabel": "Title",
  "editor.titlePlaceholder": "Name this prompt",
  "editor.categoryLabel": "Category",
  "editor.newCategoryName": "New category name",
  "editor.addCategory": "+ New category",
  "editor.add": "Add",
  "editor.bodyLabel": "Body",
  "editor.bodyPlaceholder": "Write the prompt here... Markdown is supported",

  "context.rename": "Rename...",
  "context.moveToCategory": "Move to category",
  "context.delete": "Delete...",

  "settings.title": "Sync Settings",
  "settings.language": "Interface language",
  "settings.languageHint": "Your language preference is saved on this device and applied immediately.",
  "settings.languageZh": "中文",
  "settings.languageEn": "English",
  "settings.autostart": "Launch at login",
  "settings.autostartHint": "Runs in the background after login so the global hotkey is always ready.",
  "settings.autostartFailed": "Failed to update autostart: {error}",
  "settings.provider": "Sync backend",
  "settings.providerWebdav": "Nutstore",
  "settings.providerGithub": "GitHub archive",
  "settings.statusSyncing": "Syncing...",
  "settings.statusNotConfigured": "Not configured",
  "settings.statusError": "Sync error",
  "settings.statusWaiting": "Ready",
  "settings.account": "Nutstore account",
  "settings.accountPlaceholder": "Your Nutstore email / phone number",
  "settings.appPassword": "App password",
  "settings.help": "How to get one?",
  "settings.passwordSaved": "✓ Saved (no need to enter again)",
  "settings.editPassword": "Edit",
  "settings.passwordPlaceholder": "App password generated on Nutstore",
  "settings.passwordHintBefore": "The app password is encrypted locally, so uploads/downloads will not ask again. It is not your login password. Generate one in",
  "settings.passwordHintLink": "Nutstore website → Account info → Security → Third-party app management",
  "settings.passwordHintAfter": ".",
  "settings.remoteRoot": "Remote storage path",
  "settings.remoteRootHint": "Prompts are stored in this Nutstore folder.",
  "settings.ghRepo": "GitHub repository",
  "settings.ghRepoPlaceholder": "owner/repo, e.g. techdou/prompts",
  "settings.ghRepoHint": "Use an existing repository (private recommended); it must be initialized (at least one commit).",
  "settings.ghToken": "Personal access token (PAT)",
  "settings.ghTokenPlaceholder": "Fine-grained PAT",
  "settings.ghTokenHintBefore": "The token is stored in the system credential manager, never as plaintext in the config file. Recommended: a fine-grained PAT with Contents read/write on this repo only, created in",
  "settings.ghTokenHintLink": "GitHub → Settings → Personal access tokens",
  "settings.ghTokenHintAfter": ".",
  "settings.ghBranch": "Branch",
  "settings.ghBranchHint": "Leave empty to use main.",
  "settings.ghPrefix": "Path prefix in repo",
  "settings.ghPrefixHint": "Empty means repo root; enter a subdirectory (e.g. archive) to store prompts there.",
  "settings.fillGhRepo": "Enter the repository (owner/repo)",
  "settings.fillGhToken": "Enter the access token",
  "settings.ghTokenKeepReserved": "This token value is reserved by the system, please choose another one",
  "settings.ghTestOk": "✓ Connection succeeded. Repo is accessible and writable",
  "settings.manualSync": "Manual sync",
  "settings.uploading": "Uploading...",
  "settings.upload": "↑ Upload to {target}",

  "settings.syncUploaded": "Uploaded {n} file(s)",
  "settings.syncDeletedRemote": "{n} deleted remotely",
  "settings.syncFailedCount": "{n} failed",
  "settings.syncDownloaded": "Download: {u} updated, {s} skipped, {d} cleaned",
  "settings.downloading": "Downloading...",
  "settings.download": "↓ Download to local",
  "settings.syncHint": "Transfers are manual. Conflicts keep a separate copy; replacements are backed up and local-only files are kept.",
  "settings.testing": "Testing...",
  "settings.testConnection": "Test connection",
  "settings.saving": "Saving...",
  "settings.saveConfig": "Save settings",
  "settings.testOk": "✓ Connection succeeded. Account and app password are valid",
  "settings.connectionFailed": "Connection failed: {error}",
  "settings.fillUsername": "Enter your Nutstore account",
  "settings.fillPassword": "Enter the app password",
  "settings.passwordKeepReserved": "This password value is reserved by the system, please choose another one",
  "settings.configSaved": "✓ Settings saved",

  "reorder.needTwoPrompts": "At least 2 prompts are needed to reorder",
  "reorder.singleCategory": "Switch to one category to reorder",
  "reorder.searchDisabled": "Search results cannot be reordered",  "view.quick": "Quick use",
  "view.manage": "Manage prompts",
  "view.all": "All",
  "view.favorites": "Favorites",
  "view.recent": "Recent",
  "view.favorite": "Add to favorites",
  "view.unfavorite": "Remove from favorites",
  "view.keyboard": "↑ ↓ Select · Enter Use · Esc Hide",
  "editor.loading": "Loading…",
  "editor.saving": "Saving…",
  "editor.unsaved": "Unsaved changes",
  "editor.leaveConfirm": "Discard the unsaved changes?",
  "editor.copyMode": "Copy format",
  "editor.markdown": "Markdown source",
  "editor.plain": "Plain text",
  "editor.duplicate": "Duplicate",
  "editor.insertSnippet": "Insert a snippet",
  "editor.chooseSnippet": "Choose a prompt as a snippet…",
  "editor.templateHint": "Use {{field}} for a variable or {{field|default}} for a default value.",
  "app.pastedToast": "✓ Copied and paste sent",
  "app.pasteFailedToast": "Copied; automatic paste failed. Paste manually.",
  "template.raw": "Copy original",
  "template.title": "Fill template",
  "template.hint": "Fill in values for this use. The template stays unchanged.",
  "template.preview": "Generated result",
  "template.apply": "Use this text",
  "template.missing": "Fill in all variables",
  "settings.syncConflicts": "{n} conflict(s) preserved as copies",
  "settings.hotkey": "Global shortcut",
  "settings.hotkeyHint": "For example Ctrl+Alt+P. A failed change keeps the previous shortcut.",
  "settings.saveHotkey": "Save shortcut",
  "settings.hotkeySaved": "Shortcut updated",
  "settings.recovery": "Recovery",
  "settings.recoveryHint": "Deleted and previous content is kept locally. Restoring creates a copy if the path already exists.",
  "settings.recoveryEmpty": "No recovery entries yet",
  "settings.restore": "Restore file",
  "settings.restored": "Restored: {path}",
  "settings.recoveryHistory": "Previous version",
  "settings.recoveryDeleted": "Deleted",
  "settings.recoverySync": "Sync backup",
  "settings.viewRecovery": "View content",
  "settings.transferConfirm": "Download applies cloud changes and keeps recovery copies before replacing content. Continue?",
  "settings.fillCredentials": "Enter account and app password",
};

const translations = { zh, en } as const;

export type TranslationKey = keyof typeof zh;
export type Translator = (
  key: TranslationKey,
  values?: TranslationValues,
) => string;

type LanguageStorage = Pick<Storage, "getItem" | "setItem" | "removeItem">;

export function isLanguage(value: unknown): value is Language {
  return value === "zh" || value === "en";
}

export function getStoredLanguage(
  storage: LanguageStorage | null | undefined,
): Language {
  try {
    const value = storage?.getItem(LANGUAGE_STORAGE_KEY);
    return isLanguage(value) ? value : "zh";
  } catch {
    return "zh";
  }
}

export function setStoredLanguage(
  storage: LanguageStorage | null | undefined,
  value: unknown,
): void {
  try {
    if (isLanguage(value)) {
      storage?.setItem(LANGUAGE_STORAGE_KEY, value);
    } else {
      storage?.removeItem(LANGUAGE_STORAGE_KEY);
    }
  } catch {
    /* Local storage can be unavailable in restricted webviews. */
  }
}

export function nextLanguage(language: Language): Language {
  return language === "zh" ? "en" : "zh";
}

/** 相对时间（列表副行用）：分钟/小时/天/月/年两语言就近取档 */
export function formatRelativeTime(iso: string, language: Language): string {
  const then = Date.parse(iso);
  if (!Number.isFinite(then)) return "";
  const minutes = Math.floor((Date.now() - then) / 60000);
  if (minutes < 1) return language === "zh" ? "刚刚" : "just now";
  if (minutes < 60) {
    return language === "zh" ? `${minutes} 分钟前` : `${minutes}m ago`;
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    return language === "zh" ? `${hours} 小时前` : `${hours}h ago`;
  }
  const days = Math.floor(hours / 24);
  if (days < 30) {
    return language === "zh" ? `${days} 天前` : `${days}d ago`;
  }
  const months = Math.floor(days / 30);
  if (months < 12) {
    return language === "zh" ? `${months} 个月前` : `${months}mo ago`;
  }
  const years = Math.floor(days / 365);
  return language === "zh" ? `${years} 年前` : `${years}y ago`;
}

export function translate(
  language: Language,
  key: TranslationKey,
  values: TranslationValues = {},
): string {
  const template = translations[language][key] ?? translations.zh[key] ?? key;
  return template.replace(/\{(\w+)\}/g, (match, name) => {
    const value = values[name];
    return value === undefined ? match : String(value);
  });
}

export function createTranslator(language: Language): Translator {
  return (key, values) => translate(language, key, values);
}


/**
 * 后端错误/状态码 → 本地化文案。后端只回码（SYNC_IN_PROGRESS 等），
 * 展示文案由前端按当前语言拼装；未匹配的原文返回（文件级错误详情等）
 */
export function mapBackendMessage(raw: string, t: Translator): string {
  if (raw === "SYNC_IN_PROGRESS") return t("app.syncInProgress");
  if (raw === "SYNC_NOT_CONFIGURED") return t("app.syncNotConfigured");
  const up = raw.match(/^SYNC_UPLOADED:(\d+)$/);
  if (up) return t("app.lastUploaded", { n: up[1] });
  const dl = raw.match(/^SYNC_DOWNLOADED:(\d+)\/(\d+)\/(\d+)$/);
  if (dl) return t("app.lastDownloaded", { u: dl[1], s: dl[2], d: dl[3] });
  return raw;
}
