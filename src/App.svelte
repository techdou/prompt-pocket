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
      error = String(e);
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

  // 监听后端 sync-finished 事件，自动刷新
  let unlisten: (() => void) | null = null;
  $effect(() => {
    import("@tauri-apps/api/event").then(({ listen }) => {
      listen("sync-finished", () => {
        void refresh();
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
      error = String(e);
    }
  }

  // 问题5修复：保存用结构化字段，Rust 端规范序列化
  async function doSave() {
    if (!selectedPrompt) return;
    try {
      await savePrompt(selectedPrompt.path, {
        title: editingTitle.trim() || "未命名",
        copy_mode: editingCopyMode,
        body: editingBody,
      });
      await refresh();
      lastLoadedPath = selectedPath; // 避免立即重载覆盖
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
      const p = await createPrompt(cat, "新提示词");
      await refresh();
      selectedPath = p.path;
      lastLoadedPath = null;
      query = "";
      // 进入编辑，字段初始化
      editingTitle = "新提示词";
      editingCategory = cat;
      editingCopyMode = "markdown";
      editingBody = "";
      editorMode = "edit";
    } catch (e) {
      error = String(e);
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
      error = String(e);
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
      error = String(e);
    }
  }

  // 拖拽排序：本地重排 + 持久化 + 推送云端
  async function doReorder(from: number, to: number) {
    if (from === to) return;
    // 只在非搜索、非"全部"视图下允许拖拽（搜索结果是过滤后的，"全部"是跨分类的）
    if (query.trim() || selectedCategory === "__all__") return;

    const reordered = [...visiblePrompts];
    const [moved] = reordered.splice(from, 1);
    reordered.splice(to, 0, moved);
    allPrompts = [...allPrompts]; // 触发响应式

    // 重建当前分类的 prompts 顺序（用 visiblePrompts 的新顺序覆盖 allPrompts 里该分类的部分）
    const categoryName = selectedCategory;
    const newPathOrder = reordered.map((p) => p.path);
    // 同步更新 allPrompts 里该分类项的顺序
    const others = allPrompts.filter((p) => p.category !== categoryName);
    allPrompts = [...others, ...reordered];

    try {
      await reorderPrompts(categoryName, newPathOrder);
    } catch (e) {
      error = String(e);
      await refresh(); // 失败则回滚
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
      .catch((e) => (error = String(e)));
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
      error = String(e);
    }
  }

  // 问题3：新建分类
  async function onCreateCategory(name: string) {
    try {
      await createCategory(name);
      await refresh();
    } catch (e) {
      error = String(e);
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
      error = String(e);
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
  {:else if error}
    <div class="state error">
      <strong>出错了</strong>
      <pre>{error}</pre>
      <button class="ghost" onclick={() => (error = null)}>关闭</button>
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
        draggable={!query.trim() && selectedCategory !== "__all__"}
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
    background: var(--bg);
    color: var(--fg);
    font-size: 14px;
  }

  .topbar {
    flex-shrink: 0;
    padding: 12px 16px 10px;
    border-bottom: 1px solid var(--border);
    background: var(--bg);
  }
  .search-wrap {
    display: flex;
    align-items: center;
    gap: 8px;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 6px;
    padding: 0 10px;
    height: 34px;
    transition: border-color 0.12s;
  }
  .search-wrap:focus-within {
    border-color: var(--fg);
  }
  .icon {
    color: var(--muted);
    font-size: 16px;
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
    width: 24px;
    height: 24px;
    border-radius: 6px;
    border: none;
    background: var(--bg-active);
    color: var(--fg);
    font-size: 18px;
    line-height: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: background 0.12s;
  }
  .new-btn:hover {
    background: var(--fg);
    color: var(--bg);
  }

  .sync-indicator {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: #22a06b;
    flex-shrink: 0;
    cursor: help;
  }
  .sync-indicator.syncing {
    background: #4a7cf7;
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
    grid-template-columns: 300px 1fr;
    min-height: 0;
  }

  .tabs {
    flex-shrink: 0;
    height: 40px;
    border-bottom: 1px solid var(--border);
    background: var(--bg);
    padding: 0 16px;
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
  .state.error pre {
    max-width: 80%;
    white-space: pre-wrap;
    color: var(--danger);
    background: var(--bg-elevated);
    padding: 12px;
    border-radius: 6px;
  }

  .spinner {
    width: 28px;
    height: 28px;
    border: 2px solid var(--border);
    border-top-color: var(--fg);
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
    background: var(--fg);
    color: var(--bg);
    padding: 8px 16px;
    border-radius: 6px;
    font-size: 13px;
    z-index: 100;
  }

  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.35);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 200;
    backdrop-filter: blur(2px);
  }
  .dialog {
    width: 380px;
    max-width: 90vw;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.25);
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
    border-radius: 6px;
    padding: 7px 10px;
    font-size: 13px;
    outline: none;
  }
  .dialog-row input:focus,
  .dialog-row select:focus {
    border-color: var(--fg);
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
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 8px 28px rgba(0, 0, 0, 0.16);
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
    border-radius: 5px;
    cursor: pointer;
    text-align: left;
    font-family: inherit;
  }
  .cat-menu-item:hover {
    background: var(--bg-hover);
  }
  .cat-menu-item .ico {
    width: 16px;
    text-align: center;
    opacity: 0.8;
  }
</style>
