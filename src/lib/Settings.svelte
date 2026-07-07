<script lang="ts">
  import { fade, scale } from "svelte/transition";
  import type { CloudConfigView, SyncStatus } from "./types";
  import {
    createTranslator,
    type Language,
    type Translator,
  } from "./i18n";
  import {
    downloadAll,
    getCloudConfig,
    getSyncStatus,
    openUrl,
    saveCloudConfig,
    testCloudConnection,
    uploadAll,
  } from "./api";

  const fallbackT = createTranslator("zh");

  let {
    open = $bindable(false),
    onsynced,
    language = "zh",
    onlanguagechange = (_language: Language) => {},
    t = fallbackT,
  }: {
    open: boolean;
    onsynced: () => void;
    language?: Language;
    onlanguagechange?: (language: Language) => void;
    t?: Translator;
  } = $props();

  let config = $state<CloudConfigView | null>(null);
  let status = $state<SyncStatus | null>(null);

  let username = $state("");
  let password = $state("");
  let remoteRoot = $state("PromptPocket");
  // 密码编辑模式：已配置时默认锁定（显示"已保存"），点"修改"才解锁
  let editingPassword = $state(false);

  let testing = $state(false);
  let saving = $state(false);
  let transferring = $state<"upload" | "download" | null>(null);
  let message = $state<{ type: "ok" | "err"; text: string } | null>(null);

  // 坚果云帮助页：如何获取应用密码
  const HELP_URL = "https://help.jianguoyun.com/?p=2064";

  // 密码是否已保存（用于显示状态）
  let hasPassword = $derived(!!config?.hasPassword);

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
      [config, status] = await Promise.all([getCloudConfig(), getSyncStatus()]);
      username = config.username;
      remoteRoot = config.remoteRoot || "PromptPocket";
      // 密码不回显（安全）；已配置则锁定编辑模式，点"修改"才解锁
      password = "";
      editingPassword = !config.hasPassword;
    } catch (e) {
      message = { type: "err", text: String(e) };
    }
  }

  async function refreshStatus() {
    try {
      status = await getSyncStatus();
    } catch {
      /* 忽略 */
    }
  }

  async function doTest() {
    if (!username.trim() || !password.trim()) {
      message = { type: "err", text: t("settings.fillCredentials") };
      return;
    }
    testing = true;
    message = null;
    try {
      await testCloudConnection(username.trim(), password.trim(), remoteRoot.trim() || "PromptPocket");
      message = { type: "ok", text: t("settings.testOk") };
    } catch (e) {
      message = {
        type: "err",
        text: t("settings.connectionFailed", { error: String(e) }),
      };
    } finally {
      testing = false;
    }
  }

  async function doSave() {
    if (!username.trim()) {
      message = { type: "err", text: t("settings.fillUsername") };
      return;
    }
    const pwd = password.trim();
    // 已配置且未进入密码编辑模式 → 保留旧密码；否则必须填密码
    if (editingPassword && !pwd) {
      message = { type: "err", text: t("settings.fillPassword") };
      return;
    }
    saving = true;
    message = null;
    try {
      // 未编辑密码（已配置）传 __KEEP__ 占位符保留旧密码
      const finalPwd = editingPassword ? pwd : "__KEEP__";
      await saveCloudConfig(
        username.trim(),
        finalPwd,
        remoteRoot.trim() || "PromptPocket",
      );
      message = { type: "ok", text: t("settings.configSaved") };
      await load();
    } catch (e) {
      message = { type: "err", text: String(e) };
    } finally {
      saving = false;
    }
  }

  // 上传到坚果云：本地覆盖云端（只增不删云端）
  async function doUpload() {
    transferring = "upload";
    message = null;
    try {
      const result = await uploadAll();
      message = { type: "ok", text: "↑ " + result };
      await refreshStatus();
      onsynced();
    } catch (e) {
      message = { type: "err", text: String(e) };
    } finally {
      transferring = null;
    }
  }

  // 下载到本地：云端覆盖本地
  async function doDownload() {
    transferring = "download";
    message = null;
    try {
      const result = await downloadAll();
      message = { type: "ok", text: "↓ " + result };
      await refreshStatus();
      onsynced();
    } catch (e) {
      message = { type: "err", text: String(e) };
    } finally {
      transferring = null;
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
        <h2>{t("settings.title")}</h2>
        <button class="close" onclick={close} aria-label={t("common.close")}>×</button>
      </header>

      <div class="modal-body">
        <section class="field">
          <span class="field-label">{t("settings.language")}</span>
          <div class="language-segment" role="group" aria-label={t("settings.language")}>
            <button
              type="button"
              class:active={language === "zh"}
              onclick={() => onlanguagechange("zh")}
            >
              {t("settings.languageZh")}
            </button>
            <button
              type="button"
              class:active={language === "en"}
              onclick={() => onlanguagechange("en")}
            >
              {t("settings.languageEn")}
            </button>
          </div>
          <p class="hint">{t("settings.languageHint")}</p>
        </section>

        <!-- 同步状态 -->
        {#if status}
          <div class="status-box" class:syncing={status.syncing} class:error={status.lastError}>
            {#if status.syncing}
              <span class="dot syncing-dot"></span> {t("settings.statusSyncing")}
            {:else if !status.configured}
              <span class="dot off-dot"></span> {t("settings.statusNotConfigured")}
            {:else if status.lastError}
              <span class="dot err-dot"></span> {t("settings.statusError")}
            {:else if status.lastSync}
              <span class="dot ok-dot"></span> {status.lastSync}
            {:else}
              <span class="dot off-dot"></span> {t("settings.statusWaiting")}
            {/if}
          </div>
          {#if status.lastError}
            <p class="err-detail">{status.lastError}</p>
          {/if}
        {/if}

        <!-- 配置表单 -->
        <section class="field">
          <span class="field-label">{t("settings.account")}</span>
          <input
            class="form-input"
            type="text"
            bind:value={username}
            placeholder={t("settings.accountPlaceholder")}
            spellcheck="false"
          />
        </section>

        <section class="field">
          <span class="field-label">
            {t("settings.appPassword")}
            <button class="help-link" onclick={() => void openUrl(HELP_URL)}>
              {t("settings.help")}
            </button>
          </span>
          {#if hasPassword && !editingPassword}
            <!-- 已保存：显示状态 + 修改按钮（明确告知密码已持久化）-->
            <div class="pwd-saved">
              <span class="pwd-saved-text">{t("settings.passwordSaved")}</span>
              <button
                class="pwd-edit-btn"
                onclick={() => {
                  editingPassword = true;
                  password = "";
                }}
              >
                {t("settings.editPassword")}
              </button>
            </div>
          {:else}
            <!-- 未配置或编辑模式：输入框 -->
            <input
              class="form-input"
              type="password"
              bind:value={password}
              placeholder={t("settings.passwordPlaceholder")}
              spellcheck="false"
              autocomplete="off"
            />
          {/if}
          <p class="hint">
            {t("settings.passwordHintBefore")}
            <button class="inline-link" onclick={() => void openUrl(HELP_URL)}>
              {t("settings.passwordHintLink")}
            </button>
            {t("settings.passwordHintAfter")}
          </p>
        </section>

        <section class="field">
          <span class="field-label">{t("settings.remoteRoot")}</span>
          <input
            class="form-input"
            type="text"
            bind:value={remoteRoot}
            placeholder="PromptPocket"
            spellcheck="false"
          />
          <p class="hint">{t("settings.remoteRootHint")}</p>
        </section>

        <!-- 手动同步操作区 -->
        {#if status?.configured}
          <section class="sync-actions">
            <span class="field-label">{t("settings.manualSync")}</span>
            <div class="sync-btns">
              <button
                class="sync-btn upload"
                onclick={doUpload}
                disabled={transferring !== null}
              >
                {#if transferring === "upload"}{t("settings.uploading")}{:else}{t("settings.upload")}{/if}
              </button>
              <button
                class="sync-btn download"
                onclick={doDownload}
                disabled={transferring !== null}
              >
                {#if transferring === "download"}{t("settings.downloading")}{:else}{t("settings.download")}{/if}
              </button>
            </div>
            <p class="hint">{t("settings.syncHint")}</p>
          </section>
        {/if}

        {#if message}
          <div class="msg" class:ok={message.type === "ok"} class:err={message.type === "err"}>
            {message.text}
          </div>
        {/if}
      </div>

      <footer class="modal-foot">
        <div class="spacer"></div>
        <button class="ghost" onclick={close}>{t("common.close")}</button>
        <button class="ghost" onclick={doTest} disabled={testing || saving || transferring !== null}>
          {testing ? t("settings.testing") : t("settings.testConnection")}
        </button>
        <button class="primary" onclick={doSave} disabled={saving || testing}>
          {saving ? t("settings.saving") : t("settings.saveConfig")}
        </button>
      </footer>
    </div>
  </div>
{/if}

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

  .modal {
    width: 500px;
    max-width: 92vw;
    max-height: 90vh;
    overflow-y: auto;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 12px;
    box-shadow: var(--shadow-soft);
    display: flex;
    flex-direction: column;
  }

  .modal-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 18px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
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
    border-radius: 8px;
  }
  .close:hover {
    color: var(--fg);
    background: var(--bg-hover);
  }

  .modal-body {
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .status-box {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-radius: 8px;
    font-size: 13px;
    background: var(--bg-elevated);
  }
  .status-box.syncing {
    background: var(--accent-soft);
  }
  .status-box.error {
    background: rgba(217, 48, 37, 0.08);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .ok-dot {
    background: #22a06b;
  }
  .syncing-dot {
    background: var(--accent);
    animation: pulse 1s infinite;
  }
  .err-dot {
    background: var(--danger);
  }
  .off-dot {
    background: var(--muted);
  }
  @keyframes pulse {
    50% {
      opacity: 0.4;
    }
  }
  .err-detail {
    margin: -8px 0 0;
    padding: 0 12px;
    font-size: 12px;
    color: var(--danger);
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .field-label {
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .language-segment {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px;
    padding: 3px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
  }
  .language-segment button {
    height: 28px;
    border: 1px solid transparent;
    border-radius: 6px;
    background: transparent;
    color: var(--muted);
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
  }
  .language-segment button:hover {
    color: var(--fg);
    background: var(--bg-hover);
  }
  .language-segment button.active {
    border-color: var(--border);
    background: var(--bg-elevated);
    color: var(--accent);
    box-shadow: 0 1px 2px rgba(31, 42, 68, 0.06);
  }
  .help-link {
    background: transparent;
    border: none;
    color: var(--accent);
    font-size: 11px;
    font-weight: 400;
    cursor: pointer;
    text-decoration: underline;
    padding: 0;
  }
  .form-input {
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    color: var(--fg);
    border-radius: 8px;
    padding: 7px 10px;
    font-size: 13px;
    outline: none;
    width: 100%;
    box-sizing: border-box;
  }
  .form-input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .hint {
    font-size: 11.5px;
    color: var(--muted);
    line-height: 1.5;
    margin: 0;
  }
  .inline-link {
    background: transparent;
    border: none;
    color: var(--accent);
    font-size: 11.5px;
    cursor: pointer;
    text-decoration: underline;
    padding: 0;
    font-family: inherit;
  }
  /* 手动同步操作区 */
  .sync-actions {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .sync-btns {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }
  /* 密码已保存状态 */
  .pwd-saved {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 8px 12px;
    background: rgba(34, 160, 107, 0.1);
    border: 1px solid rgba(34, 160, 107, 0.3);
    border-radius: 8px;
  }
  .pwd-saved-text {
    font-size: 13px;
    color: #1a7a52;
  }
  .pwd-edit-btn {
    background: transparent;
    border: 1px solid var(--border-strong);
    color: var(--fg);
    font-size: 12px;
    padding: 3px 10px;
    border-radius: 7px;
    cursor: pointer;
  }
  .pwd-edit-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }

  .sync-btn {
    padding: 9px 12px;
    border-radius: 8px;
    border: 1px solid var(--border-strong);
    background: var(--bg-elevated);
    color: var(--fg);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.12s;
  }
  .sync-btn:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
  }
  .sync-btn.upload:hover:not(:disabled) {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }
  .sync-btn.download:hover:not(:disabled) {
    background: #22a06b;
    color: #fff;
    border-color: #22a06b;
  }
  .sync-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .msg {
    padding: 8px 12px;
    border-radius: 8px;
    font-size: 12.5px;
  }
  .msg.ok {
    background: rgba(34, 160, 107, 0.1);
    color: #1a7a52;
  }
  .msg.err {
    background: rgba(217, 48, 37, 0.1);
    color: var(--danger);
  }

  .modal-foot {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 18px;
    border-top: 1px solid var(--border);
    background: var(--bg-elevated);
    flex-shrink: 0;
  }
  .spacer {
    flex: 1;
  }
  button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
