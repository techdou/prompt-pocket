<script lang="ts">
  import { fade, scale } from "svelte/transition";
  import type { CloudConfigView, SyncStatus } from "./types";
  import {
    downloadAll,
    getCloudConfig,
    getSyncStatus,
    openUrl,
    saveCloudConfig,
    testCloudConnection,
    uploadAll,
  } from "./api";

  let {
    open = $bindable(false),
    onsynced,
  }: {
    open: boolean;
    onsynced: () => void;
  } = $props();

  let config = $state<CloudConfigView | null>(null);
  let status = $state<SyncStatus | null>(null);

  let username = $state("");
  let password = $state("");
  let remoteRoot = $state("PromptPocket");

  let testing = $state(false);
  let saving = $state(false);
  let transferring = $state<"upload" | "download" | null>(null);
  let message = $state<{ type: "ok" | "err"; text: string } | null>(null);

  // 坚果云帮助页：如何获取应用密码
  const HELP_URL = "https://help.jianguoyun.com/?p=2064";

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
      // 密码不回显，仅清空让用户在需要时重填
      password = "";
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
      message = { type: "err", text: "请填写账号和应用密码" };
      return;
    }
    testing = true;
    message = null;
    try {
      await testCloudConnection(username.trim(), password.trim(), remoteRoot.trim() || "PromptPocket");
      message = { type: "ok", text: "✓ 连接成功！账号和应用密码有效" };
    } catch (e) {
      message = { type: "err", text: "连接失败：" + String(e) };
    } finally {
      testing = false;
    }
  }

  async function doSave() {
    // 若用户没填密码但已有密码，保留旧密码——需要密码字段
    if (!username.trim()) {
      message = { type: "err", text: "请填写坚果云账号" };
      return;
    }
    // 已配置过且没填新密码：用空字符串占位，后端判断
    const pwd = password.trim();
    if (!pwd && config && !config.hasPassword) {
      message = { type: "err", text: "请填写应用密码" };
      return;
    }
    saving = true;
    message = null;
    try {
      // 若没填新密码，传空字符串，后端识别"空密码"为保留旧密码
      // 若没填新密码，传 __KEEP__ 占位符保留旧密码
      const finalPwd = pwd || (config?.hasPassword ? "__KEEP__" : "");
      await saveCloudConfig(
        username.trim(),
        finalPwd,
        remoteRoot.trim() || "PromptPocket",
      );
      message = { type: "ok", text: "✓ 配置已保存" };
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
        <h2>坚果云同步设置</h2>
        <button class="close" onclick={close} aria-label="关闭">×</button>
      </header>

      <div class="modal-body">
        <!-- 同步状态 -->
        {#if status}
          <div class="status-box" class:syncing={status.syncing} class:error={status.lastError}>
            {#if status.syncing}
              <span class="dot syncing-dot"></span> 正在同步…
            {:else if !status.configured}
              <span class="dot off-dot"></span> 未配置
            {:else if status.lastError}
              <span class="dot err-dot"></span> 同步出错
            {:else if status.lastSync}
              <span class="dot ok-dot"></span> {status.lastSync}
            {:else}
              <span class="dot off-dot"></span> 已配置，等待同步
            {/if}
          </div>
          {#if status.lastError}
            <p class="err-detail">{status.lastError}</p>
          {/if}
        {/if}

        <!-- 配置表单 -->
        <section class="field">
          <span class="field-label">坚果云账号</span>
          <input
            class="form-input"
            type="text"
            bind:value={username}
            placeholder="你的坚果云登录邮箱 / 手机号"
            spellcheck="false"
          />
        </section>

        <section class="field">
          <span class="field-label">
            应用密码
            <button class="help-link" onclick={() => void openUrl(HELP_URL)}>
              如何获取？
            </button>
          </span>
          <input
            class="form-input"
            type="password"
            bind:value={password}
            placeholder={config?.hasPassword ? "已设置（留空则不改）" : "在坚果云官网生成的应用密码"}
            spellcheck="false"
            autocomplete="off"
          />
          <p class="hint">
            不是登录密码。需在
            <button class="inline-link" onclick={() => void openUrl(HELP_URL)}>
              坚果云官网 → 账户信息 → 安全选项 → 第三方应用管理
            </button>
            中添加应用生成。
          </p>
        </section>

        <section class="field">
          <span class="field-label">远程存储路径</span>
          <input
            class="form-input"
            type="text"
            bind:value={remoteRoot}
            placeholder="PromptPocket"
            spellcheck="false"
          />
          <p class="hint">提示词会存在坚果云的这个文件夹下。</p>
        </section>

        <!-- 手动同步操作区 -->
        {#if status?.configured}
          <section class="sync-actions">
            <span class="field-label">手动同步</span>
            <div class="sync-btns">
              <button
                class="sync-btn upload"
                onclick={doUpload}
                disabled={transferring !== null}
              >
                {#if transferring === "upload"}上传中…{:else}↑ 上传到坚果云{/if}
              </button>
              <button
                class="sync-btn download"
                onclick={doDownload}
                disabled={transferring !== null}
              >
                {#if transferring === "download"}下载中…{:else}↓ 下载到本地{/if}
              </button>
            </div>
            <p class="hint">
              上传：本地文件推送到云端（不删除云端已有）。下载：云端覆盖本地（含清理）。
            </p>
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
        <button class="ghost" onclick={close}>关闭</button>
        <button class="ghost" onclick={doTest} disabled={testing || saving || transferring !== null}>
          {testing ? "测试中…" : "测试连接"}
        </button>
        <button class="primary" onclick={doSave} disabled={saving || testing}>
          {saving ? "保存中…" : "保存配置"}
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
    width: 500px;
    max-width: 92vw;
    max-height: 90vh;
    overflow-y: auto;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.25);
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
    gap: 16px;
  }

  .status-box {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-radius: 6px;
    font-size: 13px;
    background: var(--bg-elevated);
  }
  .status-box.syncing {
    background: rgba(74, 124, 247, 0.1);
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
    background: #4a7cf7;
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
  .row-field {
    flex-direction: row;
    align-items: center;
  }
  .field-label {
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
    display: flex;
    align-items: center;
    justify-content: space-between;
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
    border-radius: 6px;
    padding: 7px 10px;
    font-size: 13px;
    outline: none;
    width: 100%;
    box-sizing: border-box;
  }
  .form-input:focus {
    border-color: var(--fg);
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
  .sync-btn {
    padding: 9px 12px;
    border-radius: 6px;
    border: 1px solid var(--border-strong);
    background: var(--bg-elevated);
    color: var(--fg);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    transition: all 0.12s;
  }
  .sync-btn:hover:not(:disabled) {
    border-color: var(--fg);
  }
  .sync-btn.upload:hover:not(:disabled) {
    background: #4a7cf7;
    color: #fff;
    border-color: #4a7cf7;
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
    border-radius: 6px;
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
