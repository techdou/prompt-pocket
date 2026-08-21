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
  "app.deleteConfirm": "确定删除「{title}」？文件会移入 .trash 备份目录。",
  "app.discardChanges": "当前有未保存的修改，切换后将丢失。确定继续？",
  "app.unsavedTitle": "未保存的修改",
  "app.copiedToast": "✓ 已复制，回到原应用粘贴",
  "app.renameMoveTitle": "重命名 / 移动分类",
  "app.titleLabel": "标题",
  "app.categoryLabel": "分类",
  "app.untitled": "未命名",
  "app.categoryRenameTitle": "重命名分类",
  "app.newCategoryName": "新分类名",
  "app.renameCategoryAction": "重命名分类",
  "app.resizeWindow": "调整窗口大小：{edge}",

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
  "prompt.empty": "没有匹配的提示词",

  "editor.emptyTitle": "选中一条提示词查看详情",
  "editor.emptyHint": "或按 Ctrl+N 新建",
  "editor.edit": "编辑",
  "editor.reveal": "显示文件",
  "editor.delete": "删除",
  "editor.save": "保存",
  "editor.copyTitle": "复制 (Enter)",
  "editor.copyAria": "复制提示词",
  "editor.copyLabel": "复制",
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
  "settings.statusWaiting": "已配置，等待同步",
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
  "settings.downloading": "下载中...",
  "settings.download": "↓ 下载到本地",
  "settings.syncHint": "上传：本地文件推送到远端并传播删除。下载：远端覆盖本地（覆盖前备份到 .trash）。",
  "settings.testing": "测试中...",
  "settings.testConnection": "测试连接",
  "settings.saving": "保存中...",
  "settings.saveConfig": "保存配置",
  "settings.fillCredentials": "请填写账号和应用密码",
  "settings.testOk": "✓ 连接成功！账号和应用密码有效",
  "settings.connectionFailed": "连接失败：{error}",
  "settings.fillUsername": "请填写坚果云账号",
  "settings.passwordKeepReserved": "该密码值是系统保留占位符，请换一个密码",
  "settings.fillPassword": "请填写应用密码",
  "settings.configSaved": "✓ 配置已保存",

  "reorder.needTwoPrompts": "至少需要 2 条提示词才能排序",
  "reorder.singleCategory": "切到单个分类后可拖拽排序",
  "reorder.searchDisabled": "搜索结果不支持拖拽排序",
} as const;

const en: Record<keyof typeof zh, string> = {
  "app.loading": "Loading prompts...",
  "app.searchPlaceholder": "Search titles or content...",
  "app.newPrompt": "New (Ctrl+N)",
  "app.settings": "Settings",
  "app.switchLanguageTitle": "Switch to Chinese",
  "app.switchLanguageAria": "Switch language",
  "app.syncConnected": "Nutstore connected",
  "app.deleteConfirm": 'Delete "{title}"? The file will be moved to the .trash backup folder.',
  "app.discardChanges": "You have unsaved changes that will be lost. Continue?",
  "app.unsavedTitle": "Unsaved Changes",
  "app.copiedToast": "✓ Copied. Return to the previous app to paste.",
  "app.renameMoveTitle": "Rename / Move Category",
  "app.titleLabel": "Title",
  "app.categoryLabel": "Category",
  "app.untitled": "Untitled",
  "app.categoryRenameTitle": "Rename Category",
  "app.newCategoryName": "New category name",
  "app.renameCategoryAction": "Rename category",
  "app.resizeWindow": "Resize window: {edge}",

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
  "prompt.empty": "No matching prompts",

  "editor.emptyTitle": "Select a prompt to view details",
  "editor.emptyHint": "Or press Ctrl+N to create one",
  "editor.edit": "Edit",
  "editor.reveal": "Show file",
  "editor.delete": "Delete",
  "editor.save": "Save",
  "editor.copyTitle": "Copy (Enter)",
  "editor.copyAria": "Copy prompt",
  "editor.copyLabel": "Copy",
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
  "settings.statusWaiting": "Configured, waiting to sync",
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
  "settings.downloading": "Downloading...",
  "settings.download": "↓ Download to local",
  "settings.syncHint": "Upload pushes local files to the remote and propagates deletions. Download lets the remote overwrite local files (overwritten files are backed up to .trash first).",
  "settings.testing": "Testing...",
  "settings.testConnection": "Test connection",
  "settings.saving": "Saving...",
  "settings.saveConfig": "Save settings",
  "settings.fillCredentials": "Enter account and app password",
  "settings.testOk": "✓ Connection succeeded. Account and app password are valid",
  "settings.connectionFailed": "Connection failed: {error}",
  "settings.fillUsername": "Enter your Nutstore account",
  "settings.fillPassword": "Enter the app password",
  "settings.passwordKeepReserved": "This password value is reserved by the system, please choose another one",
  "settings.configSaved": "✓ Settings saved",

  "reorder.needTwoPrompts": "At least 2 prompts are needed to reorder",
  "reorder.singleCategory": "Switch to one category to reorder",
  "reorder.searchDisabled": "Search results cannot be reordered",
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
