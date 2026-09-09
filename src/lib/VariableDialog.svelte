<script lang="ts">
  import { fly } from "svelte/transition";
  import type { Translator } from "./i18n";
  import { autofocus } from "./actions";

  let {
    variables,
    t,
    onconfirm,
    oncopyraw,
    oncancel,
  }: {
    variables: string[];
    t: Translator;
    /** values 只含非空输入：留空的变量由 applyVariables 保留原样 */
    onconfirm: (values: Record<string, string>) => void;
    /** 逃生门：跳过替换，按原文复制（正文恰好天然含 {{}} 时用） */
    oncopyraw: () => void;
    oncancel: () => void;
  } = $props();

  let values = $state<Record<string, string>>({});

  function submit() {
    const filled: Record<string, string> = {};
    for (const name of variables) {
      const value = (values[name] ?? "").trim();
      if (value) filled[name] = value;
    }
    onconfirm(filled);
  }
</script>

<!-- Esc 拦截与 renameDialog 同模式：stopPropagation 阻止冒泡到
     <svelte:window>，避免关弹窗的同时把整个窗口也隐藏 -->
<div
  class="backdrop"
  transition:fly={{ duration: 100 }}
  onclick={(e) => {
    if (e.target === e.currentTarget) oncancel();
  }}
  onkeydown={(e) => {
    if (e.key === "Escape") {
      e.stopPropagation();
      e.preventDefault();
      oncancel();
    }
  }}
  role="presentation"
>
  <form
    class="dialog"
    transition:fly={{ y: -10, duration: 120 }}
    onsubmit={(e) => {
      e.preventDefault();
      submit();
    }}
  >
    <h3>{t("vars.title")}</h3>
    <p class="hint">{t("vars.hint")}</p>
    <div class="vars">
      {#each variables as name, i (name)}
        <div class="dialog-row">
          <label for="var-{i}">{name}</label>
          <input
            id="var-{i}"
            type="text"
            bind:value={values[name]}
            use:autofocus={i === 0}
            autocomplete="off"
            spellcheck="false"
          />
        </div>
      {/each}
    </div>
    <div class="dialog-actions">
      <button type="button" class="ghost raw-btn" onclick={oncopyraw}>
        {t("vars.copyRaw")}
      </button>
      <button type="button" class="ghost" onclick={oncancel}>
        {t("common.cancel")}
      </button>
      <button type="submit" class="primary">{t("vars.copy")}</button>
    </div>
  </form>
</div>

<style>
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
  .hint {
    margin: -6px 0 0;
    font-size: 12px;
    color: var(--muted);
    line-height: 1.5;
  }
  .vars {
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-height: 40vh;
    overflow-y: auto;
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
    /* 变量名用等宽展示：它对应正文里的 {{占位符}} 原文 */
    font-family: ui-monospace, "Cascadia Code", Consolas, monospace;
    text-transform: none;
    letter-spacing: 0.3px;
    word-break: break-all;
  }
  .dialog-row input {
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    color: var(--fg);
    border-radius: 8px;
    padding: 7px 10px;
    font-size: 13px;
    outline: none;
  }
  .dialog-row input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .dialog-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .raw-btn {
    margin-right: auto;
  }
</style>
