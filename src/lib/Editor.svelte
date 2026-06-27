<script lang="ts">
  import type { CategoryCount, Prompt } from "./types";

  let {
    prompt,
    mode,
    body = $bindable(""),
    // 结构化编辑字段（双向绑定）
    title = $bindable(""),
    category = $bindable(""),
    pinned = $bindable(false),
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
    pinned: boolean;
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

  // 极简 Markdown → HTML 渲染
  function renderMarkdown(src: string): string {
    if (!src || !src.trim()) return '<p class="empty-body">（无内容）</p>';
    const esc = (s: string) =>
      s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

    const lines = esc(src).split("\n");
    let html = "";
    let inCode = false;
    let inList = false;
    let inOrder = false;

    const closeList = () => {
      if (inList) {
        html += "</ul>";
        inList = false;
      }
      if (inOrder) {
        html += "</ol>";
        inOrder = false;
      }
    };

    for (const line of lines) {
      if (line.trim().startsWith("```")) {
        closeList();
        if (inCode) {
          html += "</code></pre>";
          inCode = false;
        } else {
          html += "<pre><code>";
          inCode = true;
        }
        continue;
      }
      if (inCode) {
        html += line + "\n";
        continue;
      }
      const h = line.match(/^(#{1,4})\s+(.*)$/);
      if (h) {
        closeList();
        const level = h[1].length + 1;
        html += `<h${level}>${inline(h[2])}</h${level}>`;
        continue;
      }
      if (/^\d+\.\s+/.test(line)) {
        if (!inOrder) {
          closeList();
          html += "<ol>";
          inOrder = true;
        }
        html += `<li>${inline(line.replace(/^\d+\.\s+/, ""))}</li>`;
        continue;
      }
      if (/^[-*]\s+/.test(line)) {
        if (!inList) {
          closeList();
          html += "<ul>";
          inList = true;
        }
        html += `<li>${inline(line.replace(/^[-*]\s+/, ""))}</li>`;
        continue;
      }
      closeList();
      if (line.trim() !== "") {
        html += `<p>${inline(line)}</p>`;
      }
    }
    closeList();
    if (inCode) html += "</code></pre>";
    return html;
  }

  function inline(s: string): string {
    return s
      .replace(/`([^`]+)`/g, "<code>$1</code>")
      .replace(/\*\*([^*]+)\*\*/g, "<strong>$1</strong>")
      .replace(/\*([^*]+)\*/g, "<em>$1</em>")
      .replace(
        /\[([^\]]+)\]\(([^)]+)\)/g,
        '<a href="$2" target="_blank">$1</a>',
      );
  }
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
        {#if pinned}<span class="pin">★</span>{/if}
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
      <div class="preview prose">
        {@html renderMarkdown(body)}
      </div>
      <footer class="editor-foot">
        <button class="primary" onclick={() => oncopy("markdown")}>
          复制
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

        <div class="form-row">
          <label class="checkbox-wrap">
            <input type="checkbox" bind:checked={pinned} />
            <span>置顶（显示在列表顶部）</span>
          </label>
        </div>

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
    height: 100%;
    background: var(--bg);
  }
  .editor.empty {
    align-items: center;
    justify-content: center;
  }

  .editor-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 20px;
    border-bottom: 1px solid var(--border);
    gap: 8px;
  }
  .title-block {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .pin {
    font-size: 12px;
    color: var(--fg);
  }
  .title {
    font-size: 15px;
    font-weight: 600;
    margin: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .text-btn {
    background: transparent;
    border: none;
    color: var(--muted);
    font-size: 13px;
    padding: 4px 8px;
    border-radius: 4px;
    cursor: pointer;
    transition: all 0.12s;
  }
  .text-btn:hover {
    color: var(--fg);
    background: var(--bg-hover);
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
    overflow-y: auto;
    padding: 24px 32px;
    font-size: 14px;
    line-height: 1.75;
    color: var(--fg);
  }
  .preview :global(.empty-body) {
    color: var(--muted);
    font-style: italic;
  }

  .editor-foot {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 20px;
    border-top: 1px solid var(--border);
    background: var(--bg);
  }
  .editor-foot kbd {
    display: inline-block;
    margin-left: 6px;
    padding: 1px 5px;
    font-size: 10px;
    font-family: ui-monospace, monospace;
    background: rgba(255, 255, 255, 0.15);
    border: 1px solid rgba(255, 255, 255, 0.25);
    border-radius: 3px;
    line-height: 1.4;
  }
  .meta-info {
    margin-left: auto;
    font-size: 12px;
    color: var(--muted);
  }

  /* ── 填空式表单 ── */
  .edit-area {
    flex: 1;
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
    font-family: inherit;
    outline: none;
    width: 100%;
    box-sizing: border-box;
  }
  .form-input:focus {
    border-color: var(--fg);
  }
  select.form-input {
    cursor: pointer;
  }
  .checkbox-wrap {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
    cursor: pointer;
    user-select: none;
  }
  .checkbox-wrap input {
    width: 15px;
    height: 15px;
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
    color: var(--fg);
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
    font-family: ui-monospace, "Cascadia Code", Consolas, monospace;
    font-size: 13px;
    line-height: 1.6;
    resize: none;
    outline: none;
    min-height: 160px;
  }
  .body-input:focus {
    border-color: var(--fg);
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
    background: var(--bg-elevated);
    padding: 1px 5px;
    border-radius: 4px;
    font-family: ui-monospace, "Cascadia Code", Consolas, monospace;
    font-size: 12.5px;
  }
  .prose :global(pre) {
    background: var(--bg-elevated);
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
  }
</style>
