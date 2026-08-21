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
    getAutostart,
    getCloudConfig,
    getSyncStatus,
    openUrl,
    saveCloudConfig,
    saveGithubConfig,
    setAutostart,
    setSyncProvider,
    testCloudConnection,
    testGithubConnection,
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

  // 同步后端 + GitHub 表单（provider 切换立即持久化，两侧配置互不影响）
  let provider = $state<"webdav" | "github">("webdav");
  let providerBusy = $state(false);
  let ghRepo = $state("");
  let ghToken = $state("");
  let ghBranch = $state("");
  let ghPrefix = $state("");
  let editingToken = $state(false);

  let testing = $state(false);
  let saving = $state(false);
  let transferring = $state<"upload" | "download" | null>(null);
  let message = $state<{ type: "ok" | "err"; text: string } | null>(null);

  // 开机自启动：状态读系统真实值，切换失败时回滚开关
  let autostart = $state(false);
  let autostartBusy = $state(false);

  // 坚果云帮助页：如何获取应用密码
  const HELP_URL = "https://help.jianguoyun.com/?p=2064";
  // GitHub fine-grained PAT 创建页
  const GH_HELP_URL = "https://github.com/settings/personal-access-tokens";

  // 密码是否已保存（用于显示状态）
  let hasPassword = $derived(!!config?.hasPassword);
  let hasToken = $derived(!!config?.hasToken);
  // 上传按钮目标跟随当前后端
  let uploadLabel = $derived(
    t("settings.upload", {
      target: provider === "github" ? "GitHub" : t("settings.providerWebdav"),
    }),
  );

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
      // GitHub 表单回显；token 同样不回显
      provider = config.provider === "github" ? "github" : "webdav";
      ghRepo = config.ghRepo ?? "";
      ghBranch = config.ghBranch ?? "";
      ghPrefix = config.ghPrefix ?? "";
      ghToken = "";
      editingToken = !config.hasToken;
      // 自启动状态读系统真实值；读取失败不阻断设置页其余内容
      autostart = await getAutostart().catch(() => false);
    } catch (e) {
      message = { type: "err", text: String(e) };
    }
  }

  // 切换后端：立即持久化（set_sync_provider 只改激活标记，不动两侧配置），失败回滚
  async function switchProvider(next: "webdav" | "github") {
    if (provider === next || providerBusy) return;
    providerBusy = true;
    const prev = provider;
    provider = next; // 乐观更新，失败回滚
    message = null;
    try {
      await setSyncProvider(next);
      await refreshStatus();
    } catch (e) {
      provider = prev;
      message = { type: "err", text: String(e) };
    } finally {
      providerBusy = false;
    }
  }

  async function toggleAutostart() {
    if (autostartBusy) return;
    autostartBusy = true;
    const next = !autostart;
    autostart = next; // 乐观更新，失败回滚
    try {
      await setAutostart(next);
    } catch (e) {
      autostart = !next;
      message = { type: "err", text: t("settings.autostartFailed", { error: String(e) }) };
    } finally {
      autostartBusy = false;
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
    if (provider === "github") return doTestGithub();
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

  async function doTestGithub() {
    if (!ghRepo.trim()) {
      message = { type: "err", text: t("settings.fillGhRepo") };
      return;
    }
    if (!ghToken.trim()) {
      message = { type: "err", text: t("settings.fillGhToken") };
      return;
    }
    testing = true;
    message = null;
    try {
      await testGithubConnection(ghRepo.trim(), ghToken.trim(), ghBranch.trim(), ghPrefix.trim());
      message = { type: "ok", text: t("settings.ghTestOk") };
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
    if (provider === "github") return doSaveGithub();
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
    // __KEEP__ 是"保留旧密码"的占位符：真实密码恰好等于它会被静默忽略，拒绝之
    if (editingPassword && pwd === "__KEEP__") {
      message = { type: "err", text: t("settings.passwordKeepReserved") };
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

  async function doSaveGithub() {
    if (!ghRepo.trim()) {
      message = { type: "err", text: t("settings.fillGhRepo") };
      return;
    }
    const tok = ghToken.trim();
    // 与坚果云密码同规则：已配置未点"修改"→ __KEEP__ 保留旧 token
    if (editingToken && !tok) {
      message = { type: "err", text: t("settings.fillGhToken") };
      return;
    }
    if (editingToken && tok === "__KEEP__") {
      message = { type: "err", text: t("settings.ghTokenKeepReserved") };
      return;
    }
    saving = true;
    message = null;
    try {
      await saveGithubConfig(
        ghRepo.trim(),
        editingToken ? tok : "__KEEP__",
        ghBranch.trim(),
        ghPrefix.trim(),
      );
      message = { type: "ok", text: t("settings.configSaved") };
      await load();
    } catch (e) {
      message = { type: "err", text: String(e) };
    } finally {
      saving = false;
    }
  }

  // 全量上传到当前后端（坚果云 / GitHub，按 provider 分派）
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

  // 从当前后端全量下载并覆盖本地（覆盖前备份 .trash）
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
    onkeydown={(e) => {
      if (e.key === "Escape") {
        // 阻止冒泡到 <svelte:window>：关弹窗不应连带隐藏整个窗口
        e.stopPropagation();
        e.preventDefault();
        close();
      }
    }}
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
          <div class="segment" role="group" aria-label={t("settings.language")}>
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
          <span class="field-label">{t("settings.autostart")}</span>
          <button
            type="button"
            class="switch"
            class:on={autostart}
            role="switch"
            aria-checked={autostart}
            aria-label={t("settings.autostart")}
            disabled={autostartBusy}
            onclick={toggleAutostart}
          >
            <span class="switch-dot"></span>
          </button>
          <p class="hint">{t("settings.autostartHint")}</p>
        </section>

        <!-- 同步后端切换：立即持久化，两侧配置互不影响 -->
        <section class="field">
          <span class="field-label">{t("settings.provider")}</span>
          <div class="segment" role="group" aria-label={t("settings.provider")}>
            <button
              type="button"
              class:active={provider === "webdav"}
              disabled={providerBusy}
              onclick={() => void switchProvider("webdav")}
            >
              {t("settings.providerWebdav")}
            </button>
            <button
              type="button"
              class:active={provider === "github"}
              disabled={providerBusy}
              onclick={() => void switchProvider("github")}
            >
              {t("settings.providerGithub")}
            </button>
          </div>
        </section>

        <!-- 同步状态（跟随当前激活的后端） -->
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

        <!-- 配置表单：按当前后端切换 -->
        {#if provider === "github"}
          <section class="field">
            <span class="field-label">{t("settings.ghRepo")}</span>
            <input
              class="form-input"
              type="text"
              bind:value={ghRepo}
              placeholder={t("settings.ghRepoPlaceholder")}
              spellcheck="false"
            />
            <p class="hint">{t("settings.ghRepoHint")}</p>
          </section>

          <section class="field">
            <span class="field-label">
              {t("settings.ghToken")}
              <button class="help-link" onclick={() => void openUrl(GH_HELP_URL)}>
                {t("settings.help")}
              </button>
            </span>
            {#if hasToken && !editingToken}
              <!-- 已保存：显示状态 + 修改按钮 -->
              <div class="pwd-saved">
                <span class="pwd-saved-text">{t("settings.passwordSaved")}</span>
                <button
                  class="pwd-edit-btn"
                  onclick={() => {
                    editingToken = true;
                    ghToken = "";
                  }}
                >
                  {t("settings.editPassword")}
                </button>
              </div>
            {:else}
              <input
                class="form-input"
                type="password"
                bind:value={ghToken}
                placeholder={t("settings.ghTokenPlaceholder")}
                spellcheck="false"
                autocomplete="off"
              />
            {/if}
            <p class="hint">
              {t("settings.ghTokenHintBefore")}
              <button class="inline-link" onclick={() => void openUrl(GH_HELP_URL)}>
                {t("settings.ghTokenHintLink")}
              </button>
              {t("settings.ghTokenHintAfter")}
            </p>
          </section>

          <div class="field-row">
            <section class="field">
              <span class="field-label">{t("settings.ghBranch")}</span>
              <input
                class="form-input"
                type="text"
                bind:value={ghBranch}
                placeholder="main"
                spellcheck="false"
              />
              <p class="hint">{t("settings.ghBranchHint")}</p>
            </section>

            <section class="field">
              <span class="field-label">{t("settings.ghPrefix")}</span>
              <input
                class="form-input"
                type="text"
                bind:value={ghPrefix}
                placeholder="archive"
                spellcheck="false"
              />
              <p class="hint">{t("settings.ghPrefixHint")}</p>
            </section>
          </div>
        {:else}
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
        {/if}

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
                {#if transferring === "upload"}{t("settings.uploading")}{:else}{uploadLabel}{/if}
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
        <button class="primary" onclick={doSave} disabled={saving || testing || transferring !== null}>
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
  .segment {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 4px;
    padding: 3px;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--bg);
  }
  .segment button {
    height: 28px;
    border: 1px solid transparent;
    border-radius: 6px;
    background: transparent;
    color: var(--muted);
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
  }
  .segment button:hover:not(:disabled) {
    color: var(--fg);
    background: var(--bg-hover);
  }
  .segment button.active {
    border-color: var(--border);
    background: var(--bg-elevated);
    color: var(--accent);
    box-shadow: 0 1px 2px rgba(31, 42, 68, 0.06);
  }
  /* 双列表单行（分支 / 路径前缀） */
  .field-row {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  /* 自启动开关：pill 滑块，状态即系统真实状态 */
  .switch {
    width: 38px;
    height: 22px;
    border-radius: 11px;
    border: 1px solid var(--border);
    background: var(--bg);
    padding: 2px;
    cursor: pointer;
    display: inline-flex;
    align-items: center;
    transition: background 0.15s, border-color 0.15s;
    align-self: flex-start;
  }
  .switch:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .switch-dot {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--muted);
    transition: transform 0.15s, background 0.15s;
  }
  .switch.on {
    background: var(--accent);
    border-color: var(--accent);
  }
  .switch.on .switch-dot {
    transform: translateX(16px);
    background: #fff;
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
