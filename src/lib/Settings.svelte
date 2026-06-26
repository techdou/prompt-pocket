<script lang="ts">
  import { fade, scale } from "svelte/transition";
  import type { AppConfig } from "./types";
  import { getConfig, openDataDir, pickDataDir, setDataDir } from "./api";

  let {
    open = $bindable(false),
    onchanged,
  }: {
    open: boolean;
    onchanged: (config: AppConfig) => void;
  } = $props();

  let config = $state<AppConfig | null>(null);
  let customPath = $state("");
  let saving = $state(false);
  let message = $state<{ type: "ok" | "err"; text: string } | null>(null);

  // 打开时加载当前配置
  let lastOpen = false;
  $effect(() => {
    if (open && !lastOpen) {
      lastOpen = open;
      void load();
    }
    if (!open) lastOpen = false;
  });

  async function load() {
    try {
      config = await getConfig();
      customPath = config.data_dir;
    } catch (e) {
      message = { type: "err", text: String(e) };
    }
  }

  async function doPick() {
    saving = true;
    message = null;
    try {
      const picked = await pickDataDir();
      if (picked) {
        customPath = picked;
      }
    } catch (e) {
      message = { type: "err", text: "无法打开选择器: " + String(e) };
    } finally {
      saving = false;
    }
  }

  async function doApply() {
    if (!customPath.trim()) {
      message = { type: "err", text: "请先选择一个目录" };
      return;
    }
    saving = true;
    message = null;
    try {
      const updated = await setDataDir(customPath.trim());
      config = updated;
      message = { type: "ok", text: "已切换数据目录并刷新列表" };
      onchanged(updated);
    } catch (e) {
      message = { type: "err", text: String(e) };
    } finally {
      saving = false;
    }
  }

  function close() {
    open = false;
    message = null;
  }

  function onBackdrop(e: MouseEvent) {
    if (e.target === e.currentTarget) close();
  }
</script>

{#if open}
  <div
    class="backdrop"
    transition:fade={{ duration: 120 }}
    onclick={onBackdrop}
    onkeydown={(e) => e.key === "Escape" && close()}
    role="presentation"
  >
    <div class="modal" transition:scale={{ duration: 150, start: 0.96 }}>
      <header class="modal-head">
        <h2>设置</h2>
        <button class="close" onclick={close} aria-label="关闭">×</button>
      </header>

      <div class="modal-body">
        <section class="field">
          <span class="field-label">提示词存储目录</span>
          <p class="hint">
            提示词会以 Markdown 文件存到此目录。把它指向你的
            OneDrive / iCloud / 坚果云同步目录，即可实现多设备云同步。
          </p>
          <div class="path-row">
            <input
              type="text"
              bind:value={customPath}
              placeholder="例如 D:\\OneDrive\\PromptPocket"
              spellcheck="false"
            />
            <button class="ghost" onclick={doPick} disabled={saving}>
              选择…
            </button>
            <button
              class="ghost"
              onclick={() => void openDataDir()}
              title="在文件管理器中打开"
            >
              打开
            </button>
          </div>
        </section>

        <section class="field">
          <span class="field-label">全局快捷键</span>
          <div class="hotkey-display">
            <kbd>Ctrl</kbd> + <kbd>Alt</kbd> + <kbd>P</kbd>
            <span class="hint-inline">唤出 / 隐藏窗口</span>
          </div>
        </section>

        {#if message}
          <div class="msg" class:ok={message.type === "ok"} class:err={message.type === "err"}>
            {message.text}
          </div>
        {/if}
      </div>

      <footer class="modal-foot">
        <button class="ghost" onclick={close}>取消</button>
        <button class="primary" onclick={doApply} disabled={saving}>
          {saving ? "应用中…" : "应用并刷新"}
        </button>
      </footer>
    </div>
  </div>
{/if}

<style>
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

  .modal {
    width: 480px;
    max-width: 90vw;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.25);
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  .modal-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 18px;
    border-bottom: 1px solid var(--border);
  }
  .modal-head h2 {
    margin: 0;
    font-size: 15px;
    font-weight: 600;
  }
  .close {
    background: transparent;
    border: none;
    font-size: 22px;
    line-height: 1;
    color: var(--muted);
    cursor: pointer;
    padding: 0 4px;
    border-radius: 4px;
  }
  .close:hover {
    color: var(--fg);
    background: var(--bg-hover);
  }

  .modal-body {
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 20px;
  }

  .field-label {
    display: block;
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    margin-bottom: 6px;
  }
  .hint {
    font-size: 12px;
    color: var(--muted);
    line-height: 1.5;
    margin: 0 0 10px;
  }

  .path-row {
    display: flex;
    gap: 6px;
  }
  .path-row input {
    flex: 1;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    color: var(--fg);
    border-radius: 6px;
    padding: 7px 10px;
    font-size: 12.5px;
    font-family: ui-monospace, "Cascadia Code", Consolas, monospace;
    outline: none;
    min-width: 0;
  }
  .path-row input:focus {
    border-color: var(--fg);
  }

  .hotkey-display {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
  }
  kbd {
    display: inline-block;
    padding: 2px 8px;
    font-size: 12px;
    font-family: ui-monospace, monospace;
    background: var(--bg-elevated);
    border: 1px solid var(--border-strong);
    border-bottom-width: 2px;
    border-radius: 5px;
    line-height: 1.5;
  }
  .hint-inline {
    color: var(--muted);
    font-size: 12px;
    margin-left: 6px;
  }

  .msg {
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 12.5px;
  }
  .msg.ok {
    background: rgba(34, 139, 34, 0.1);
    color: #227022;
  }
  .msg.err {
    background: rgba(217, 48, 37, 0.1);
    color: var(--danger);
  }

  .modal-foot {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    padding: 12px 18px;
    border-top: 1px solid var(--border);
    background: var(--bg-elevated);
  }

  button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
