<script lang="ts">
  import { onMount, tick, untrack } from "svelte";
  import { ask } from "@tauri-apps/plugin-dialog";
  import { listen } from "@tauri-apps/api/event";
  import { fly } from "svelte/transition";
  import { getCurrentWindow } from "@tauri-apps/api/window";
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
  import { createRequestGate, draftChanged, type Draft } from "./lib/editor-session";
  import { readLibrary, writeLibrary, toggleFavorite, recordUse, moveLibraryEntry, type Library } from "./lib/library";
  import { templateFields, renderTemplate } from "./lib/templates";
  import { markdownToPlain } from "./lib/plain-text";
  import { dialogFocus } from "./lib/dialog";
  import TemplateDialog from "./lib/TemplateDialog.svelte";
  import { setWindowMode, setInteractionLock } from "./lib/api";
  import { filterPrompts } from "./lib/search";
  import CategoryTabs from "./lib/CategoryTabs.svelte";
  import PromptList from "./lib/PromptList.svelte";
  import Editor from "./lib/Editor.svelte";
  import Settings from "./lib/Settings.svelte";
  import ContextMenu from "./lib/ContextMenu.svelte";
  import {
    canReorderPromptList,
    getReorderCategory,
    getReorderDisabledReason,
    moveCategoryOrder,
    movePathOrder,
  } from "./lib/reorder";
  import {
    createTranslator,
    mapBackendMessage,
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

  let draftNew = $state(false);
  let library = $state<Library>({});
  let scope = $state<"all" | "favorites" | "recent">("all");
  let windowMode = $state<"quick" | "manage">("quick");
  let loadedPath = $state<string | null>(null);
  let contentLoading = $state(false);
  let saving = $state(false);
  let copying = $state(false);
  let baseline = $state<Draft>({ title: "", body: "", category: "", copy_mode: "markdown" });
  let currentDraft = $derived({ title: editingTitle, body: editingBody, category: editingCategory, copy_mode: editingCopyMode });
  let dirty = $derived(editorMode === "edit" && draftChanged(currentDraft, baseline));
  let ready = $derived((draftNew || !!selectedPath && loadedPath === selectedPath) && !contentLoading && !saving);
  let templateSession = $state<{ body: string; title: string; path: string; mode: "markdown" | "plain"; hideAfter: boolean } | null>(null);
  const requestGate = createRequestGate();
  let loading = $state(true);
  let error = $state<string | null>(null);
  let language = $state<Language>("zh");
  let t = $derived(createTranslator(language));

  // 统一错误提示：显示后 5 秒自动消失，不阻塞 UI
  function showError(msg: string) {
    msg = mapBackendMessage(msg, t);
    error = msg;
    setTimeout(() => {
      if (error === msg) error = null;
    }, 5000);
  }
  let copiedFlash = $state(false);
  let copyMessage = $state("");
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

  let scopedPrompts = $derived(scope === "favorites" ? categoryFiltered.filter((p) => library[p.path]?.favorite) :
    scope === "recent" ? categoryFiltered.filter((p) => library[p.path]?.lastUsed).sort((a, b) => library[b.path].lastUsed - library[a.path].lastUsed) : categoryFiltered);
  let visiblePrompts = $derived(filterPrompts(scopedPrompts, query, library));
  let canReorderPrompts = $derived(
    editorMode !== "edit" && !saving && scope === "all" && windowMode === "manage" && canReorderPromptList(query, selectedCategory, visiblePrompts),
  );
  let reorderDisabledReason = $derived(
    getReorderDisabledReason(query, selectedCategory, visiblePrompts),
  );
  let reorderDisabledLabel = $derived(
    translateReorderDisabledReason(reorderDisabledReason, language),
  );

  let selectedIndex = $derived(selectedPath ? visiblePrompts.findIndex((p) => p.path === selectedPath) : -1);
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
      void tick().then(focusSearch);
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

  function translateReorderDisabledReason(reason: string | null, lang: Language): string {
    const translateFor = createTranslator(lang);
    switch (reason) {
      case "needTwo":
        return translateFor("reorder.needTwoPrompts");
      case "singleCategory":
        return translateFor("reorder.singleCategory");
      case "searchDisabled":
        return translateFor("reorder.searchDisabled");
      default:
        return reason ?? "";
    }
  }

  async function refresh() {
    const res = await scanPrompts();
    allPrompts = res.prompts;
    categories = res.categories;
  }

  // 设置界面切换数据目录后：更新配置、重置选中、重新扫描
  // 同步完成后：重新加载列表 + 刷新同步状态
  async function onSynced() {
    await refresh();
    if (editorMode !== "edit" && selectedPath) void loadPromptContent(selectedPath);
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
  let dragGestureActive = false;
  function onDragGestureStart() { dragGestureActive = true; }
  function onDragGestureEnd() {
    dragGestureActive = false;
    if (pendingRefresh && !reorderInFlight) {
      pendingRefresh = false;
      void guardedRefresh().catch((e) => showError(String(e)));
    }
  }
  async function guardedRefresh() {
    if (reorderInFlight || dragGestureActive) {
      // 重排写盘中：标记需要补刷，等 doReorder 完成后自己刷
      pendingRefresh = true;
      return;
    }
    await refresh();
    if (editorMode !== "edit" && selectedPath) void loadPromptContent(selectedPath);
  }

  // Selection loads have identity tokens; stale responses never replace another prompt.
  let lastLoadedPath: string | null = null;
  $effect(() => {
    const path = selectedPath;
    untrack(() => {
      if (path && path !== lastLoadedPath) { lastLoadedPath = path; void loadPromptContent(path); }
      if (!path) { requestGate.invalidate(); lastLoadedPath = null; loadedPath = null; contentLoading = false; }
    });
  });
  $effect(() => {
    const locked = windowMode === "manage" || editorMode === "edit" || settingsOpen || !!templateSession || dirty || renameDialog.open || catRenameDialog.open || contextMenu.open || catContextMenu.open;
    void setInteractionLock(locked).catch(() => {});
  });
  async function loadPromptContent(path: string) {
    const token = requestGate.begin(path);
    contentLoading = true;
    loadedPath = null;
    try {
      const { meta, body } = await readPrompt(path);
      if (!requestGate.accepts(token, path) || selectedPath !== path) return;
      applyMetaToEditFields(meta);
      editingBody = body;
      baseline = { ...currentDraft };
      loadedPath = path;
      editorMode = "view";
    } catch (e) {
      if (!requestGate.accepts(token, path) || selectedPath !== path) return;
      lastLoadedPath = null;
      if (String(e).includes("FILE_NOT_FOUND")) removePromptFromList(path);
      else showError(String(e));
    } finally {
      if (requestGate.accepts(token, path)) contentLoading = false;
    }
  }
  async function permitLeave(): Promise<boolean> {
    if (saving || copying) return false;
    if (dirty && !(await ask(t("editor.leaveConfirm"), { title: t("app.unsavedTitle"), kind: "warning" }))) return false;
    if (editorMode === "edit") {
      editingTitle = baseline.title;
      editingBody = baseline.body;
      editingCategory = baseline.category;
      editingCopyMode = baseline.copy_mode === "plain" ? "plain" : "markdown";
    }
    draftNew = false;
    editorMode = "view";
    return true;
  }
  async function selectPath(path: string) {
    if (path === selectedPath) { if (!contentLoading && loadedPath !== path) void loadPromptContent(path); return; }
    if (!(await permitLeave())) return;
    selectedPath = path;
  }
  function persistLibrary(next: Library) {
    library = next;
    writeLibrary(getLanguageStorage(), library);
  }
  function favorite(path: string) { if (editorMode === "edit" || saving) return; persistLibrary(toggleFavorite(library, path)); }
  async function changeWindowMode(mode: "quick" | "manage") {
    if (mode === "quick" && !(await permitLeave())) return;
    windowMode = mode;
    try { await setWindowMode(mode); } catch (e) { showError(String(e)); }
  }
  function startEditing() { if (ready) { void changeWindowMode("manage"); baseline = { ...currentDraft }; editorMode = "edit"; } }
  async function cancelEdit() { if (await permitLeave()) { if (selectedPath) void loadPromptContent(selectedPath); else await refresh(); } }

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
      } else {
        selectedPath = null;
        lastLoadedPath = null;
      }
    }
    // 刷新分类计数
    void refresh();
  }

  async function completeCopy(text: string, mode: "markdown" | "plain", path: string, hideAfter = true) {
    if (copying) return;
    copying = true;
    try {
      const result = await copyOrPaste(mode === "plain" ? markdownToPlain(text) : text, mode, hideAfter);
      persistLibrary(recordUse(library, path));
      copyMessage = t(result.status === "pasted" ? "app.pastedToast" : result.status === "paste_failed" ? "app.pasteFailedToast" : "app.copiedToast");
      copiedFlash = true;
      setTimeout(() => (copiedFlash = false), 2200);
    } finally { copying = false; }
  }
  async function doCopy(mode: "markdown" | "plain", hideAfter = true) {
    if (!selectedPrompt || !ready || copying || editorMode === "edit") return;
    const path = selectedPrompt.path;
    if (templateFields(editingBody).length) {
      templateSession = { body: editingBody, title: selectedPrompt.title, path, mode, hideAfter };
      return;
    }
    try { await completeCopy(renderTemplate(editingBody, {}), mode, path, hideAfter); } catch (e) { showError(String(e)); }
  }
  async function doSave() {
    if ((!selectedPrompt && !draftNew) || !ready || saving) return;
    const path = selectedPrompt?.path ?? "";
    const draft = { ...currentDraft };
    saving = true;
    try {
      let savePath = path;
      if (draftNew) savePath = (await createPrompt(draft.category, draft.title.trim())).path;
      const saved = await savePrompt(savePath, { ...draft, title: draft.title.trim() || t("app.untitled") });
      draftNew = false;
      requestGate.invalidate();
      persistLibrary(moveLibraryEntry(library, path, saved.path));
      selectedPath = saved.path;
      lastLoadedPath = saved.path;
      loadedPath = saved.path;
      baseline = { ...draft, title: saved.title, category: saved.category };
      editingCategory = saved.category;
      editingTitle = saved.title;
      editorMode = "view";
      selectedCategory = "__all__";
      query = "";
      scope = "all";
      await refresh();
    } catch (e) { showError(String(e)); }
    finally { saving = false; }
  }
  async function duplicatePrompt() {
    if (!selectedPrompt || !ready || !(await permitLeave())) return;
    const source = selectedPrompt;
    const content = editingBody;
    try {
      const created = await createPrompt(source.category, source.title + (language === "zh" ? " 副本" : " copy"));
      const saved = await savePrompt(created.path, { title: created.title, category: source.category, body: content, copy_mode: editingCopyMode });
      query = ""; scope = "all"; selectedCategory = "__all__";
      await refresh();
      await selectPath(saved.path);
      void changeWindowMode("manage");
    } catch (e) { showError(String(e)); }
  }
  async function insertSnippet(path: string) {
    const source = allPrompts.find((prompt) => prompt.path === path);
    if (source) editingBody += (editingBody.trim() ? "\n\n" : "") + (source.body ?? "");
  }

  async function doCreate() {
    if (!(await permitLeave())) return;
    requestGate.invalidate();
    await changeWindowMode("manage");
    const cat = selectedCategory === "__all__" ? "未分类" : selectedCategory;
    selectedPath = null;
    lastLoadedPath = null;
    loadedPath = null;
    contentLoading = false;
    query = "";
    scope = "all";
    editingTitle = "";
    editingCategory = cat;
    editingCopyMode = "markdown";
    editingBody = "";
    baseline = { ...currentDraft };
    draftNew = true;
    editorMode = "edit";
  }

  async function doDelete() {
    const prompt = selectedPrompt;
    if (!prompt || !(await permitLeave())) return;
    if (!(await ask(t("app.deleteConfirm", { title: prompt.title }), { title: t("editor.delete"), kind: "warning" }))) return;
    try {
      await deletePrompt(prompt.path);
      selectedPath = null;
      lastLoadedPath = null;
      await refresh();
    } catch (e) {
      showError(String(e));
    }
  }

  // 问题2：右键菜单操作
  function openContextMenu(prompt: Prompt, x: number, y: number) {
    contextMenu = { open: true, x, y, prompt };
  }

  async function onCtxRename() {
    const prompt = contextMenu.prompt;
    if (!prompt || !(await permitLeave())) return;
    renameDialog = {
      open: true,
      path: prompt.path,
      title: prompt.title,
      category: prompt.category,
    };
  }

  async function onCtxMove(category: string) {
    const prompt = contextMenu.prompt;
    if (!prompt || !(await permitLeave())) return;
    const oldPath = prompt.path;
    try {
      const moved = await renamePrompt(
        prompt.path,
        prompt.title,
        category,
      );
      persistLibrary(moveLibraryEntry(library, oldPath, moved.path));
      if (selectedPath === oldPath) { selectedPath = moved.path; lastLoadedPath = null; }
      await refresh();
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
    const p = contextMenu.prompt;
    if (!p || !(await permitLeave())) return;
    if (!(await ask(t("app.deleteConfirm", { title: p.title }), { title: t("editor.delete"), kind: "warning" }))) return;
    deletePrompt(p.path)
      .then(() => {
        if (selectedPath === p.path) {
          selectedPath = null;
          lastLoadedPath = null;
        }
        return refresh();
      })
      .catch((e) => showError(String(e)));
  }

  // 重命名对话框提交
  async function submitRename() {
    const renamingSelected = renameDialog.path === selectedPath;
    if (renamingSelected && !(await permitLeave())) return;
    try {
      const newPrompt = await renamePrompt(
        renameDialog.path,
        renameDialog.title.trim() || t("app.untitled"),
        renameDialog.category,
      );
      persistLibrary(moveLibraryEntry(library, renameDialog.path, newPrompt.path));
      await refresh();
      if (renamingSelected) { selectedPath = newPrompt.path; lastLoadedPath = null; loadedPath = null; }
      renameDialog.open = false;
    } catch (e) {
      showError(String(e));
    }
  }

  // 问题3：新建分类
  async function onCreateCategory(name: string) {
    try {
      await createCategory(name);
      await refresh();
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
    if (!(await permitLeave())) return;
    catRenameDialog = { open: true, oldName, newName: oldName };
  }

  async function submitCatRename() {
    if (!(await permitLeave())) return;
    const newName = catRenameDialog.newName.trim();
    if (!newName || newName === catRenameDialog.oldName) {
      catRenameDialog.open = false;
      return;
    }
    try {
      const prefix = catRenameDialog.oldName + "/";
      await renameCategory(catRenameDialog.oldName, newName);
      let nextLibrary = library;
      for (const p of allPrompts) if (p.path.startsWith(prefix)) nextLibrary = moveLibraryEntry(nextLibrary, p.path, newName + p.path.slice(prefix.length - 1));
      persistLibrary(nextLibrary);
      if (selectedPath?.startsWith(prefix)) { selectedPath = newName + selectedPath.slice(prefix.length - 1); lastLoadedPath = null; }
      if (selectedCategory === catRenameDialog.oldName) {
        selectedCategory = newName;
      }
      await refresh();
      catRenameDialog.open = false;
    } catch (e) {
      showError(String(e));
    }
  }

  // Filtering chooses a visible result; edits and saves preserve the active draft.
  let lastFilterKey: string | null = null;
  $effect(() => {
    const key = `${query}\u0000${selectedCategory}\u0000${scope}`;
    untrack(() => {
      if (key === lastFilterKey) return;
      lastFilterKey = key;
      if (editorMode !== "edit" && !saving) selectedPath = visiblePrompts[0]?.path ?? null;
    });
  });
  $effect(() => {
    const prompts = visiblePrompts;
    untrack(() => {
      if (editorMode === "edit" || saving || draftNew) return;
      if (!selectedPath || !allPrompts.some((p) => p.path === selectedPath)) selectedPath = prompts[0]?.path ?? null;
    });
  });
  $effect(() => {
    const index = selectedIndex;
    void tick().then(() => scrollToIndexFn?.(index));
  });
  function handleKeydown(e: KeyboardEvent) {
    if (e.defaultPrevented || e.isComposing || e.keyCode === 229) return;
    if (templateSession) return;
    if (e.key === "Escape") {
      e.preventDefault();
      if (dragGestureActive) onDragGestureEnd();
      if (contextMenu.open) { contextMenu.open = false; return; }
      if (catContextMenu.open) { catContextMenu.open = false; return; }
      if (renameDialog.open) { renameDialog.open = false; return; }
      if (catRenameDialog.open) { catRenameDialog.open = false; return; }
      if (settingsOpen) { settingsOpen = false; return; }
      if (editorMode === "edit") { cancelEdit(); return; }
      if (query.trim()) { query = ""; return; }
      void hideWindow();
      return;
    }
    if (settingsOpen || renameDialog.open || catRenameDialog.open || contextMenu.open || catContextMenu.open) return;
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "s" && editorMode === "edit") { e.preventDefault(); void doSave(); return; }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "n") { e.preventDefault(); void doCreate(); return; }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "f") { e.preventDefault(); document.querySelector<HTMLInputElement>("#search-input")?.focus(); return; }
    const target = e.target as HTMLElement;
    const isSearchInput = target?.id === "search-input";
    if (["TEXTAREA", "INPUT", "SELECT", "BUTTON"].includes(target?.tagName) && !isSearchInput) return;
    if (editorMode === "edit") return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = Math.max(0, Math.min(selectedIndex + (e.key === "ArrowDown" ? 1 : -1), visiblePrompts.length - 1));
      if (visiblePrompts[next]) void selectPath(visiblePrompts[next].path);
    } else if (e.key === "Enter") { e.preventDefault(); void doCopy(e.shiftKey ? (editingCopyMode === "plain" ? "markdown" : "plain") : editingCopyMode); }
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

  function focusSearch() {
    if (editorMode === "edit" && !settingsOpen && !templateSession && !renameDialog.open && !catRenameDialog.open) { document.querySelector<HTMLTextAreaElement>("#f-body")?.focus(); return; }
    if (editorMode !== "edit" && !settingsOpen && !templateSession && !renameDialog.open && !catRenameDialog.open) {
      const input = document.querySelector<HTMLInputElement>("#search-input");
      input?.focus();
      input?.select();
    }
  }

  onMount(() => {
    language = getStoredLanguage(getLanguageStorage());
    library = readLibrary(getLanguageStorage());
    void bootstrap();
    let disposed = false;
    const cleanups: (() => void)[] = [];
    void listen("sync-finished", () => { void guardedRefresh().catch((e) => showError(String(e))); void getSyncStatus().then((status) => (syncStatus = status)).catch(() => {}); }).then((stop) => { if (disposed) stop(); else cleanups.push(stop); }).catch(() => {});
    void listen("window-shown", focusSearch).then((stop) => { if (disposed) stop(); else cleanups.push(stop); }).catch(() => {});
    return () => { disposed = true; cleanups.forEach((stop) => stop()); requestGate.invalidate(); };
  });
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="app" class:quick={windowMode === "quick"} role="application" aria-label="Prompt Pocket">
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
          disabled={editorMode === "edit" || saving}
          autocomplete="off"
          spellcheck="false"
        />
        <button class="new-btn" onclick={doCreate} title={t("app.newPrompt")}>+</button>
        {#if syncStatus?.configured}
          <span
            class="sync-indicator"
            class:syncing={syncStatus.syncing}
            class:error={!!syncStatus.lastError}
            title={syncStatus.lastError || (syncStatus.lastSync ? mapBackendMessage(syncStatus.lastSync, t) : "") || t("app.syncConnected")}
          ></span>
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

    <div class="library-bar">
      <div class="library-scopes" role="group" aria-label={t("view.all")}>
        {#each ["all", "favorites", "recent"] as tab}
          <button class:active={scope === tab} disabled={editorMode === "edit" || saving} onclick={() => { scope = tab as typeof scope; }}>
            {t(tab === "all" ? "view.all" : tab === "favorites" ? "view.favorites" : "view.recent")}
          </button>
        {/each}
      </div>
      <button class="mode-toggle" onclick={() => void changeWindowMode(windowMode === "quick" ? "manage" : "quick")}>{t(windowMode === "quick" ? "view.manage" : "view.quick")} ↗</button>
    </div>
    <nav class="tabs" class:locked={editorMode === "edit" || saving} inert={editorMode === "edit" || saving}>
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
      <PromptList
        prompts={visiblePrompts}
        {selectedPath}
        {selectedIndex}
        {query}
        {library}
        compact={windowMode === "quick"}
        preferencesDisabled={editorMode === "edit" || saving}
        onfavorite={favorite}
        draggable={canReorderPrompts}
        disabledReason={reorderDisabledLabel}
        onmounted={(fn) => (scrollToIndexFn = fn)}
        onselect={selectPath}
        oncontextmenu={openContextMenu}
        onreorder={doReorder}
        ondragstart={onDragGestureStart}
        ondragend={onDragGestureEnd}
        subMode={selectedCategory === "__all__" ? "category" : "time"}
        {language}
        {t}
      />

      {#if windowMode === "manage"}
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
        busy={!ready || copying}
        {saving}
        {dirty}
        snippets={allPrompts.filter((p) => p.path !== selectedPath)}
        onduplicate={duplicatePrompt}
        oninsertsnippet={insertSnippet}
        oncancel={cancelEdit}
        onedit={startEditing}
        onreveal={() => selectedPrompt && void revealInFinder(selectedPrompt.path)}
        ondelete={doDelete}
        oncreatecategory={onCreateCategory}
      />
      {/if}
    </main>
    {#if windowMode === "quick"}
      <footer class="quick-footer"><span>{t("view.keyboard")}</span><button class="ghost" disabled={!ready} onclick={startEditing}>{t("editor.edit")}</button><button class="primary" disabled={!ready || copying} onclick={() => doCopy(editingCopyMode)}>{contentLoading ? t("editor.loading") : templateFields(editingBody).length ? t("template.title") : t("editor.copyLabel")} ↵</button></footer>
    {/if}

    {#if copiedFlash}
      <div class="toast" role="status" transition:fly={{ y: 20 }}>
        {copyMessage}
      </div>
    {/if}

    {#if error}
      <div class="toast error-toast" role="alert" transition:fly={{ y: 20 }}>
        <span class="error-text">{error}</span>
        <button class="error-close" onclick={() => (error = null)}>×</button>
      </div>
    {/if}

    <Settings
      bind:open={settingsOpen}
      onsynced={onSynced}
      onrestored={async (path) => { await refresh(); await selectPath(path); }}
      {language}
      {t}
      onlanguagechange={changeLanguage}
    />

    {#if templateSession}
      <TemplateDialog body={templateSession.body} title={templateSession.title} {t} onclose={() => (templateSession = null)} onraw={async () => { const session = templateSession; if (!session) return; await completeCopy(session.body, session.mode, session.path, session.hideAfter); templateSession = null; }} onapply={async (text) => { const session = templateSession; if (!session) return; await completeCopy(text, session.mode, session.path, session.hideAfter); templateSession = null; }} />
    {/if}
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

    {#if renameDialog.open}
      <div
        class="backdrop"
        transition:fly={{ duration: 100 }}
        onclick={(e) => {
          if (e.target === e.currentTarget) renameDialog.open = false;
        }}
        onkeydown={(e) => { if (e.key === "Escape") { e.stopPropagation(); e.preventDefault(); renameDialog.open = false; } }}
        role="presentation"
      >
        <div class="dialog" role="dialog" aria-modal="true" aria-label={t("app.renameMoveTitle")} tabindex="-1" use:dialogFocus transition:fly={{ y: -10, duration: 120 }}>
          <h3>{t("app.renameMoveTitle")}</h3>
          <div class="dialog-row">
            <label for="rn-title">{t("app.titleLabel")}</label>
            <input id="rn-title" type="text" bind:value={renameDialog.title} />
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
        onkeydown={(e) => { if (e.key === "Escape") { e.stopPropagation(); e.preventDefault(); catRenameDialog.open = false; } }}
        role="presentation"
      >
        <div class="dialog" role="dialog" aria-modal="true" aria-label={t("app.categoryRenameTitle")} tabindex="-1" use:dialogFocus transition:fly={{ y: -10, duration: 120 }}>
          <h3>{t("app.categoryRenameTitle")}</h3>
          <div class="dialog-row">
            <label for="cat-rn">{t("app.newCategoryName")}</label>
            <input
              id="cat-rn"
              type="text"
              bind:value={catRenameDialog.newName}
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
  .library-bar { display: flex; align-items: center; justify-content: space-between; padding: 0 18px 8px; gap: 12px; }
  .library-scopes { display: flex; gap: 5px; } .library-scopes button, .mode-toggle { border: 0; border-radius: 6px; padding: 6px 10px; font-size: 12px; color: var(--muted); background: transparent; cursor: pointer; } .library-scopes button.active { color: var(--accent); background: var(--accent-soft); } .mode-toggle { color: var(--accent); }
  .quick .body { grid-template-columns: 1fr; } .quick-footer { display: flex; align-items: center; gap: 10px; padding: 12px 18px; border-top: 1px solid var(--border); background: var(--bg-elevated); } .quick-footer > span { flex: 1; color: var(--muted); font-size: 11px; } .locked { opacity: .65; }

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
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: #22a06b;
    flex-shrink: 0;
    cursor: help;
    box-shadow: 0 0 0 3px rgba(34, 160, 107, 0.12);
  }
  .sync-indicator.syncing {
    background: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
    animation: sync-pulse 1s infinite;
  }
  .sync-indicator.error {
    background: var(--danger);
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
