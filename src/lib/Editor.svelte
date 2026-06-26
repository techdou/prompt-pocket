<script lang="ts">
  import type { Prompt } from "./types";

  let {
    prompt,
    mode,
    body = $bindable(""),
    meta = $bindable(""),
    oncopy,
    onsave,
    oncancel,
    onedit,
    onreveal,
    ondelete,
  }: {
    prompt: Prompt | null;
    mode: "view" | "edit";
    body: string;
    meta: string;
    oncopy: (mode: "markdown" | "plain") => void;
    onsave: () => void;
    oncancel: () => void;
    onedit: () => void;
    onreveal: () => void;
    ondelete: () => void;
  } = $props();

  // 极简 Markdown → HTML 渲染，仅处理标题/粗体/列表/代码块/行内代码
  function renderMarkdown(src: string): string {
    const esc = (s: string) =>
      s
        .replace(/&/g, "&amp;")
        .replace(/</g, "&lt;")
        .replace(/>/g, "&gt;");

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
      // 代码围栏
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

      // 标题
      const h = line.match(/^(#{1,4})\s+(.*)$/);
      if (h) {
        closeList();
        const level = h[1].length + 1; // h2..h5，避免与页面 h1 冲突
        html += `<h${level}>${inline(h[2])}</h${level}>`;
        continue;
      }

      // 有序列表
      if (/^\d+\.\s+/.test(line)) {
        if (!inOrder) {
          closeList();
          html += "<ol>";
          inOrder = true;
        }
        html += `<li>${inline(line.replace(/^\d+\.\s+/, ""))}</li>`;
        continue;
      }
      // 无序列表
      if (/^[-*]\s+/.test(line)) {
        if (!inList) {
          closeList();
          html += "<ul>";
          inList = true;
        }
        html += `<li>${inline(line.replace(/^[-*]\s+/, ""))}</li>`;
        continue;
      }

      // 普通段落
      closeList();
      if (line.trim() === "") {
        html += "";
      } else {
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
        {#if prompt.meta.pinned}<span class="pin">★</span>{/if}
        <h2 class="title">{prompt.title}</h2>
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
        <button class="ghost" onclick={() => oncopy("plain")}>
          复制纯文本
        </button>
        <span class="meta-info">
          {prompt.category}
          {#if prompt.meta.tags.length > 0}
            · {prompt.meta.tags.map((t) => "#" + t).join(" ")}
          {/if}
        </span>
      </footer>
    {:else}
      <!-- 编辑：frontmatter + 正文 -->
      <div class="edit-area">
        <span class="fm-label">元数据 (YAML frontmatter)</span>
        <textarea
          class="fm-input"
          bind:value={meta}
          spellcheck="false"
          rows="6"
          placeholder="title: 标题&#10;tags: [标签1, 标签2]&#10;copy_mode: markdown"
        ></textarea>
        <span class="fm-label">正文 (Markdown)</span>
        <textarea
          class="body-input"
          bind:value={body}
          spellcheck="false"
          placeholder="在这里写提示词内容…"
        ></textarea>
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

  .edit-area {
    flex: 1;
    display: flex;
    flex-direction: column;
    padding: 12px 16px;
    gap: 6px;
    min-height: 0;
  }
  .fm-label {
    font-size: 11px;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .fm-input,
  .body-input {
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    color: var(--fg);
    border-radius: 6px;
    padding: 8px 10px;
    font-family: "Cascadia Code", "Fira Code", Consolas, monospace;
    font-size: 13px;
    line-height: 1.6;
    resize: none;
    outline: none;
    flex-shrink: 0;
  }
  .fm-input:focus,
  .body-input:focus {
    border-color: var(--accent);
  }
  .body-input {
    flex: 1;
    min-height: 200px;
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
    font-family: "Cascadia Code", "Fira Code", Consolas, monospace;
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
