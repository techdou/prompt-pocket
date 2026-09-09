<script lang="ts">
  import { onMount, tick } from "svelte";
  import { fly } from "svelte/transition";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { ask } from "@tauri-apps/plugin-dialog";
  import type { CategoryCount, Prompt, PromptMeta, SyncStatus } from "./lib/types";
  import {
    copyOrPaste,
    createCategory,
    createPrompt,
    deletePrompt,
    getSyncStatus,
    hideWindow,
    initApp,
    readPrompt,
    renamePrompt,
    renameCategory,
    reorderCategories,
    reorderPrompts,
    revealInFinder,
    savePrompt,
    scanPrompts,
  } from "./lib/api";
  import { filterPrompts } from "./lib/search";
  import { markdownToPlain } from "./lib/markdown";
  import { applyVariables, extractVariables } from "./lib/variables";
  import CategoryTabs from "./lib/CategoryTabs.svelte";
  import PromptList from "./lib/PromptList.svelte";
  import Editor from "./lib/Editor.svelte";
  import Settings from "./lib/Settings.svelte";
  import ContextMenu from "./lib/ContextMenu.svelte";
  import VariableDialog from "./lib/VariableDialog.svelte";
  import { autofocus } from "./lib/actions";
  import {
    canReorderPromptList,
    getReorderCategory,
    getReorderDisabledReason,
    moveCategoryOrder,
    movePathOrder,
  } from "./lib/reorder";
  import type { ReorderDisabledReason } from "./lib/reorder";
  import {
    createTranslator,
    getStoredLanguage,
    nextLanguage,
    setStoredLanguage,
    type Language,
  } from "./lib/i18n";

  let allPrompts: Prompt[] = $state([]);
  let categories: CategoryCount[] = $state([]);
  let syncStatus: SyncStatus | null = $state(null);

  let selectedCategory = $state<string>("__all__");
  let query = $state("");
  let selectedPath = $state<string | null>(null);

  // 编辑器状态（结构化字段，双向绑定到 Editor）
  let editorMode = $state<"view" | "edit">("view");
  let editingBody = $state("");
  let editingTitle = $state("");
  let editingCategory = $state("");
  let editingCopyMode = $state<"markdown" | "plain">("markdown");

  // 加载/保存成功时的内容快照：脏检查基准（编辑字段偏离快照 = 有未保存修改）
  let loadedSnapshot = $state<{
    body: string;
    title: string;
    category: string;
    copyMode: "markdown" | "plain";
  } | null>(null);

  let isDirty = $derived(
    editorMode === "edit" &&
      loadedSnapshot !== null &&
      (editingBody !== loadedSnapshot.body ||
        editingTitle !== loadedSnapshot.title ||
        editingCategory !== loadedSnapshot.category ||
        editingCopyMode !== loadedSnapshot.copyMode),
  );

  /** 有未保存修改时弹确认；返回 true 表示可以继续（不脏或用户确认丢弃） */
  async function confirmDiscardIfDirty(): Promise<boolean> {
    if (!isDirty) return true;
    return ask(t("app.discardChanges"), {
      title: t("app.unsavedTitle"),
      kind: "warning",
    });
  }

  let loading = $state(true);
  let error = $state<string | null>(null);
  let language = $state<Language>("zh");
  let t = $derived(createTranslator(language));

  // 统一错误提示：显示后 5 秒自动消失，不阻塞 UI
  function showError(msg: string) {
    error = msg;
    setTimeout(() => {
      if (error === msg) error = null;
    }, 5000);
  }
  let copiedFlash = $state(false);
  let settingsOpen = $state(false);

  // 右键菜单 + 重命名对话框
  let contextMenu = $state({ open: false, x: 0, y: 0, prompt: null as Prompt | null });
  let renameDialog = $state({
    open: false,
    path: "",
    title: "",
    category: "",
  });

  // 分类右键菜单 + 分类重命名
  let catContextMenu = $state({ open: false, x: 0, y: 0, name: "" });
  let catRenameDialog = $state({ open: false, oldName: "", newName: "" });

  let selectedPrompt = $derived(
    allPrompts.find((p) => p.path === selectedPath) ?? null,
  );

  let categoryFiltered = $derived(
    selectedCategory === "__all__"
      ? allPrompts
      : allPrompts.filter((p) => p.category === selectedCategory),
  );

  let visiblePrompts = $derived(filterPrompts(categoryFiltered, query));
  let canReorderPrompts = $derived(
    canReorderPromptList(query, selectedCategory, visiblePrompts),
  );
  let reorderDisabledReason = $derived(
    getReorderDisabledReason(query, selectedCategory, visiblePrompts),
  );
  let reorderDisabledLabel = $derived(
    translateReorderDisabledReason(reorderDisabledReason, language),
  );

  let selectedIndex = $derived(
    selectedPath
      ? visiblePrompts.findIndex((p) => p.path === selectedPath)
      : -1,
  );
  // PromptList 上报的滚动函数（键盘导航用）
  let scrollToIndexFn: ((i: number) => void) | null = null;

  async function bootstrap() {
    try {
      loading = true;
      await initApp();
      await refresh();
      try {
        syncStatus = await getSyncStatus();
      } catch {
        /* 同步状态获取失败不阻断 */
      }
    } catch (e) {
      showError(String(e));
    } finally {
      loading = false;
      // 首启窗口显示早于 webview JS 就绪（Rust 侧不为此 emit window-shown，
      // 监听未注册必丢）：loading 翻 false、DOM 渲染出搜索框后直接聚焦，
      // 兑现首启直接打字
      void tick().then(() => {
        if (editorMode === "view") {
          document.querySelector<HTMLInputElement>("#search-input")?.focus();
        }
      });
    }
  }

  function getLanguageStorage(): Storage | null {
    return typeof window === "undefined" ? null : window.localStorage;
  }

  function changeLanguage(next: Language) {
    language = next;
    setStoredLanguage(getLanguageStorage(), next);
  }

  function toggleLanguage() {
    changeLanguage(nextLanguage(language));
  }

  // reason code → i18n key 的直接映射（reorder.ts 不再返回中文文案）
  function translateReorderDisabledReason(
    reason: ReorderDisabledReason,
    lang: Language,
  ): string {
    if (!reason) return "";
    const translateFor = createTranslator(lang);
    switch (reason) {
      case "needTwo":
        return translateFor("reorder.needTwoPrompts");
      case "singleCategory":
        return translateFor("reorder.singleCategory");
      case "searchDisabled":
        return translateFor("reorder.searchDisabled");
    }
  }

  async function refresh() {
    // 更新前 capture 选中在过滤视图里的位置：列表更新后 selectedIndex 读到
    // 的是新列表（选中已不在其中，恒 -1），要旧位置才能兑现"选相邻项"
    const prevIdx = selectedIndex;
    const res = await scanPrompts();
    allPrompts = res.prompts;
    categories = res.categories;
    reconcileSelection(prevIdx);
  }

  // 刷新后的选中调和：selectedPath 还在全局列表 → 保持；
  // 不在了（外部删除/同步清理）→ 选可见列表里同位置的相邻项。
  // 这是 selectedPath 唯一真相原则的关键：任何重排都不能劫持选中。
  function reconcileSelection(prevIdx = 0) {
    if (!selectedPath) {
      // 无选中（首启/删除后）：列表非空时默认选第一条，保证 Enter 始终可用
      if (visiblePrompts.length > 0) selectedPath = visiblePrompts[0].path;
      return;
    }
    if (allPrompts.some((p) => p.path === selectedPath)) return;
    const idx = prevIdx >= 0 ? prevIdx : 0;
    const next = visiblePrompts[Math.min(idx, visiblePrompts.length - 1)];
    selectedPath = next?.path ?? null;
  }

  // 过滤条件变化后的选中调和：搜索/切分类后，当前选中项不在可见列表时自动
  // 高亮第一条（Raycast/Alfred 惯例：结果列表永远有默认选中项，搜完 Enter 永远有效）。
  // 只由过滤条件驱动——列表数据刷新（保存/同步/拖拽）走 reconcileSelection，
  // 不会劫持用户正在查看或编辑的选中项。首跑只记录基准不动作（首启由 refresh 负责）。
  // 编辑态跳过：切分类/搜索时 CategoryTabs 与搜索框没有脏确认入口，
  // 此处移动选中会让 loadPromptContent 无确认覆盖编辑中的内容（旧版注释
  // 警告过的 effect 反写劫持）；编辑态维持选中不动，退出编辑后自然恢复。
  let lastFilterKey: string | null = null;
  $effect(() => {
    const key = `${selectedCategory}\u0000${query}`;
    if (lastFilterKey === null || key === lastFilterKey) {
      lastFilterKey = key;
      return;
    }
    lastFilterKey = key;
    if (editorMode === "edit") return;
    if (visiblePrompts.length === 0 || selectedIndex >= 0) return;
    selectedPath = visiblePrompts[0].path;
  });

  // 设置界面切换数据目录后：更新配置、重置选中、重新扫描
  // 同步完成后：重新加载列表 + 刷新同步状态
  async function onSynced() {
    await guardedRefresh();
    try {
      syncStatus = await getSyncStatus();
    } catch {
      /* 忽略 */
    }
  }

  // 拖拽重排进行中标志：重排把新顺序写盘前，若 sync-finished
  // 抵达并触发 refresh()，会读到旧 order 文件把刚拖的顺序冲掉。
  // 用该标志让写盘期间的 refresh 延迟到写盘完成后，避免竞态。
  let reorderInFlight = false;
  let pendingRefresh = false;
  // 拖拽手势期间（pointerdown 到 pointerup）同样挂起 refresh：
  // 手势中途列表被重绘会打断落点计算，手势结束后统一补刷。
  let dragGestureActive = false;

  function onDragGestureStart() {
    dragGestureActive = true;
  }

  function onDragGestureEnd() {
    dragGestureActive = false;
    // 手势期间攒下的补刷：写盘不在飞时才自己刷（在飞则由 doReorder 的 finally 刷）
    if (pendingRefresh && !reorderInFlight) {
      pendingRefresh = false;
      void refresh().catch((e) => showError(String(e)));
    }
  }

  async function guardedRefresh() {
    if (reorderInFlight || dragGestureActive) {
      // 重排写盘/拖拽手势中：标记需要补刷，等结束后自己刷
      pendingRefresh = true;
      return;
    }
    await refresh();
  }

  // 监听后端 sync-finished 事件，自动刷新
  $effect(() => {
    let unlisten: (() => void) | null = null;
    let disposed = false;
    import("@tauri-apps/api/event")
      .then(({ listen }) => {
        // cleanup 已跑（dev HMR 卸载）：不再注册
        if (disposed) return;
        listen("sync-finished", () => {
          void guardedRefresh().catch((e) => showError(String(e)));
          void getSyncStatus()
            .then((s) => (syncStatus = s))
            .catch(() => {});
        }).then((fn) => {
          // 注册完成时组件已卸载：立即反注册，防止监听器泄漏
          if (disposed) fn();
          else unlisten = fn;
        });
      })
      .catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  });

  // 唤出（快捷键/托盘/单实例/首启）后把焦点放进搜索框并全选残留词：
  // 「唤出→直接打字」核心链路的最后一环。弹窗/设置开着时不抢焦点（焦点归弹窗）。
  // 普通点击窗口获得焦点不触发该事件，用户点回窗口继续编辑不会被劫持。
  $effect(() => {
    let unlisten: (() => void) | null = null;
    let disposed = false;
    import("@tauri-apps/api/event")
      .then(({ listen }) => {
        if (disposed) return;
        listen("window-shown", () => {
          const overlayOpen =
            variableDialog.open ||
            renameDialog.open ||
            catRenameDialog.open ||
            contextMenu.open ||
            catContextMenu.open ||
            settingsOpen;
          if (overlayOpen) return;
          // 编辑态唤出：焦点回正文 textarea（唤出前多半就在编辑，
          // 抢到搜索框会把打字引进搜索框，连带触发过滤劫持编辑内容）
          if (editorMode === "edit") {
            document.querySelector<HTMLTextAreaElement>("#f-body")?.focus();
            return;
          }
          const input = document.querySelector<HTMLInputElement>("#search-input");
          if (!input) return;
          input.focus();
          // 全选残留搜索词：唤出后直接打字即覆盖，无需逐字删除
          input.select();
        }).then((fn) => {
          if (disposed) fn();
          else unlisten = fn;
        });
      })
      .catch(() => {});
    return () => {
      disposed = true;
      unlisten?.();
    };
  });

  // 选中变化时加载内容。
  // lastLoadedPath 只做"防重复发起"；loadedPath 才表示"内容已就绪"——
  // 两者必须分开：effect 在发起时就置 lastLoadedPath（同步），
  // 若 doCopy 用它判断会把"加载在飞"误判成"已加载"，复制到上一条内容
  let lastLoadedPath: string | null = null;
  let loadedPath: string | null = null;
  let loadToken = 0;
  $effect(() => {
    if (selectedPath && selectedPath !== lastLoadedPath) {
      lastLoadedPath = selectedPath;
      void loadPromptContent(selectedPath);
    }
  });

  async function loadPromptContent(path: string) {
    // 竞态防护：快速连续切换时，旧响应不得覆盖新选中项的内容
    const token = ++loadToken;
    try {
      const { meta, body } = await readPrompt(path);
      if (token !== loadToken || path !== selectedPath) return;
      applyMetaToEditFields(meta);
      editingBody = body;
      loadedPath = path;
      loadedSnapshot = {
        body,
        title: meta.title,
        category: selectedPrompt?.category ?? "未分类",
        copyMode: meta.copy_mode === "plain" ? "plain" : "markdown",
      };
      editorMode = "view";
    } catch (e) {
      // 与 try 分支同样的双保险：失败响应到达时选中可能已切换/新建，
      // 响应只对"发起加载那一刻仍是当前选中"的路径生效
      if (token !== loadToken || path !== selectedPath) return;
      const msg = String(e);
      if (msg.includes("FILE_NOT_FOUND")) {
        // 问题1：文件被外部删除 → 从列表移除，不报错卡死
        removePromptFromList(path);
      } else {
        showError(msg);
      }
    }
  }

  function applyMetaToEditFields(meta: PromptMeta) {
    editingTitle = meta.title;
    editingCategory = selectedPrompt?.category ?? "未分类";
    editingCopyMode = (meta.copy_mode === "plain" ? "plain" : "markdown");
  }

  // 问题1：从列表移除已删除的 prompt，自动选中相邻项
  function removePromptFromList(path: string) {
    const idx = allPrompts.findIndex((p) => p.path === path);
    if (idx >= 0) {
      allPrompts = allPrompts.filter((p) => p.path !== path);
      // 重新选相邻项
      const next = allPrompts[Math.min(idx, allPrompts.length - 1)];
      if (next) {
        selectedPath = next.path;
        lastLoadedPath = null; // 强制重新加载
        loadedPath = null;
      } else {
        selectedPath = null;
        lastLoadedPath = null;
        loadedPath = null;
      }
    }
    // 刷新分类计数（走 guarded 版本：拖拽手势/写盘中挂起，结束后补刷）
    void guardedRefresh().catch((e) => showError(String(e)));
  }

  // hideAfter：Enter 快捷键链路 = true（复制后隐藏窗口回到原应用）；
  // 主窗口内点「复制」按钮 = false（窗口保持可见，方便连续复制/继续浏览）
  async function doCopy(mode: "markdown" | "plain", hideAfter = true) {
    if (!selectedPrompt) return;
    try {
      // 内容加载是异步的：快速 ↓↓ 切换后按 Enter，editingBody 可能还是上一条的。
      // 用"已就绪"标记判断（loadedPath 只在加载成功时置位），未就绪则按路径
      // 直接读盘，保证复制内容与选中项一致
      let body = editingBody;
      if (loadedPath !== selectedPrompt.path) {
        const content = await readPrompt(selectedPrompt.path);
        body = content.body;
      }
      // 正文含 {{占位符}}：先弹窗填空再复制（替换发生在 plain 剥离之前，
      // 用户填的值以原文参与后续转换）
      const vars = extractVariables(body);
      if (vars.length > 0) {
        variableDialog = { open: true, variables: vars, body, mode, hideAfter };
        return;
      }
      await finishCopy(body, mode, hideAfter);
    } catch (e) {
      showError(String(e));
    }
  }

  // 复制的最终段：plain 模式剥掉 Markdown 标记 → 写剪贴板 → 隐藏窗口 →
  // 按快捷键来源决定是否注入 Ctrl+V（copyOrPaste 内部处理）
  async function finishCopy(
    body: string,
    mode: "markdown" | "plain",
    hideAfter = true,
  ) {
    try {
      if (mode === "plain") body = markdownToPlain(body);
      await copyOrPaste(body, mode, hideAfter);
      if (!hideAfter) {
        // 窗口内复制：窗口不隐藏，toast 是唯一的确认反馈
        copiedFlash = true;
        setTimeout(() => (copiedFlash = false), 800);
      }
    } catch (e) {
      showError(String(e));
    }
  }

  // 变量填空弹窗：正文含 {{占位符}} 时复制前先填写
  let variableDialog = $state<{
    open: boolean;
    variables: string[];
    body: string;
    mode: "markdown" | "plain";
    hideAfter: boolean;
  }>({ open: false, variables: [], body: "", mode: "markdown", hideAfter: true });

  function closeVarDialog() {
    variableDialog = { ...variableDialog, open: false };
  }

  function onVarsConfirm(values: Record<string, string>) {
    const { body, mode, hideAfter } = variableDialog;
    closeVarDialog();
    void finishCopy(applyVariables(body, values), mode, hideAfter);
  }

  // 逃生门：正文恰好天然含 {{}}（如模板示例）时跳过替换，按原文复制
  function onVarsCopyRaw() {
    const { body, mode, hideAfter } = variableDialog;
    closeVarDialog();
    void finishCopy(body, mode, hideAfter);
  }

  // 问题5修复：保存用结构化字段，Rust 端规范序列化
  async function doSave() {
    if (!selectedPrompt) return;
    try {
      const saved = await savePrompt(selectedPrompt.path, {
        title: editingTitle.trim() || t("app.untitled"),
        copy_mode: editingCopyMode,
        body: editingBody,
        category: editingCategory,
      });
      // 保存可能因标题/分类变化而移动了文件，用新路径更新选中
      selectedPath = saved.path;
      lastLoadedPath = saved.path; // 避免立即重载覆盖编辑内容
      loadedPath = saved.path; // 保存的内容已就绪，doCopy 无需重读
      // 快照同步到已保存状态（保存后不再是脏的）
      loadedSnapshot = {
        body: editingBody,
        title: editingTitle,
        category: saved.category,
        copyMode: editingCopyMode,
      };
      editingCategory = saved.category;
      await guardedRefresh(); // reconcile 保住 selectedPath，不被重排劫持
      editorMode = "view";
    } catch (e) {
      const msg = String(e);
      if (msg.includes("FILE_NOT_FOUND")) {
        removePromptFromList(selectedPrompt.path);
      } else {
        showError(msg);
      }
    }
  }

  async function doCreate() {
    // 当前编辑有未保存修改时先确认（新建会重置全部编辑字段）
    if (!(await confirmDiscardIfDirty())) return;
    const cat = selectedCategory === "__all__" ? "未分类" : selectedCategory;
    try {
      // 新建用占位标题（文件名是时间戳），进入编辑后用户填写真实标题
      // 保存时若标题变化会自动重命名文件
      const p = await createPrompt(cat, "");
      await guardedRefresh();
      selectedPath = p.path;
      lastLoadedPath = p.path;
      loadedPath = p.path; // 新建内容（空）已就绪
      query = "";
      // 进入编辑，标题留空引导用户输入
      editingTitle = "";
      editingCategory = cat;
      editingCopyMode = "markdown";
      editingBody = "";
      // 新建即快照基准：用户开始输入才变脏
      loadedSnapshot = { body: "", title: "", category: cat, copyMode: "markdown" };
      editorMode = "edit";
    } catch (e) {
      showError(String(e));
    }
  }

  async function doDelete() {
    if (!selectedPrompt) return;
    const ok = await ask(t("app.deleteConfirm", { title: selectedPrompt.title }), {
      title: t("editor.delete"),
      kind: "warning",
    });
    if (!ok) return;
    try {
      await deletePrompt(selectedPrompt.path);
      selectedPath = null;
      lastLoadedPath = null;
      loadedPath = null;
      loadedSnapshot = null;
      await guardedRefresh();
    } catch (e) {
      showError(String(e));
    }
  }

  // 取消编辑：丢弃修改，从磁盘重载当前选中项
  function cancelEdit() {
    if (selectedPath) {
      lastLoadedPath = null;
      loadedPath = null;
      void loadPromptContent(selectedPath);
    } else {
      editorMode = "view";
    }
  }

  // 问题2：右键菜单操作
  function openContextMenu(prompt: Prompt, x: number, y: number) {
    contextMenu = { open: true, x, y, prompt };
  }

  function onCtxRename() {
    if (!contextMenu.prompt) return;
    renameDialog = {
      open: true,
      path: contextMenu.prompt.path,
      title: contextMenu.prompt.title,
      category: contextMenu.prompt.category,
    };
  }

  async function onCtxMove(category: string) {
    if (!contextMenu.prompt) return;
    const target = contextMenu.prompt;
    try {
      const moved = await renamePrompt(target.path, target.title, category);
      // 移动的就是当前选中项：跟随新路径并保留编辑内容（不重载），
      // 因此无需脏确认——未保存修改原样留在编辑器里
      if (target.path === selectedPath) {
        selectedPath = moved.path;
        lastLoadedPath = moved.path;
        loadedPath = moved.path;
        editingCategory = moved.category;
        if (loadedSnapshot) {
          loadedSnapshot = { ...loadedSnapshot, category: moved.category };
        }
      }
      await guardedRefresh();
    } catch (e) {
      showError(String(e));
    }
  }

  // 拖拽排序：原生 DnD 直接给出 from/to（基于当前分类列表的索引）。
  // from = 被拖项索引，to = 目标插入点（移动后插到该 index 之前，允许等于 length）。
  // 这里基于 visiblePrompts 重排得到新顺序，更新该分类各 prompt 的 order 字段，
  // 然后对整个 allPrompts 稳定重排（保持全局 category 字母序，避免「全部」视图闪错序）。
  async function doReorder(from: number, to: number) {
    // 并发防护：上一次写盘未完成时忽略二次拖拽提交，防止交错写 order 文件
    if (reorderInFlight) return;
    if (query.trim()) return;
    const categoryName = getReorderCategory(selectedCategory, visiblePrompts);
    if (!categoryName) return;
    const newPathOrder = movePathOrder(visiblePrompts, from, to);
    if (!newPathOrder) return;

    // 乐观更新：按新顺序给该分类各项赋 order，再全局稳定排序
    // （category 字母序 → order 升序 → updated 倒序，与后端 scan_prompts 一致）
    const orderMap = new Map(newPathOrder.map((path, i) => [path, i]));
    allPrompts = allPrompts
      .map((p) =>
        p.category === categoryName
          ? { ...p, order: orderMap.has(p.path) ? orderMap.get(p.path) : undefined }
          : p,
      )
      .sort((a, b) => {
        // 码点序与后端 scan_prompts 的 String::cmp 对齐（localeCompare 是本地化
        // 拼音序，中文结果与码点序完全不同，会导致"全部"视图乐观排序跳变）。
        // 已知取舍：emoji 等非 BMP 字符按 UTF-16 码元比较与 Rust 码点序仍可能
        // 相反，极端场景（分类名含 emoji）接受一次刷新跳变
        const c = a.category < b.category ? -1 : a.category > b.category ? 1 : 0;
        if (c !== 0) return c;
        const oa = a.order ?? Number.MAX_SAFE_INTEGER;
        const ob = b.order ?? Number.MAX_SAFE_INTEGER;
        if (oa !== ob) return oa - ob;
        return b.meta.updated < a.meta.updated
          ? -1
          : b.meta.updated > a.meta.updated
            ? 1
            : 0;
      });

    reorderInFlight = true;
    pendingRefresh = false;
    try {
      await reorderPrompts(categoryName, newPathOrder);
    } catch (e) {
      showError(String(e));
      await refresh().catch((e2) => showError(String(e2)));
    } finally {
      reorderInFlight = false;
      if (pendingRefresh) {
        pendingRefresh = false;
        // 补刷失败要走 showError，裸 await 的 rejection 会从 onreorder
        // 回调逃逸成 unhandled rejection（其余 refresh 调用点都带 catch）
        await refresh().catch((e) => showError(String(e)));
      }
    }
  }

  // 分类拖拽重排：from/to 都是 categories 数组索引（不含"全部"）。
  // 乐观更新本地顺序 → 写盘 .category-order.json。失败回滚靠 refresh 重读后端。
  async function doReorderCategory(from: number, to: number) {
    if (reorderInFlight) return; // 同 doReorder：写盘期间忽略二次提交
    const next = moveCategoryOrder(categories, from, to);
    if (!next) return;

    categories = next;
    reorderInFlight = true;
    pendingRefresh = false;
    try {
      await reorderCategories(next.map((c) => c.name));
    } catch (e) {
      showError(String(e));
      await refresh().catch((e2) => showError(String(e2)));
    } finally {
      reorderInFlight = false;
      if (pendingRefresh) {
        pendingRefresh = false;
        // 补刷失败要走 showError，裸 await 的 rejection 会从 onreorder
        // 回调逃逸成 unhandled rejection（其余 refresh 调用点都带 catch）
        await refresh().catch((e) => showError(String(e)));
      }
    }
  }

  async function onCtxDelete() {
    if (!contextMenu.prompt) return;
    const p = contextMenu.prompt;
    const ok = await ask(t("app.deleteConfirm", { title: p.title }), {
      title: t("editor.delete"),
      kind: "warning",
    });
    if (!ok) return;
    try {
      await deletePrompt(p.path);
      if (selectedPath === p.path) {
        selectedPath = null;
        lastLoadedPath = null;
        loadedPath = null;
        loadedSnapshot = null;
      }
      await guardedRefresh();
    } catch (e) {
      showError(String(e));
    }
  }

  // 重命名对话框提交
  async function submitRename() {
    // 重命名当前正在编辑的项会强制重载内容，未保存修改先确认
    if (renameDialog.path === selectedPath && !(await confirmDiscardIfDirty())) {
      return;
    }
    try {
      const newPrompt = await renamePrompt(
        renameDialog.path,
        renameDialog.title.trim() || t("app.untitled"),
        renameDialog.category,
      );
      await guardedRefresh();
      selectedPath = newPrompt.path;
      lastLoadedPath = null;
      loadedPath = null;
      renameDialog.open = false;
    } catch (e) {
      showError(String(e));
    }
  }

  // 问题3：新建分类
  async function onCreateCategory(name: string) {
    try {
      await createCategory(name);
      await guardedRefresh();
    } catch (e) {
      showError(String(e));
    }
  }

  // 优化3：分类右键菜单
  function onCatContextMenu(name: string, x: number, y: number) {
    catContextMenu = { open: true, x, y, name };
  }

  // 优化3：重命名分类
  async function onRenameCategory(oldName: string) {
    catRenameDialog = { open: true, oldName, newName: oldName };
  }

  async function submitCatRename() {
    const newName = catRenameDialog.newName.trim();
    if (!newName || newName === catRenameDialog.oldName) {
      catRenameDialog.open = false;
      return;
    }
    try {
      await renameCategory(catRenameDialog.oldName, newName);
      if (selectedCategory === catRenameDialog.oldName) {
        selectedCategory = newName;
      }
      await guardedRefresh();
      catRenameDialog.open = false;
    } catch (e) {
      showError(String(e));
    }
  }

  // 键盘导航：↑↓ 直接驱动 selectedPath（事件驱动，不做响应式反写——
  // 旧版用 effect 持续把 selectedIndex 反写进 selectedPath，
  // 保存重排/新建/同步刷新都会劫持选中并覆盖正在编辑的内容）
  async function navigateSelection(delta: number) {
    if (visiblePrompts.length === 0) return;
    if (!(await confirmDiscardIfDirty())) return;
    const cur = selectedIndex >= 0 ? selectedIndex : delta > 0 ? -1 : 0;
    const next = Math.min(Math.max(cur + delta, 0), visiblePrompts.length - 1);
    if (next === selectedIndex) return;
    selectedPath = visiblePrompts[next].path;
    scrollIntoView();
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.defaultPrevented) return;
    if (e.key === "Escape") {
      // 拖拽手势中 Esc：先结束手势（复位标志 + 补刷挂起的刷新）。
      // 否则窗口隐藏后 pointerup 丢失，dragGestureActive 滞留 true，
      // 后续 refresh 全部被挂起直到下次鼠标事件才自愈
      if (dragGestureActive) onDragGestureEnd();
      if (contextMenu.open) {
        contextMenu.open = false;
        return;
      }
      // 变量弹窗优先于其他弹窗关闭（backdrop 已拦截聚焦态的 Esc，这里是
      // 焦点不在弹窗内时的兜底，防止关弹窗的同时隐藏整个窗口）
      if (variableDialog.open) {
        closeVarDialog();
        return;
      }
      if (catContextMenu.open) {
        catContextMenu.open = false;
        return;
      }
      if (renameDialog.open) {
        renameDialog.open = false;
        return;
      }
      if (catRenameDialog.open) {
        catRenameDialog.open = false;
        return;
      }
      if (settingsOpen) {
        settingsOpen = false;
        return;
      }
      // 编辑态 Esc：先确认丢弃未保存修改，再退出编辑（回到 view 并重载磁盘内容）
      if (editorMode === "edit") {
        void confirmDiscardIfDirty().then((ok) => {
          if (ok) cancelEdit();
        });
        return;
      }
      // 裸视图下 Esc：搜索框有词先清词（一次 Esc 回到全列表），再按才隐藏窗口
      if (query.trim()) {
        query = "";
        e.preventDefault();
        return;
      }
      e.preventDefault();
      void hideWindow();
      return;
    }
    // 变量填空弹窗打开：挂起其余全局快捷键（Ctrl+N/F、列表导航），
    // 防止隔空操作背后的列表/编辑器（Esc 已在上方分支处理）
    if (variableDialog.open) return;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "n") {
      e.preventDefault();
      void doCreate();
      return;
    }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") {
      e.preventDefault();
      document.querySelector<HTMLInputElement>("#search-input")?.focus();
      return;
    }
    // 编辑态 Ctrl/Cmd+S：保存。必须在下方输入控件早退之前处理——
    // 否则正文 textarea 里的 Ctrl+S 永远到不了这里（键盘优先工具的保存闭环）
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s") {
      if (editorMode === "edit" && !settingsOpen) {
        e.preventDefault();
        void doSave();
      }
      return;
    }

    const tag = (e.target as HTMLElement)?.tagName;
    const inEditor = tag === "TEXTAREA" || tag === "INPUT" || tag === "SELECT";
    // 搜索框聚焦时放行 ↑↓ / Enter 做列表导航（单行 input 不需要这些键编辑文本），
    // 实现「Ctrl+F 搜索 → ↑↓ 选中 → Enter 复制」全程不离开搜索框、不碰鼠标。
    // 其余编辑态（正文 textarea、重命名 input、select）照旧早退，不干扰编辑。
    const isSearchInput =
      (e.target as HTMLElement)?.id === "search-input";
    if (inEditor && !isSearchInput) return;

    if (e.key === "ArrowDown") {
      e.preventDefault();
      void navigateSelection(1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      void navigateSelection(-1);
    } else if (e.key === "Enter") {
      e.preventDefault();
      // 复制目标恒为 selectedPrompt，copy_mode 同样取自它——内容与模式永远
      // 同源。无选中且列表非空的态已被 reconcileSelection/过滤调和双保险
      // 消除（编辑态选中被过滤出列表时复制正在编辑的那条，语义一致）
      if (selectedPrompt) {
        const stored =
          selectedPrompt.meta.copy_mode === "plain" ? "plain" : "markdown";
        // Shift+Enter = 临时用另一复制模式（markdown↔plain），不改存储的 copy_mode
        const mode = e.shiftKey
          ? stored === "markdown"
            ? "plain"
            : "markdown"
          : stored;
        void doCopy(mode);
      }
    }
  }

  function scrollIntoView() {
    queueMicrotask(() => {
      if (selectedIndex >= 0) scrollToIndexFn?.(selectedIndex);
    });
  }

  // 无边框窗口自定义 resize：8 个边缘热区，mousedown 触发系统缩放手柄。
  // Tauri v2 的 ResizeDirection 使用方位词，不是 CSS 的 top/left 命名。
  const RESIZE_EDGES = [
    "top",
    "right",
    "bottom",
    "left",
    "top-left",
    "top-right",
    "bottom-left",
    "bottom-right",
  ] as const;
  type ResizeDirection =
    | "East"
    | "North"
    | "NorthEast"
    | "NorthWest"
    | "South"
    | "SouthEast"
    | "SouthWest"
    | "West";
  const EDGE_TO_DIRECTION: Record<(typeof RESIZE_EDGES)[number], ResizeDirection> = {
    top: "North",
    right: "East",
    bottom: "South",
    left: "West",
    "top-left": "NorthWest",
    "top-right": "NorthEast",
    "bottom-left": "SouthWest",
    "bottom-right": "SouthEast",
  };
  function startResize(edge: (typeof RESIZE_EDGES)[number]) {
    void getCurrentWindow().startResizeDragging(EDGE_TO_DIRECTION[edge]);
  }

  onMount(() => {
    language = getStoredLanguage(getLanguageStorage());
    void bootstrap();
  });
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="app" role="application" aria-label="Prompt Pocket">
  <!-- 无边框窗口的 resize 热区：8 个透明按钮贴在窗口边缘 -->
  {#each RESIZE_EDGES as edge}
    <button
      type="button"
      class="resize-edge"
      data-edge={edge}
      aria-label={t("app.resizeWindow", { edge })}
      tabindex="-1"
      onmousedown={(e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        e.stopPropagation();
        startResize(edge);
      }}
    ></button>
  {/each}
  {#if loading}
    <div class="state">
      <div class="spinner"></div>
      <span>{t("app.loading")}</span>
    </div>
  {:else}
    <header class="topbar" data-tauri-drag-region>
      <div class="search-wrap">
        <span class="icon">⌕</span>
        <input
          id="search-input"
          type="text"
          placeholder={t("app.searchPlaceholder")}
          bind:value={query}
          autocomplete="off"
          spellcheck="false"
        />
        <button class="new-btn" onclick={doCreate} title={t("app.newPrompt")}>+</button>
        {#if syncStatus?.configured}
          <button
            type="button"
            class="sync-indicator"
            class:syncing={syncStatus.syncing}
            class:error={!!syncStatus.lastError}
            title={syncStatus.lastError || syncStatus.lastSync || t("app.syncConnected")}
            aria-label={t("app.syncStatusAria")}
            onclick={() => (settingsOpen = true)}
          ><span class="sync-dot" aria-hidden="true"></span></button>
        {/if}
        <button
          class="new-btn lang-btn"
          onclick={toggleLanguage}
          title={t("app.switchLanguageTitle")}
          aria-label={t("app.switchLanguageAria")}
        >
          {language === "zh" ? "EN" : "中"}
        </button>
        <button
          class="new-btn"
          onclick={() => (settingsOpen = true)}
          title={t("app.settings")}
          aria-label={t("app.settings")}
        >
          ⚙
        </button>
      </div>
    </header>

    <nav class="tabs">
      <CategoryTabs
        {categories}
        total={allPrompts.length}
        bind:selected={selectedCategory}
        oncreate={onCreateCategory}
        onrename={onRenameCategory}
        oncontextmenu={onCatContextMenu}
        onreorder={doReorderCategory}
        ondragstart={onDragGestureStart}
        ondragend={onDragGestureEnd}
        {t}
      />
    </nav>

    <main class="body">
      <!-- 副行内容只看视图：「全部」显示分类名（跨分类有辨识度），
           单分类显示相对时间（该视图下分类名永远冗余，与是否搜索无关） -->
      <PromptList
        prompts={visiblePrompts}
        {selectedPath}
        {selectedIndex}
        {query}
        draggable={canReorderPrompts}
        disabledReason={reorderDisabledLabel}
        subMode={selectedCategory === "__all__" ? "category" : "time"}
        {language}
        onmounted={(fn) => (scrollToIndexFn = fn)}
        onselect={async (path) => {
          if (path === selectedPath) return;
          // 编辑中有未保存修改：确认后再切换（防静默丢失）
          if (!(await confirmDiscardIfDirty())) return;
          selectedPath = path;
        }}
        oncontextmenu={openContextMenu}
        onreorder={doReorder}
        ondragstart={onDragGestureStart}
        ondragend={onDragGestureEnd}
        {t}
      />

      <Editor
        prompt={selectedPrompt}
        mode={editorMode}
        bind:body={editingBody}
        bind:title={editingTitle}
        bind:category={editingCategory}
        bind:copyMode={editingCopyMode}
        {categories}
        {t}
        oncopy={(m) => doCopy(m, false)}
        onsave={doSave}
        oncancel={cancelEdit}
        onedit={() => {
          // 进入编辑前，把当前 prompt 的分类同步到编辑字段
          if (selectedPrompt) editingCategory = selectedPrompt.category;
          editorMode = "edit";
        }}
        onreveal={() => selectedPrompt && void revealInFinder(selectedPrompt.path)}
        ondelete={doDelete}
        oncreatecategory={onCreateCategory}
      />
    </main>

    {#if copiedFlash}
      <div class="toast" transition:fly={{ y: 20 }}>
        {t("app.copiedToast")}
      </div>
    {/if}

    {#if error}
      <div class="toast error-toast" transition:fly={{ y: 20 }}>
        <span class="error-text">{error}</span>
        <button class="error-close" onclick={() => (error = null)}>×</button>
      </div>
    {/if}

    <Settings
      bind:open={settingsOpen}
      onsynced={onSynced}
      {language}
      {t}
      onlanguagechange={changeLanguage}
    />

    <ContextMenu
      bind:open={contextMenu.open}
      prompt={contextMenu.prompt}
      x={contextMenu.x}
      y={contextMenu.y}
      {categories}
      {t}
      onrename={onCtxRename}
      onmove={onCtxMove}
      ondelete={onCtxDelete}
      onclose={() => (contextMenu.prompt = null)}
    />

    {#if variableDialog.open}
      <VariableDialog
        variables={variableDialog.variables}
        {t}
        onconfirm={onVarsConfirm}
        oncopyraw={onVarsCopyRaw}
        oncancel={closeVarDialog}
      />
    {/if}

    {#if renameDialog.open}
      <div
        class="backdrop"
        transition:fly={{ duration: 100 }}
        onclick={(e) => {
          if (e.target === e.currentTarget) renameDialog.open = false;
        }}
        onkeydown={(e) => {
          if (e.key === "Escape") {
            // 阻止冒泡到 <svelte:window>：否则关弹窗的同时会把整个窗口也隐藏
            e.stopPropagation();
            e.preventDefault();
            renameDialog.open = false;
          }
        }}
        role="presentation"
      >
        <div class="dialog" transition:fly={{ y: -10, duration: 120 }}>
          <h3>{t("app.renameMoveTitle")}</h3>
          <div class="dialog-row">
            <label for="rn-title">{t("app.titleLabel")}</label>
            <input id="rn-title" type="text" bind:value={renameDialog.title} use:autofocus />
          </div>
          <div class="dialog-row">
            <label for="rn-cat">{t("app.categoryLabel")}</label>
            <select id="rn-cat" bind:value={renameDialog.category}>
              <option value={"未分类"}>{t("common.uncategorized")}</option>
              {#each categories as c}
                <option value={c.name}>{c.name}</option>
              {/each}
            </select>
          </div>
          <div class="dialog-actions">
            <button class="ghost" onclick={() => (renameDialog.open = false)}>
              {t("common.cancel")}
            </button>
            <button class="primary" onclick={submitRename}>{t("common.confirm")}</button>
          </div>
        </div>
      </div>
    {/if}

    {#if catContextMenu.open}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="backdrop"
        onclick={() => (catContextMenu.open = false)}
        oncontextmenu={(e) => {
          e.preventDefault();
          catContextMenu.open = false;
        }}
        transition:fly={{ duration: 80 }}
      ></div>
      <div
        class="cat-menu"
        style="left: {catContextMenu.x}px; top: {catContextMenu.y}px;"
        transition:fly={{ y: -4, duration: 100 }}
      >
        <button
          class="cat-menu-item"
          onclick={() => {
            onRenameCategory(catContextMenu.name);
            catContextMenu.open = false;
          }}
        >
          <span class="ico">✎</span> {t("app.renameCategoryAction")}
        </button>
      </div>
    {/if}

    {#if catRenameDialog.open}
      <div
        class="backdrop"
        transition:fly={{ duration: 100 }}
        onclick={(e) => {
          if (e.target === e.currentTarget) catRenameDialog.open = false;
        }}
        onkeydown={(e) => {
          if (e.key === "Escape") {
            e.stopPropagation();
            e.preventDefault();
            catRenameDialog.open = false;
          }
        }}
        role="presentation"
      >
        <div class="dialog" transition:fly={{ y: -10, duration: 120 }}>
          <h3>{t("app.categoryRenameTitle")}</h3>
          <div class="dialog-row">
            <label for="cat-rn">{t("app.newCategoryName")}</label>
            <input
              id="cat-rn"
              type="text"
              bind:value={catRenameDialog.newName}
              use:autofocus
              onkeydown={(e) => e.key === "Enter" && submitCatRename()}
            />
          </div>
          <div class="dialog-actions">
            <button class="ghost" onclick={() => (catRenameDialog.open = false)}>
              {t("common.cancel")}
            </button>
            <button class="primary" onclick={submitCatRename}>{t("common.confirm")}</button>
          </div>
        </div>
      </div>
    {/if}
  {/if}
</div>

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
    overflow: hidden;
    background: var(--bg);
    color: var(--fg);
    font-size: 14px;
  }

  .topbar {
    flex-shrink: 0;
    padding: 14px 18px 10px;
    border-bottom: 1px solid transparent;
    background: var(--bg);
  }
  .search-wrap {
    display: flex;
    align-items: center;
    gap: 8px;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 0 8px 0 12px;
    height: 42px;
    box-shadow: 0 1px 2px rgba(31, 42, 68, 0.04);
    transition:
      border-color 0.12s,
      box-shadow 0.12s;
  }
  .search-wrap:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .icon {
    color: var(--muted);
    font-size: 17px;
    line-height: 1;
  }
  #search-input {
    flex: 1;
    background: transparent;
    border: none;
    outline: none;
    color: var(--fg);
    font-size: 14px;
    height: 100%;
  }
  #search-input::placeholder {
    color: var(--muted);
  }
  .new-btn {
    width: 30px;
    height: 30px;
    border-radius: 8px;
    border: 1px solid transparent;
    background: var(--accent-soft);
    color: var(--accent);
    font-size: 18px;
    line-height: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition:
      background 0.12s,
      border-color 0.12s,
      color 0.12s;
  }
  .new-btn:hover {
    background: var(--accent);
    border-color: var(--accent);
    color: #ffffff;
  }
  .lang-btn {
    width: 36px;
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0;
  }

  .sync-indicator {
    width: 24px;
    height: 24px;
    padding: 0;
    border: none;
    background: transparent;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 50%;
    cursor: pointer;
    flex-shrink: 0;
  }
  .sync-indicator .sync-dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--success);
    box-shadow: 0 0 0 3px var(--success-soft);
    pointer-events: none;
    transition: transform 0.12s;
  }
  .sync-indicator:hover .sync-dot {
    transform: scale(1.2);
  }
  .sync-indicator.syncing .sync-dot {
    background: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
    animation: sync-pulse 1s infinite;
  }
  .sync-indicator.error .sync-dot {
    background: var(--danger);
    /* 出错态脉冲提醒：静默小红点几乎不可能被注意到 */
    animation: sync-pulse 1s infinite;
  }
  @keyframes sync-pulse {
    50% {
      opacity: 0.4;
    }
  }

  .body {
    flex: 1;
    display: grid;
    grid-template-columns: 312px 1fr;
    grid-template-rows: minmax(0, 1fr);
    min-height: 0;
    overflow: hidden;
    background: var(--bg);
  }
  .body :global(.list),
  .body :global(.editor) {
    min-width: 0;
    min-height: 0;
  }

  .tabs {
    flex-shrink: 0;
    height: 44px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-elevated);
    padding: 0 18px;
  }

  .state {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    color: var(--muted);
  }
  .spinner {
    width: 28px;
    height: 28px;
    border: 2px solid var(--border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .toast {
    position: fixed;
    bottom: 24px;
    left: 50%;
    transform: translateX(-50%);
    background: var(--bg-elevated);
    color: var(--fg);
    padding: 10px 16px;
    border: 1px solid var(--border);
    border-left: 3px solid var(--accent);
    border-radius: 10px;
    box-shadow: var(--shadow-soft);
    font-size: 13px;
    z-index: 100;
  }
  .error-toast {
    background: var(--danger);
    color: #fff;
    max-width: 80vw;
    display: flex;
    align-items: center;
    gap: 10px;
    bottom: 64px;
  }
  .error-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .error-close {
    background: transparent;
    border: none;
    color: #fff;
    font-size: 18px;
    line-height: 1;
    cursor: pointer;
    padding: 0 2px;
    opacity: 0.8;
  }
  .error-close:hover {
    opacity: 1;
  }

  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(31, 42, 68, 0.24);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 200;
    backdrop-filter: blur(2px);
  }
  .dialog {
    width: 380px;
    max-width: 90vw;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 12px;
    box-shadow: var(--shadow-soft);
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 14px;
  }
  .dialog h3 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  .dialog-row {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .dialog-row label {
    font-size: 11px;
    font-weight: 600;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .dialog-row input,
  .dialog-row select {
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    color: var(--fg);
    border-radius: 8px;
    padding: 7px 10px;
    font-size: 13px;
    outline: none;
  }
  .dialog-row input:focus,
  .dialog-row select:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }

  .cat-menu {
    position: fixed;
    z-index: 160;
    min-width: 140px;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: var(--shadow-soft);
    padding: 4px;
  }
  .cat-menu-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    background: transparent;
    border: none;
    color: var(--fg);
    font-size: 13px;
    padding: 7px 10px;
    border-radius: 7px;
    cursor: pointer;
    text-align: left;
    font-family: inherit;
  }
  .cat-menu-item:hover {
    background: var(--bg-hover);
    color: var(--accent);
  }
  .cat-menu-item .ico {
    width: 16px;
    text-align: center;
    opacity: 0.8;
  }

  /* 无边框窗口的 resize 热区：8 个透明条/角，z-index 置顶不挡视觉 */
  .resize-edge {
    position: fixed;
    z-index: 9999;
    padding: 0;
    border: 0;
    appearance: none;
    background: transparent;
  }
  .resize-edge[data-edge="top"] {
    top: 0;
    left: 8px;
    right: 8px;
    height: 6px;
    cursor: ns-resize;
  }
  .resize-edge[data-edge="bottom"] {
    bottom: 0;
    left: 8px;
    right: 8px;
    height: 6px;
    cursor: ns-resize;
  }
  .resize-edge[data-edge="left"] {
    top: 8px;
    bottom: 8px;
    left: 0;
    width: 6px;
    cursor: ew-resize;
  }
  .resize-edge[data-edge="right"] {
    top: 8px;
    bottom: 8px;
    right: 0;
    width: 6px;
    cursor: ew-resize;
  }
  .resize-edge[data-edge="top-left"] {
    top: 0;
    left: 0;
    width: 14px;
    height: 14px;
    cursor: nwse-resize;
  }
  .resize-edge[data-edge="top-right"] {
    top: 0;
    right: 0;
    width: 14px;
    height: 14px;
    cursor: nesw-resize;
  }
  .resize-edge[data-edge="bottom-left"] {
    bottom: 0;
    left: 0;
    width: 14px;
    height: 14px;
    cursor: nesw-resize;
  }
  .resize-edge[data-edge="bottom-right"] {
    bottom: 0;
    right: 0;
    width: 14px;
    height: 14px;
    cursor: nwse-resize;
  }
</style>
