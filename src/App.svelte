<script lang="ts">
  import { onMount } from "svelte";
  import { fly } from "svelte/transition";
  import type { CategoryCount, Prompt, PromptMeta, SyncStatus } from "./lib/types";
  import {
    copyText,
    createCategory,
    createPrompt,
    deletePrompt,
    getSyncStatus,
    hideWindow,
    initApp,
    readPrompt,
    renamePrompt,
    renameCategory,
    reorderPrompts,
    revealInFinder,
    savePrompt,
    scanPrompts,
  } from "./lib/api";
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
    movePathOrder,
  } from "./lib/reorder";

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

  let loading = $state(true);
  let error = $state<string | null>(null);

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

  let selectedIndex = $state(0);
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
    try {
      syncStatus = await getSyncStatus();
    } catch {
      /* 忽略 */
    }
  }

  // 拖拽重排进行中标志：reorderPrompts 把新顺序写盘前，若 sync-finished
  // 抵达并触发 refresh()，会读到旧 .order.json 把刚拖的顺序冲掉。
  // 用该标志让写盘期间的 refresh 延迟到写盘完成后，避免竞态。
  let reorderInFlight = false;
  let pendingRefresh = false;
  async function guardedRefresh() {
    if (reorderInFlight) {
      // 重排写盘中：标记需要补刷，等 doReorder 完成后自己刷
      pendingRefresh = true;
      return;
    }
    await refresh();
  }

  // 监听后端 sync-finished 事件，自动刷新
  let unlisten: (() => void) | null = null;
  $effect(() => {
    import("@tauri-apps/api/event").then(({ listen }) => {
      listen("sync-finished", () => {
        void guardedRefresh();
        void getSyncStatus().then((s) => (syncStatus = s));
      }).then((fn) => (unlisten = fn));
    });
    return () => unlisten?.();
  });

  // 选中变化时加载内容
  let lastLoadedPath: string | null = null;
  $effect(() => {
    if (selectedPath && selectedPath !== lastLoadedPath) {
      lastLoadedPath = selectedPath;
      void loadPromptContent(selectedPath);
    }
  });

  async function loadPromptContent(path: string) {
    try {
      const { meta, body } = await readPrompt(path);
      applyMetaToEditFields(meta);
      editingBody = body;
      editorMode = "view";
    } catch (e) {
      const msg = String(e);
      if (msg.includes("FILE_NOT_FOUND")) {
        // 问题1：文件被外部删除 → 从列表移除，不报错卡死
        removePromptFromList(path);
      } else {
        error = msg;
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
      } else {
        selectedPath = null;
        lastLoadedPath = null;
      }
    }
    // 刷新分类计数
    void refresh();
  }

  async function doCopy(mode: "markdown" | "plain") {
    if (!selectedPrompt) return;
    try {
      await copyText(editingBody);
      copiedFlash = true;
      setTimeout(() => (copiedFlash = false), 800);
      await hideWindow();
    } catch (e) {
      showError(String(e));
    }
  }

  // 问题5修复：保存用结构化字段，Rust 端规范序列化
  async function doSave() {
    if (!selectedPrompt) return;
    try {
      const saved = await savePrompt(selectedPrompt.path, {
        title: editingTitle.trim() || "未命名",
        copy_mode: editingCopyMode,
        body: editingBody,
      });
      // 保存可能因标题变化而重命名了文件，用新路径更新选中
      selectedPath = saved.path;
      lastLoadedPath = saved.path; // 避免立即重载覆盖编辑内容
      await refresh();
      editorMode = "view";
    } catch (e) {
      const msg = String(e);
      if (msg.includes("FILE_NOT_FOUND")) {
        removePromptFromList(selectedPrompt.path);
      } else {
        error = msg;
      }
    }
  }

  async function doCreate() {
    const cat = selectedCategory === "__all__" ? "未分类" : selectedCategory;
    try {
      // 新建用占位标题（文件名是时间戳），进入编辑后用户填写真实标题
      // 保存时若标题变化会自动重命名文件
      const p = await createPrompt(cat, "");
      await refresh();
      selectedPath = p.path;
      lastLoadedPath = p.path;
      query = "";
      // 进入编辑，标题留空引导用户输入
      editingTitle = "";
      editingCategory = cat;
      editingCopyMode = "markdown";
      editingBody = "";
      editorMode = "edit";
    } catch (e) {
      showError(String(e));
    }
  }

  async function doDelete() {
    if (!selectedPrompt) return;
    if (!confirm(`确定删除「${selectedPrompt.title}」？此操作不可撤销。`)) return;
    try {
      await deletePrompt(selectedPrompt.path);
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
    try {
      await renamePrompt(
        contextMenu.prompt.path,
        contextMenu.prompt.title,
        category,
      );
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
        const c = a.category.localeCompare(b.category);
        if (c !== 0) return c;
        const oa = a.order ?? Number.MAX_SAFE_INTEGER;
        const ob = b.order ?? Number.MAX_SAFE_INTEGER;
        if (oa !== ob) return oa - ob;
        return b.meta.updated.localeCompare(a.meta.updated);
      });

    reorderInFlight = true;
    pendingRefresh = false;
    try {
      await reorderPrompts(categoryName, newPathOrder);
    } catch (e) {
      showError(String(e));
      await refresh();
    } finally {
      reorderInFlight = false;
      if (pendingRefresh) {
        pendingRefresh = false;
        await refresh();
      }
    }
  }

  function onCtxDelete() {
    if (!contextMenu.prompt) return;
    const p = contextMenu.prompt;
    if (!confirm(`确定删除「${p.title}」？此操作不可撤销。`)) return;
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
    try {
      const newPrompt = await renamePrompt(
        renameDialog.path,
        renameDialog.title.trim() || "未命名",
        renameDialog.category,
      );
      await refresh();
      selectedPath = newPrompt.path;
      lastLoadedPath = null;
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
      await refresh();
      catRenameDialog.open = false;
    } catch (e) {
      showError(String(e));
    }
  }

  // 键盘导航
  $effect(() => {
    if (visiblePrompts.length === 0) {
      if (selectedIndex !== 0) selectedIndex = 0;
      return;
    }
    if (selectedIndex >= visiblePrompts.length) {
      selectedIndex = visiblePrompts.length - 1;
    }
    const cur = visiblePrompts[selectedIndex];
    if (cur && cur.path !== selectedPath) {
      selectedPath = cur.path;
    }
  });

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      if (contextMenu.open) {
        contextMenu.open = false;
        return;
      }
      if (renameDialog.open) {
        renameDialog.open = false;
        return;
      }
      if (settingsOpen) {
        settingsOpen = false;
        return;
      }
      e.preventDefault();
      void hideWindow();
      return;
    }
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

    const tag = (e.target as HTMLElement)?.tagName;
    const inEditor = tag === "TEXTAREA" || tag === "INPUT" || tag === "SELECT";
    if (inEditor) return;

    if (e.key === "ArrowDown") {
      e.preventDefault();
      selectedIndex = Math.min(selectedIndex + 1, visiblePrompts.length - 1);
      scrollIntoView();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      selectedIndex = Math.max(selectedIndex - 1, 0);
      scrollIntoView();
    } else if (e.key === "Enter") {
      e.preventDefault();
      if (visiblePrompts[selectedIndex]) {
        void doCopy(visiblePrompts[selectedIndex].meta.copy_mode as "markdown" | "plain");
      }
    }
  }

  function scrollIntoView() {
    queueMicrotask(() => {
      scrollToIndexFn?.(selectedIndex);
    });
  }

  onMount(bootstrap);
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="app" role="application" aria-label="Prompt Pocket">
  {#if loading}
    <div class="state">
      <div class="spinner"></div>
      <span>正在加载提示词…</span>
    </div>
  {:else}
    <header class="topbar" data-tauri-drag-region>
      <div class="search-wrap">
        <span class="icon">⌕</span>
        <input
          id="search-input"
          type="text"
          placeholder="搜索提示词…"
          bind:value={query}
          autocomplete="off"
          spellcheck="false"
        />
        <button class="new-btn" onclick={doCreate} title="新建 (Ctrl+N)">+</button>
        {#if syncStatus?.configured}
          <span
            class="sync-indicator"
            class:syncing={syncStatus.syncing}
            class:error={!!syncStatus.lastError}
            title={syncStatus.lastError || syncStatus.lastSync || "已连接坚果云"}
          ></span>
        {/if}
        <button
          class="new-btn"
          onclick={() => (settingsOpen = true)}
          title="设置"
          aria-label="设置"
        >
          ⚙
        </button>
      </div>
    </header>

    <nav class="tabs" data-tauri-drag-region>
      <CategoryTabs
        {categories}
        total={allPrompts.length}
        bind:selected={selectedCategory}
        oncreate={onCreateCategory}
        onrename={onRenameCategory}
        oncontextmenu={onCatContextMenu}
      />
    </nav>

    <main class="body">
      <PromptList
        prompts={visiblePrompts}
        {selectedPath}
        {selectedIndex}
        draggable={canReorderPrompts}
        disabledReason={reorderDisabledReason}
        onmounted={(fn) => (scrollToIndexFn = fn)}
        onselect={(path) => {
          selectedPath = path;
          selectedIndex = visiblePrompts.findIndex((p) => p.path === path);
        }}
        oncontextmenu={openContextMenu}
        onreorder={doReorder}
      />

      <Editor
        prompt={selectedPrompt}
        mode={editorMode}
        bind:body={editingBody}
        bind:title={editingTitle}
        bind:category={editingCategory}
        bind:copyMode={editingCopyMode}
        {categories}
        oncopy={(m) => doCopy(m)}
        onsave={doSave}
        oncancel={() => {
          if (selectedPath) {
            lastLoadedPath = null;
            void loadPromptContent(selectedPath);
          }
        }}
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
        ✓ 已复制，回到原应用粘贴
      </div>
    {/if}

    {#if error}
      <div class="toast error-toast" transition:fly={{ y: 20 }}>
        <span class="error-text">{error}</span>
        <button class="error-close" onclick={() => (error = null)}>×</button>
      </div>
    {/if}

    <Settings bind:open={settingsOpen} onsynced={onSynced} />

    <ContextMenu
      bind:open={contextMenu.open}
      prompt={contextMenu.prompt}
      x={contextMenu.x}
      y={contextMenu.y}
      {categories}
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
        onkeydown={(e) => e.key === "Escape" && (renameDialog.open = false)}
        role="presentation"
      >
        <div class="dialog" transition:fly={{ y: -10, duration: 120 }}>
          <h3>重命名 / 移动分类</h3>
          <div class="dialog-row">
            <label for="rn-title">标题</label>
            <input id="rn-title" type="text" bind:value={renameDialog.title} />
          </div>
          <div class="dialog-row">
            <label for="rn-cat">分类</label>
            <select id="rn-cat" bind:value={renameDialog.category}>
              <option value={"未分类"}>未分类</option>
              {#each categories as c}
                <option value={c.name}>{c.name}</option>
              {/each}
            </select>
          </div>
          <div class="dialog-actions">
            <button class="ghost" onclick={() => (renameDialog.open = false)}>
              取消
            </button>
            <button class="primary" onclick={submitRename}>确定</button>
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
          <span class="ico">✎</span> 重命名分类
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
        onkeydown={(e) => e.key === "Escape" && (catRenameDialog.open = false)}
        role="presentation"
      >
        <div class="dialog" transition:fly={{ y: -10, duration: 120 }}>
          <h3>重命名分类</h3>
          <div class="dialog-row">
            <label for="cat-rn">新分类名</label>
            <input
              id="cat-rn"
              type="text"
              bind:value={catRenameDialog.newName}
              onkeydown={(e) => e.key === "Enter" && submitCatRename()}
            />
          </div>
          <div class="dialog-actions">
            <button class="ghost" onclick={() => (catRenameDialog.open = false)}>
              取消
            </button>
            <button class="primary" onclick={submitCatRename}>确定</button>
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
</style>
