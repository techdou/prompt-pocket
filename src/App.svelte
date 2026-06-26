<script lang="ts">
  import { onMount } from "svelte";
  import { fly } from "svelte/transition";
  import type { AppConfig, CategoryCount, Prompt } from "./lib/types";
  import {
    copyText,
    createPrompt,
    deletePrompt,
    hideWindow,
    initApp,
    readPrompt,
    revealInFinder,
    savePrompt,
    scanPrompts,
  } from "./lib/api";
  import { filterPrompts } from "./lib/search";
  import CategoryTabs from "./lib/CategoryTabs.svelte";
  import PromptList from "./lib/PromptList.svelte";
  import Editor from "./lib/Editor.svelte";
  import Settings from "./lib/Settings.svelte";

  let config: AppConfig | null = null;
  let allPrompts: Prompt[] = $state([]);
  let categories: CategoryCount[] = $state([]);

  let selectedCategory = $state<string>("__all__");
  let query = $state("");
  let selectedPath = $state<string | null>(null);

  // 编辑器状态
  let editorMode = $state<"view" | "edit">("view");
  let editingBody = $state("");
  let editingMeta = $state("");

  let loading = $state(true);
  let error = $state<string | null>(null);
  let copiedFlash = $state(false);
  let settingsOpen = $state(false);

  // 选中项
  let selectedPrompt = $derived(
    allPrompts.find((p) => p.path === selectedPath) ?? null,
  );

  // 分类过滤
  let categoryFiltered = $derived(
    selectedCategory === "__all__"
      ? allPrompts
      : allPrompts.filter((p) => p.category === selectedCategory),
  );

  // 搜索过滤后的最终列表
  let visiblePrompts = $derived(filterPrompts(categoryFiltered, query));

  // 列表项 DOM，键盘滚动跟随
  let listRefs: HTMLLIElement[] = $state([]);
  let selectedIndex = $state(0);

  async function bootstrap() {
    try {
      loading = true;
      config = await initApp();
      await refresh();
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
    if (!selectedPath && visiblePrompts.length > 0) {
      selectedPath = visiblePrompts[0].path;
    }
  }

  // 设置界面切换数据目录后：更新配置、重置选中、重新扫描
  async function onConfigChanged(newConfig: AppConfig) {
    config = newConfig;
    selectedPath = null;
    lastLoadedPath = null;
    selectedCategory = "__all__";
    query = "";
    await refresh();
    settingsOpen = false;
  }

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
      const { meta_raw, body } = await readPrompt(path);
      editingMeta = meta_raw;
      editingBody = body;
      editorMode = "view";
    } catch (e) {
      error = String(e);
    }
  }

  async function doCopy(mode: "markdown" | "plain") {
    if (!selectedPrompt) return;
    const text = editingBody; // MVP：markdown/plain 都复制正文（plain 后续可去格式）
    try {
      await copyText(text);
      copiedFlash = true;
      setTimeout(() => (copiedFlash = false), 800);
      await hideWindow(); // 复制后隐藏，回到原应用粘贴
    } catch (e) {
      error = String(e);
    }
  }

  async function doSave() {
    if (!selectedPrompt) return;
    try {
      const full = `---\n${editingMeta.trim()}\n---\n\n${editingBody}`;
      await savePrompt(selectedPrompt.path, full);
      await refresh();
      editorMode = "view";
    } catch (e) {
      error = String(e);
    }
  }

  async function doCreate() {
    const cat =
      selectedCategory === "__all__" ? "未分类" : selectedCategory;
    const title = `新提示词`;
    try {
      const p = await createPrompt(cat, title);
      await refresh();
      selectedPath = p.path;
      query = "";
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
      await refresh();
    } catch (e) {
      error = String(e);
    }
  }

  // 键盘导航：selectedIndex 跟随 visiblePrompts
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
      const el = document.querySelector<HTMLInputElement>("#search-input");
      el?.focus();
      el?.select();
      return;
    }

    const tag = (e.target as HTMLElement)?.tagName;
    const inEditor =
      tag === "TEXTAREA" || tag === "INPUT" || tag === "SELECT";
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
      listRefs[selectedIndex]?.scrollIntoView({ block: "nearest" });
    });
  }

  onMount(bootstrap);
</script>

<svelte:window on:keydown={handleKeydown} />

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
    </div>
  {:else}
    <!-- 顶栏：搜索框（极简单行，左侧放大镜） -->
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

    <!-- 横向分类 Tab -->
    <nav class="tabs" data-tauri-drag-region>
      <CategoryTabs
        {categories}
        total={allPrompts.length}
        bind:selected={selectedCategory}
      />
    </nav>

    <!-- 主体：左列表 + 右内容 -->
    <main class="body">
      <PromptList
        prompts={visiblePrompts}
        {selectedPath}
        {selectedIndex}
        bind:listRefs
        onselect={(path) => {
          selectedPath = path;
          selectedIndex = visiblePrompts.findIndex((p) => p.path === path);
        }}
      />

      <Editor
        prompt={selectedPrompt}
        mode={editorMode}
        bind:body={editingBody}
        bind:meta={editingMeta}
        oncopy={(m) => doCopy(m)}
        onsave={doSave}
        oncancel={() => {
          if (selectedPath) void loadPromptContent(selectedPath);
        }}
        onedit={() => (editorMode = "edit")}
        onreveal={() =>
          selectedPrompt && void revealInFinder(selectedPrompt.path)}
        ondelete={doDelete}
      />
    </main>

    {#if copiedFlash}
      <div class="toast" transition:fly={{ y: 20 }}>
        ✓ 已复制，回到原应用粘贴
      </div>
    {/if}

    <Settings bind:open={settingsOpen} onchanged={onConfigChanged} />
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

  /* 顶栏：单行搜索 */
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

  /* 主体 */
  .body {
    flex: 1;
    display: grid;
    grid-template-columns: 300px 1fr;
    min-height: 0;
  }

  /* 横向分类 Tab 栏 */
  .tabs {
    flex-shrink: 0;
    height: 40px;
    border-bottom: 1px solid var(--border);
    background: var(--bg);
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

  :global(button) {
    cursor: pointer;
    font-family: inherit;
  }
  :global(.ghost) {
    background: transparent;
    border: 1px solid var(--border-strong);
    color: var(--fg);
    border-radius: 6px;
    padding: 5px 12px;
    font-size: 13px;
    transition: all 0.12s;
  }
  :global(.ghost:hover) {
    background: var(--bg-hover);
    border-color: var(--fg);
  }
  :global(.primary) {
    background: var(--fg);
    border: 1px solid var(--fg);
    color: var(--bg);
    border-radius: 6px;
    padding: 5px 14px;
    font-size: 13px;
  }
  :global(.primary:hover) {
    opacity: 0.85;
  }
  :global(.danger) {
    background: transparent;
    border: 1px solid var(--danger);
    color: var(--danger);
    border-radius: 6px;
    padding: 5px 12px;
    font-size: 13px;
  }
</style>
