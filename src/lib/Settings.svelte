<script lang="ts">
  import { fade, scale } from "svelte/transition";
  import type { CloudConfigView, RecoveryEntry, SyncStatus } from "./types";
  import { dialogFocus } from "./dialog";
  import {
    createTranslator,
    type Language,
    type Translator,
  } from "./i18n";
  import {
    getHotkey, setHotkey, listRecovery, restoreRecovery, readRecovery,
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
    onrestored = (_path: string) => {},
    t = fallbackT,
  }: {
    open: boolean;
    onsynced: () => void;
    onrestored?: (path: string) => void;
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
  let hotkey = $state("");
  let savingHotkey = $state(false);
  let recoveryEntries = $state<RecoveryEntry[]>([]);
  let recoveryLoading = $state(false);
  let recoveryPreview = $state<{ entry: RecoveryEntry; text: string } | null>(null);
  let recoveryBusyId = $state<string | null>(null);

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
    recoveryLoading = true;
    recoveryPreview = null;
    try {
      const [loadedConfig, loadedStatus, loadedHotkey, loadedRecovery] = await Promise.allSettled([
        getCloudConfig(),
        getSyncStatus(),
        getHotkey(),
        listRecovery(),
      ]);
      if (loadedConfig.status === "fulfilled") {
        config = loadedConfig.value;
        username = config.username;
        remoteRoot = config.remoteRoot || "PromptPocket";
        password = "";
        editingPassword = !config.hasPassword;
      }
      if (loadedStatus.status === "fulfilled") status = loadedStatus.value;
      if (loadedHotkey.status === "fulfilled") hotkey = loadedHotkey.value;
      if (loadedRecovery.status === "fulfilled") recoveryEntries = loadedRecovery.value.slice(0, 50);
      const errors = [loadedConfig, loadedStatus, loadedHotkey, loadedRecovery]
        .filter((result) => result.status === "rejected")
        .map((result) => String(result.reason));
      if (errors.length) message = { type: "err", text: errors.join("\n") };
    } catch (e) {
      message = { type: "err", text: String(e) };
    } finally {
      recoveryLoading = false;
    }
  }

  async function refreshStatus(): Promise<SyncStatus | null> {
    try {
      status = await getSyncStatus();
      return status;
    } catch {
      /* 忽略 */
      return null;
    }
  }

  async function refreshRecovery() {
    recoveryLoading = true;
    try {
      recoveryEntries = (await listRecovery()).slice(0, 50);
    } catch (e) {
      message = { type: "err", text: String(e) };
    } finally {
      recoveryLoading = false;
    }
  }

  async function doTest() {
    const unchangedUser = username.trim() === (config?.username ?? "").trim();
    const canKeepPassword = hasPassword && !editingPassword && unchangedUser;
    const pwd = canKeepPassword ? "__KEEP__" : password.trim();
    if (!username.trim() || !pwd) {
      message = { type: "err", text: t("settings.fillCredentials") };
      return;
    }
    testing = true;
    message = null;
    try {
      await testCloudConnection(username.trim(), pwd, remoteRoot.trim() || "PromptPocket");
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

  // 定向上传，冲突由后端保留并报告。
  async function doUpload() {
    transferring = "upload";
    message = null;
    try {
      const result = await uploadAll();
      const current = await refreshStatus();
      message = current?.lastError
        ? { type: "err", text: current.lastError }
        : { type: "ok", text: "↑ " + result };
      await refreshRecovery();
      onsynced();
    } catch (e) {
      message = { type: "err", text: String(e) };
    } finally {
      transferring = null;
    }
  }

  // 定向下载，覆盖前备份，本地改动保留。
  async function doDownload() {
    if (!confirm(t("settings.transferConfirm"))) return;
    transferring = "download";
    message = null;
    try {
      const result = await downloadAll();
      const current = await refreshStatus();
      message = current?.lastError
        ? { type: "err", text: current.lastError }
        : { type: "ok", text: "↓ " + result };
      await refreshRecovery();
      onsynced();
    } catch (e) {
      message = { type: "err", text: String(e) };
    } finally {
      transferring = null;
    }
  }

  async function doSaveHotkey() {
    if (!hotkey.trim()) return;
    savingHotkey = true;
    message = null;
    try {
      await setHotkey(hotkey.trim());
      hotkey = await getHotkey();
      message = { type: "ok", text: t("settings.hotkeySaved") };
    } catch (e) {
      message = { type: "err", text: String(e) };
    } finally {
      savingHotkey = false;
    }
  }

  async function viewRecovery(entry: RecoveryEntry) {
    if (recoveryBusyId !== null) return;
    recoveryBusyId = entry.id;
    message = null;
    try {
      recoveryPreview = { entry, text: await readRecovery(entry.id) };
    } catch (e) {
      message = { type: "err", text: String(e) };
    } finally {
      recoveryBusyId = null;
    }
  }

  async function doRestore(entry: RecoveryEntry) {
    if (recoveryBusyId !== null) return;
    recoveryBusyId = entry.id;
    message = null;
    try {
      const path = await restoreRecovery(entry.id);
      message = { type: "ok", text: t("settings.restored", { path }) };
      recoveryPreview = null;
      await refreshRecovery();
      onrestored(path);
    } catch (e) {
      message = { type: "err", text: String(e) };
    } finally {
      recoveryBusyId = null;
    }
  }

  function recoveryKindLabel(kind: string): string {
    if (kind === "deleted") return t("settings.recoveryDeleted");
    if (kind === "sync") return t("settings.recoverySync");
    return t("settings.recoveryHistory");
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
    <div
      class="modal"
      transition:scale={{ duration: 150, start: 0.96 }}
      role="dialog"
      aria-modal="true"
      aria-labelledby="settings-title"
      tabindex="-1"
      use:dialogFocus
    >
      <header class="modal-head">
        <h2 id="settings-title">{t("settings.title")}</h2>
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

        <section class="field">
          <label class="field-label" for="settings-hotkey">{t("settings.hotkey")}</label>
          <div class="inline-controls">
            <input
              class="form-input"
              id="settings-hotkey"
              type="text"
              bind:value={hotkey}
              placeholder="Ctrl+Alt+P"
              spellcheck="false"
            />
            <button class="ghost" onclick={doSaveHotkey} disabled={savingHotkey || !hotkey.trim()}>
              {savingHotkey ? t("settings.saving") : t("settings.saveHotkey")}
            </button>
          </div>
          <p class="hint">{t("settings.hotkeyHint")}</p>
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
          <label class="field-label" for="settings-account">{t("settings.account")}</label>
          <input
            class="form-input"
            id="settings-account"
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
              aria-label={t("settings.appPassword")}
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
          <label class="field-label" for="settings-remote-root">{t("settings.remoteRoot")}</label>
          <input
            class="form-input"
            id="settings-remote-root"
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

        <section class="field recovery-section">
          <span class="field-label">{t("settings.recovery")}</span>
          <p class="hint">{t("settings.recoveryHint")}</p>
          <div class="recovery-list">
            {#if recoveryLoading}
              <div class="recovery-empty">{t("editor.loading")}</div>
            {:else if recoveryEntries.length === 0}
              <div class="recovery-empty">{t("settings.recoveryEmpty")}</div>
            {:else}
              {#each recoveryEntries as entry (entry.id)}
                <article class="recovery-item">
                  <div class="recovery-main">
                    <strong>{entry.originalPath}</strong>
                    <span>{recoveryKindLabel(entry.kind)} · {entry.createdAt}</span>
                  </div>
                  <div class="recovery-actions">
                    <button
                      class="ghost small"
                      onclick={() => viewRecovery(entry)}
                      disabled={recoveryBusyId !== null}
                    >
                      {t("settings.viewRecovery")}
                    </button>
                    <button
                      class="ghost small"
                      onclick={() => doRestore(entry)}
                      disabled={recoveryBusyId !== null}
                    >
                      {t("settings.restore")}
                    </button>
                  </div>
                </article>
              {/each}
            {/if}
          </div>
          {#if recoveryPreview}
            <div class="recovery-preview">
              <div class="recovery-preview-head">
                <strong>{recoveryPreview.entry.originalPath}</strong>
                <button class="close-preview" onclick={() => (recoveryPreview = null)} aria-label={t("common.close")}>×</button>
              </div>
              <pre>{recoveryPreview.text}</pre>
            </div>
          {/if}
        </section>

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
    width: 560px;
    max-width: 92vw;
    max-height: 90vh;
    overflow: hidden;
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
    overflow-y: auto;
    min-height: 0;
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
  .inline-controls {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 8px;
    align-items: center;
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
  .recovery-section {
    min-height: 0;
  }
  .recovery-list {
    max-height: 220px;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
  }
  .recovery-empty {
    padding: 18px 12px;
    color: var(--muted);
    font-size: 12.5px;
    text-align: center;
  }
  .recovery-item {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 10px;
    align-items: center;
    padding: 9px 10px;
    border-bottom: 1px solid var(--border);
  }
  .recovery-item:last-child {
    border-bottom: 0;
  }
  .recovery-main {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .recovery-main strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12.5px;
    font-weight: 600;
  }
  .recovery-main span {
    color: var(--muted);
    font-size: 11px;
  }
  .recovery-actions {
    display: flex;
    gap: 6px;
  }
  .small {
    padding: 4px 8px;
    font-size: 11.5px;
  }
  .recovery-preview {
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg-elevated);
    overflow: hidden;
  }
  .recovery-preview-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 7px 10px;
    border-bottom: 1px solid var(--border);
    font-size: 12px;
  }
  .recovery-preview-head strong {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .close-preview {
    border: 0;
    background: transparent;
    color: var(--muted);
    cursor: pointer;
    font-size: 18px;
    line-height: 1;
  }
  .recovery-preview pre {
    max-height: 180px;
    overflow: auto;
    margin: 0;
    padding: 10px;
    font-family: var(--font-mono);
    font-size: 12px;
    line-height: 1.5;
    white-space: pre-wrap;
    word-break: break-word;
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
