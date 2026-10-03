<script lang="ts">
  import type { Translator } from "./i18n";
  import { templateFields, renderTemplate, missingTemplateFields } from "./templates";
  import { dialogFocus } from "./dialog";
  let { body, title, t, onclose, onapply, onraw }: {
    body: string; title: string; t: Translator; onclose: () => void; onapply: (text: string) => Promise<void>; onraw?: () => Promise<void>;
  } = $props();
  let fields = $derived(templateFields(body));
  let values = $state<Record<string, string>>({});
  let busy = $state(false);
  let error = $state("");
  let output = $derived(renderTemplate(body, values));
  let missing = $derived(missingTemplateFields(body, values));
  $effect(() => { values = Object.fromEntries(fields.map((field) => [field.name, field.defaultValue])); });
  async function copyRaw() {
    if (busy || !onraw) return;
    busy = true;
    try { await onraw(); } catch (e) { error = String(e); } finally { busy = false; }
  }
  async function apply() {
    if (missing.length || busy) return;
    busy = true;
    try { await onapply(output); } catch (e) { error = String(e); } finally { busy = false; }
  }
</script>

<div class="overlay" role="presentation" onclick={(event) => { if (event.target === event.currentTarget && !busy) onclose(); }} onkeydown={(event) => { if (event.key === 'Escape' && !busy) { event.stopPropagation(); onclose(); } }}>
  <div class="template-dialog" role="dialog" aria-modal="true" aria-labelledby="template-title" tabindex="-1" use:dialogFocus>
    <header><div><h2 id="template-title">{t("template.title")}</h2><p>{title}</p></div><button class="ghost" onclick={onclose} disabled={busy} aria-label={t("common.close")}>×</button></header>
    <p class="hint">{t("template.hint")}</p>
    <div class="contents">
      <div class="fields">
        {#each fields as field, index}
          <label for={`variable-${index}`}>{field.name}<textarea id={`variable-${index}`} rows="3" bind:value={values[field.name]} disabled={busy} required></textarea></label>
        {/each}
      </div>
      <div class="output"><h3>{t("template.preview")}</h3><pre>{output}</pre></div>
    </div>
    {#if error}<p class="error" role="alert">{error}</p>{/if}
    <footer><span class="hint">{missing.length ? t("template.missing") : ""}</span>{#if onraw}<button class="ghost" disabled={busy} onclick={copyRaw}>{t("template.raw")}</button>{/if}<button class="ghost" disabled={busy} onclick={onclose}>{t("common.cancel")}</button><button class="primary" disabled={!!missing.length || busy} onclick={apply}>{busy ? t("editor.loading") : t("template.apply")}</button></footer>
  </div>
</div>

<style>
  .overlay { position: fixed; inset: 0; z-index: 3000; background: #14234055; display: grid; place-items: center; padding: 20px; }
  .template-dialog { background: var(--bg-elevated); border: 1px solid var(--border); border-radius: 14px; padding: 20px; width: min(860px, 100%); max-height: 90vh; display: flex; flex-direction: column; box-shadow: var(--shadow-soft); }
  header, footer { display: flex; align-items: center; gap: 12px; flex-shrink: 0; } header { justify-content: space-between; } h2 { font-size: 19px; margin: 0; } header p { color: var(--muted); margin: 5px 0 0; } .hint { color: var(--muted); font-size: 12px; }
  .contents { display: grid; grid-template-columns: 1fr 1.15fr; gap: 20px; overflow: auto; min-height: 0; margin: 8px 0 16px; } label { display: block; font-size: 13px; margin-bottom: 14px; } textarea { display: block; width: 100%; margin-top: 7px; padding: 9px; resize: vertical; border: 1px solid var(--border-strong); border-radius: 7px; background: var(--bg); color: var(--fg); font: inherit; line-height: 1.5; }
  h3 { font-size: 13px; margin: 0 0 9px; } pre { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; font: 13px/1.7 var(--font-ui); user-select: text; } .output { background: var(--bg); border-radius: 8px; padding: 12px; min-width: 0; } footer .hint { flex: 1; } .error { color: var(--danger); }
  @media (max-width: 700px) { .contents { grid-template-columns: 1fr; } }
</style>
