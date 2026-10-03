export const LANGUAGE_STORAGE_KEY = "prompt-pocket.language";

export const LANGUAGES = ["zh", "en"] as const;
export type Language = (typeof LANGUAGES)[number];

type TranslationValues = Record<string, string | number>;

const zh = {
  "view.quick": "快速调用",
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
  "template.title": "填写模板",
  "template.hint": "填写本次内容，原模板保持不变。",
  "template.preview": "本次生成结果",
  "template.apply": "使用此内容",
  "template.missing": "请填写所有变量",
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
  "app.loading": "正在加载提示词...",
  "app.searchPlaceholder": "搜索提示词...",
  "app.newPrompt": "新建 (Ctrl+N)",
  "app.settings": "设置",
  "app.switchLanguageTitle": "切换到英文",
  "app.switchLanguageAria": "切换语言",
  "app.syncConnected": "已连接坚果云",
  "app.deleteConfirm": "确定删除「{title}」？可在设置的恢复记录中找回。",
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

  "settings.title": "设置",
  "settings.language": "界面语言",
  "settings.languageHint": "语言偏好会保存在本机，并立即应用到界面。",
  "settings.languageZh": "中文",
  "settings.languageEn": "English",
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
  "settings.manualSync": "手动同步",
  "settings.uploading": "上传中...",
  "settings.upload": "↑ 上传到坚果云",
  "settings.downloading": "下载中...",
  "settings.download": "↓ 下载到本地",
  "settings.syncHint": "上传和下载均为手动操作。分歧内容保留冲突副本；覆盖前保留备份，本地独有文件保留。",
  "settings.testing": "测试中...",
  "settings.testConnection": "测试连接",
  "settings.saving": "保存中...",
  "settings.saveConfig": "保存配置",
  "settings.fillCredentials": "请填写账号和应用密码",
  "settings.testOk": "✓ 连接成功！账号和应用密码有效",
  "settings.connectionFailed": "连接失败：{error}",
  "settings.fillUsername": "请填写坚果云账号",
  "settings.fillPassword": "请填写应用密码",
  "settings.configSaved": "✓ 配置已保存",

  "reorder.needTwoPrompts": "至少需要 2 条提示词才能排序",
  "reorder.singleCategory": "切到单个分类后可拖拽排序",
  "reorder.searchDisabled": "搜索结果不支持拖拽排序",
} as const;

const en: Record<keyof typeof zh, string> = {
  "view.quick": "Quick use",
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
  "template.title": "Fill template",
  "template.hint": "Fill in values for this use. The template stays unchanged.",
  "template.preview": "Generated result",
  "template.apply": "Use this text",
  "template.missing": "Fill in all variables",
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
  "app.loading": "Loading prompts...",
  "app.searchPlaceholder": "Search prompts...",
  "app.newPrompt": "New (Ctrl+N)",
  "app.settings": "Settings",
  "app.switchLanguageTitle": "Switch to Chinese",
  "app.switchLanguageAria": "Switch language",
  "app.syncConnected": "Nutstore connected",
  "app.deleteConfirm": 'Delete "{title}"? You can restore it from Settings.',
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

  "settings.title": "Settings",
  "settings.language": "Interface language",
  "settings.languageHint": "Your language preference is saved on this device and applied immediately.",
  "settings.languageZh": "中文",
  "settings.languageEn": "English",
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
  "settings.manualSync": "Manual sync",
  "settings.uploading": "Uploading...",
  "settings.upload": "↑ Upload to Nutstore",
  "settings.downloading": "Downloading...",
  "settings.download": "↓ Download to local",
  "settings.syncHint": "Transfers are manual. Conflicts keep a separate copy; replacements are backed up and local-only files are kept.",
  "settings.testing": "Testing...",
  "settings.testConnection": "Test connection",
  "settings.saving": "Saving...",
  "settings.saveConfig": "Save settings",
  "settings.fillCredentials": "Enter account and app password",
  "settings.testOk": "✓ Connection succeeded. Account and app password are valid",
  "settings.connectionFailed": "Connection failed: {error}",
  "settings.fillUsername": "Enter your Nutstore account",
  "settings.fillPassword": "Enter the app password",
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
