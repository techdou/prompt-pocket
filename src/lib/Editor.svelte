<script lang="ts">
  import { tick } from "svelte";
  import type { CategoryCount, Prompt } from "./types";
  import { renderMarkdown } from "./markdown";
  import { renderRich } from "./renderers";
  import { openUrl } from "./api";

  let {
    prompt,
    mode,
    body = $bindable(""),
    // 结构化编辑字段（双向绑定）
    title = $bindable(""),
    category = $bindable(""),
    copyMode = $bindable<"markdown" | "plain">("markdown"),
    // 分类列表（编辑模式下的下拉选项）
    categories = [],
    oncopy,
    onsave,
    oncancel,
    onedit,
    onreveal,
    ondelete,
    oncreatecategory,
  }: {
    prompt: Prompt | null;
    mode: "view" | "edit";
    body: string;
    title: string;
    category: string;
    copyMode: "markdown" | "plain";
    categories: CategoryCount[];
    oncopy: (mode: "markdown" | "plain") => void;
    onsave: () => void;
    oncancel: () => void;
    onedit: () => void;
    onreveal: () => void;
    ondelete: () => void;
    oncreatecategory: (name: string) => void;
  } = $props();

  // 分类下拉里加一个"未分类"选项（根目录）
  let categoryOptions = $derived(["未分类", ...categories.map((c) => c.name)]);

  // 新建分类输入态
  let newCategoryName = $state("");
  let addingCategory = $state(false);

  function addCategory() {
    const name = newCategoryName.trim();
    if (!name) return;
    oncreatecategory(name);
    category = name;
    addingCategory = false;
    newCategoryName = "";
  }

  // preview 容器引用，供 renderRich 扫描 DOM
  let previewEl: HTMLDivElement | undefined = $state();

  // 链接点击拦截：<a> 走 openUrl 在系统浏览器打开，而非 webview 内导航
  function onPreviewClick(e: MouseEvent) {
    const target = e.target as HTMLElement;
    const anchor = target.closest("a");
    if (!anchor) return;
    const href = anchor.getAttribute("href");
    if (!href || href === "#") return;
    e.preventDefault();
    void openUrl(href);
  }

  // body 变化 → marked 同步渲染（秒出 GFM）→ tick 后异步 CDN 增强（mermaid/katex/高亮）
  $effect(() => {
    if (!previewEl || mode !== "view") return;
    void tick().then(() => renderRich(previewEl!));
  });
</script>

{#if !prompt}
  <section class="editor empty">
    <div class="placeholder">
      <div class="big">📝</div>
      <p>选中一条提示词查看详情</p>
      <p class="hint">或按 Ctrl+N 新建</p>
    </div>
  </section>
{:else}
  <section class="editor">
    <header class="editor-head">
      <div class="title-block">
        <h2 class="title">{title || prompt.title}</h2>
      </div>
      <div class="actions">
        {#if mode === "view"}
          <button class="text-btn" onclick={onedit} title="编辑">编辑</button>
          <button
            class="text-btn"
            onclick={onreveal}
            title="在文件管理器中显示"
          >
            显示文件
          </button>
          <button class="text-btn danger" onclick={ondelete} title="删除">
            删除
          </button>
        {:else}
          <button class="primary" onclick={onsave}>保存</button>
          <button class="ghost" onclick={oncancel}>取消</button>
        {/if}
      </div>
    </header>

    {#if mode === "view"}
      <!-- 预览 + 复制 -->
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="preview prose"
        bind:this={previewEl}
        onclick={onPreviewClick}
      >
        {@html renderMarkdown(body)}
      </div>
      <footer class="editor-foot">
        <button
          class="copy-action"
          onclick={() => oncopy("markdown")}
          title="复制 (Enter)"
          aria-label="复制提示词"
        >
          <span class="copy-icon" aria-hidden="true">⧉</span>
          <span class="copy-label">复制</span>
          <kbd>Enter</kbd>
        </button>
        <span class="meta-info">
          {prompt.category}
        </span>
      </footer>
    {:else}
      <!-- 填空式表单：结构化字段，用户完全不碰 YAML -->
      <div class="edit-area">
        <div class="form-row">
          <label class="form-label" for="f-title">标题</label>
          <input
            id="f-title"
            class="form-input"
            type="text"
            bind:value={title}
            placeholder="给这条提示词起个名字"
          />
        </div>

        <div class="form-row">
          <label class="form-label" for="f-category">分类</label>
          <select
            id="f-category"
            class="form-input"
            bind:value={category}
          >
            {#each categoryOptions as c}
              <option value={c}>{c}</option>
            {/each}
          </select>
        </div>

        {#if addingCategory}
          <div class="form-row new-cat-row">
            <input
              class="form-input"
              type="text"
              bind:value={newCategoryName}
              placeholder="新分类名"
              onkeydown={(e) => e.key === "Enter" && addCategory()}
            />
            <button class="ghost" onclick={addCategory}>添加</button>
            <button
              class="ghost"
              onclick={() => {
                addingCategory = false;
                newCategoryName = "";
              }}
            >
              取消
            </button>
          </div>
        {:else}
          <button class="link-btn" onclick={() => (addingCategory = true)}>
            + 新建分类
          </button>
        {/if}

        <div class="form-row form-row-body">
          <label class="form-label" for="f-body">正文</label>
          <textarea
            id="f-body"
            class="body-input"
            bind:value={body}
            spellcheck="false"
            placeholder="在这里写提示词内容…支持 Markdown 语法"
          ></textarea>
        </div>
      </div>
    {/if}
  </section>
{/if}

<style>
  .editor {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    height: 100%;
    max-height: 100%;
    overflow: hidden;
    background: var(--bg-elevated);
  }
  .editor.empty {
    align-items: center;
    justify-content: center;
  }

  .editor-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-shrink: 0;
    min-height: 58px;
    padding: 0 28px;
    border-bottom: 1px solid var(--border);
    background: var(--bg-elevated);
    gap: 8px;
  }
  .title-block {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .title {
    font-family: var(--font-ui);
    font-size: 15px;
    font-weight: 600;
    margin: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .text-btn {
    background: transparent;
    border: 1px solid transparent;
    color: var(--muted);
    font-size: 13px;
    padding: 5px 9px;
    border-radius: 7px;
    cursor: pointer;
    transition:
      background 0.12s,
      border-color 0.12s,
      color 0.12s;
  }
  .text-btn:hover {
    color: var(--accent);
    background: var(--bg-hover);
    border-color: var(--border);
  }
  .text-btn.danger:hover {
    color: var(--danger);
  }
  .actions {
    display: flex;
    gap: 6px;
    flex-shrink: 0;
  }

  .preview {
    flex: 1;
    min-height: 0; /* 关键：让 flex 子项在内容超长时收缩，footer 固定在底部 */
    overflow-y: auto;
    overflow-x: hidden;
    padding: 30px 44px 36px;
    font-family: var(--font-ui);
    font-size: 15px;
    line-height: 1.78;
    color: var(--fg);
    overflow-wrap: anywhere;
    word-break: break-word;
  }
  .preview :global(.empty-body) {
    color: var(--muted);
    font-style: italic;
  }

  .editor-foot {
    display: flex;
    align-items: center;
    flex-shrink: 0;
    gap: 12px;
    min-height: 56px;
    padding: 10px 28px;
    border-top: 1px solid var(--border);
    background: #f8fbff;
  }
  .copy-action {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    min-width: 112px;
    height: 34px;
    padding: 0 12px;
    border: 1px solid var(--accent);
    border-radius: 7px;
    background: var(--accent);
    color: #ffffff;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    box-shadow: 0 6px 18px rgba(37, 99, 235, 0.18);
    transition:
      transform 0.08s,
      box-shadow 0.12s,
      opacity 0.12s;
  }
  .copy-action:hover {
    background: var(--accent-hover);
    border-color: var(--accent-hover);
    box-shadow: 0 8px 22px rgba(37, 99, 235, 0.22);
  }
  .copy-action:active {
    transform: translateY(1px);
    box-shadow: none;
  }
  .copy-action:focus-visible {
    outline: 3px solid var(--accent-soft);
    outline-offset: 2px;
  }
  .copy-icon {
    font-size: 14px;
    line-height: 1;
    opacity: 0.9;
  }
  .copy-label {
    line-height: 1;
  }
  .copy-action kbd {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    height: 20px;
    min-width: 40px;
    padding: 0 6px;
    font-size: 11px;
    font-family: var(--font-mono);
    font-weight: 500;
    line-height: 1;
    color: inherit;
    background: rgba(255, 255, 255, 0.2);
    border: 1px solid rgba(255, 255, 255, 0.35);
    border-radius: 5px;
  }
  .meta-info {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    min-height: 24px;
    max-width: 220px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg-elevated);
    font-size: 12px;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* ── 填空式表单 ── */
  .edit-area {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 18px 24px;
    display: flex;
    flex-direction: column;
    gap: 14px;
    min-height: 0;
  }
  .form-row {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .form-label {
    font-size: 11px;
    font-weight: 600;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .form-input {
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    color: var(--fg);
    border-radius: 6px;
    padding: 7px 10px;
    font-size: 13px;
    font-family: var(--font-ui);
    outline: none;
    width: 100%;
    box-sizing: border-box;
  }
  .form-input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  select.form-input {
    cursor: pointer;
  }
  .new-cat-row {
    flex-direction: row;
    gap: 6px;
    align-items: center;
  }
  .link-btn {
    align-self: flex-start;
    background: transparent;
    border: none;
    color: var(--muted);
    font-size: 12px;
    padding: 2px 0;
    cursor: pointer;
  }
  .link-btn:hover {
    color: var(--accent);
  }

  .form-row-body {
    flex: 1;
    min-height: 180px;
  }
  .body-input {
    flex: 1;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    color: var(--fg);
    border-radius: 6px;
    padding: 10px 12px;
    font-family: var(--font-mono);
    font-size: 13px;
    line-height: 1.6;
    resize: none;
    outline: none;
    min-height: 160px;
  }
  .body-input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }

  .placeholder {
    text-align: center;
    color: var(--muted);
  }
  .placeholder .big {
    font-size: 48px;
    opacity: 0.4;
    margin-bottom: 12px;
  }
  .placeholder .hint {
    font-size: 12px;
    margin-top: 4px;
  }

  /* prose 样式 */
  .prose :global(h2) {
    font-size: 18px;
    margin: 16px 0 8px;
  }
  .prose :global(h3) {
    font-size: 16px;
    margin: 12px 0 6px;
  }
  .prose :global(p) {
    margin: 8px 0;
  }
  .prose :global(ul),
  .prose :global(ol) {
    margin: 8px 0;
    padding-left: 24px;
  }
  .prose :global(li) {
    margin: 3px 0;
  }
  .prose :global(code) {
    background: var(--accent-soft);
    padding: 1px 5px;
    border-radius: 4px;
    font-family: var(--font-mono);
    font-size: 12.5px;
  }
  .prose :global(pre) {
    background: #f8fbff;
    border: 1px solid var(--border);
    padding: 12px;
    border-radius: 8px;
    overflow-x: auto;
    margin: 10px 0;
  }
  .prose :global(pre code) {
    background: transparent;
    padding: 0;
    font-size: 13px;
    line-height: 1.5;
  }
  .prose :global(strong) {
    font-weight: 600;
  }
  .prose :global(a) {
    color: var(--accent);
    cursor: pointer;
  }

  /* ── GFM 扩展语法 ── */
  .prose :global(blockquote) {
    margin: 10px 0;
    padding: 6px 14px;
    border-left: 3px solid var(--accent);
    background: var(--accent-soft);
    border-radius: 0 6px 6px 0;
    color: var(--muted);
  }
  .prose :global(blockquote p) {
    margin: 4px 0;
  }
  .prose :global(hr) {
    border: none;
    border-top: 1px solid var(--border);
    margin: 16px 0;
  }
  .prose :global(del) {
    color: var(--muted);
  }
  .prose :global(table) {
    border-collapse: collapse;
    width: auto;
    max-width: 100%;
    margin: 10px 0;
    font-size: 13.5px;
  }
  .prose :global(th),
  .prose :global(td) {
    border: 1px solid var(--border);
    padding: 6px 12px;
    text-align: left;
  }
  .prose :global(th) {
    background: var(--bg-hover);
    font-weight: 600;
  }
  .prose :global(tr:nth-child(2n) td) {
    background: rgba(0, 0, 0, 0.015);
  }
  /* 任务列表：去掉默认圆点，复选框居左 */
  .prose :global(li input[type="checkbox"]) {
    margin-right: 6px;
    vertical-align: middle;
  }
  .prose :global(li:has(input[type="checkbox"])) {
    list-style: none;
    margin-left: -18px;
  }

  /* ── 代码高亮（highlight.js atom-one-dark 主题由 CDN 注入，这里只兜底背景）── */
  .prose :global(pre code.hljs) {
    background: transparent;
    padding: 0;
  }

  /* ── Mermaid 容器 ── */
  .prose :global(.mermaid) {
    display: flex;
    justify-content: center;
    padding: 12px;
    background: #fff;
    border: 1px solid var(--border);
    border-radius: 8px;
    margin: 10px 0;
    overflow-x: auto;
  }
  .prose :global(.mermaid svg) {
    max-width: 100%;
    height: auto;
  }

  /* ── KaTeX 占位与渲染结果 ── */
  .prose :global(.katex-placeholder) {
    background: var(--accent-soft);
    padding: 1px 5px;
    border-radius: 4px;
    font-family: var(--font-mono);
    font-size: 12.5px;
  }
  .prose :global(.katex-block) {
    display: block;
    margin: 12px 0;
    overflow-x: auto;
    text-align: center;
  }

  /* ── CDN 加载失败的源码回退 ── */
  .prose :global(.render-fallback) {
    background: #fff5f5;
    border: 1px solid #f5c2c2;
    border-left: 3px solid var(--danger);
    color: #666;
    padding: 10px 12px;
    border-radius: 6px;
    font-family: var(--font-mono);
    font-size: 12.5px;
    overflow-x: auto;
    margin: 10px 0;
  }
</style>
