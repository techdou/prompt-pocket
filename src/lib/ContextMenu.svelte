<script lang="ts">
  import { fly } from "svelte/transition";
  import type { CategoryCount, Prompt } from "./types";
  import { createTranslator, type Translator } from "./i18n";

  const fallbackT = createTranslator("zh");

  let {
    open = $bindable(false),
    prompt,
    x = 0,
    y = 0,
    categories = [],
    onrename,
    onmove,
    ondelete,
    onclose,
    t = fallbackT,
  }: {
    open: boolean;
    prompt: Prompt | null;
    x: number;
    y: number;
    categories: CategoryCount[];
    onrename: () => void;
    onmove: (category: string) => void;
    ondelete: () => void;
    onclose: () => void;
    t?: Translator;
  } = $props();

  // 移动分类子菜单
  let showMoveMenu = $state(false);

  // open 被外部置 false（如 App 的 Esc 链直接关闭）时复位子菜单状态，
  // 避免下次打开时移动子菜单直接展开
  $effect(() => {
    if (!open) showMoveMenu = false;
  });

  let categoryOptions = $derived([
    "未分类",
    ...categories.map((c) => c.name).filter((n) => n !== "未分类"),
  ]);

  // 视口边界翻转：菜单挂载后测量，越出窗口右/下缘则向左/上收回
  let menuEl: HTMLDivElement | null = $state(null);
  let posX = $state(0);
  let posY = $state(0);

  $effect(() => {
    if (!open || !menuEl) return;
    // showMoveMenu 展开会改变高度，纳入依赖重测
    void showMoveMenu;
    const rect = menuEl.getBoundingClientRect();
    posX = Math.max(4, Math.min(x, window.innerWidth - rect.width - 8));
    posY = Math.max(4, Math.min(y, window.innerHeight - rect.height - 8));
  });

  function categoryLabel(name: string): string {
    return name === "未分类" ? t("common.uncategorized") : name;
  }

  function handle(action: () => void) {
    action();
    close();
  }

  function close() {
    showMoveMenu = false;
    open = false;
    onclose();
  }

  function onBackdrop(e: MouseEvent) {
    if (e.target === e.currentTarget) close();
  }
</script>

<svelte:window
  onkeydown={(e) => {
    if (e.key === "Escape" && open) {
      // preventDefault：App 的 window handler 看到 defaultPrevented 就不再穿透隐藏窗口
      e.preventDefault();
      close();
    }
  }}
/>

{#if open && prompt}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="backdrop"
    onclick={onBackdrop}
    oncontextmenu={(e) => {
      e.preventDefault();
      close();
    }}
    transition:fly={{ duration: 80 }}
  ></div>
  <div
    bind:this={menuEl}
    class="menu"
    style="left: {posX || x}px; top: {posY || y}px;"
    transition:fly={{ y: -4, duration: 100 }}
  >
    <button class="item" onclick={() => handle(onrename)}>
      <span class="ico">✎</span> {t("context.rename")}
    </button>

    <button class="item" onclick={() => (showMoveMenu = !showMoveMenu)}>
      <span class="ico">📁</span> {t("context.moveToCategory")}
      <span class="arrow">{showMoveMenu ? "▾" : "▸"}</span>
    </button>

    {#if showMoveMenu}
      <div class="sublist">
        {#each categoryOptions as c}
          <button
            class="sub-item"
            class:current={prompt.category === c}
            onclick={() => handle(() => onmove(c))}
          >
            <span>{categoryLabel(c)}</span>
            {#if prompt.category === c}<span class="check">✓</span>{/if}
          </button>
        {/each}
      </div>
    {/if}

    <div class="sep"></div>

    <button class="item danger" onclick={() => handle(ondelete)}>
      <span class="ico">🗑</span> {t("context.delete")}
    </button>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 150;
    background: transparent;
  }

  .menu {
    position: fixed;
    z-index: 160;
    min-width: 168px;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: var(--shadow-soft);
    padding: 4px;
    user-select: none;
  }

  .item {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    background: transparent;
    border: none;
    color: var(--fg);
    font-size: 13px;
    padding: 7px 10px;
    border-radius: 7px;
    cursor: pointer;
    text-align: left;
    font-family: inherit;
  }
  .item:hover {
    background: var(--bg-hover);
    color: var(--accent);
  }
  .item.danger {
    color: var(--danger);
  }
  .ico {
    width: 16px;
    text-align: center;
    opacity: 0.8;
  }
  .arrow {
    margin-left: auto;
    color: var(--muted);
    font-size: 11px;
  }

  .sublist {
    padding: 2px 0 2px 16px;
    display: flex;
    flex-direction: column;
    gap: 1px;
    border-left: 1px solid var(--border);
    margin-left: 10px;
  }
  .sub-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
    background: transparent;
    border: none;
    color: var(--fg);
    font-size: 12.5px;
    padding: 5px 10px;
    border-radius: 7px;
    cursor: pointer;
    text-align: left;
    font-family: inherit;
  }
  .sub-item:hover {
    background: var(--bg-hover);
    color: var(--accent);
  }
  .check {
    color: var(--accent);
    font-size: 11px;
  }

  .sep {
    height: 1px;
    background: var(--border);
    margin: 3px 6px;
  }
</style>
