<script lang="ts">
  import { fly } from "svelte/transition";
  import type { CategoryCount, Prompt } from "./types";

  let {
    open = $bindable(false),
    prompt,
    x = 0,
    y = 0,
    categories = [],
    onrename,
    onmove,
    ontogglepin,
    ondelete,
    onclose,
  }: {
    open: boolean;
    prompt: Prompt | null;
    x: number;
    y: number;
    categories: CategoryCount[];
    onrename: () => void;
    onmove: (category: string) => void;
    ontogglepin: () => void;
    ondelete: () => void;
    onclose: () => void;
  } = $props();

  // 移动分类子菜单
  let showMoveMenu = $state(false);

  let categoryOptions = $derived(["未分类", ...categories.map((c) => c.name)]);

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
  onkeydown={(e) => e.key === "Escape" && close()}
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
    class="menu"
    style="left: {x}px; top: {y}px;"
    transition:fly={{ y: -4, duration: 100 }}
  >
    <button class="item" onclick={() => handle(onrename)}>
      <span class="ico">✎</span> 重命名…
    </button>

    <button class="item" onclick={() => (showMoveMenu = !showMoveMenu)}>
      <span class="ico">📁</span> 移动到分类
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
            <span>{c}</span>
            {#if prompt.category === c}<span class="check">✓</span>{/if}
          </button>
        {/each}
      </div>
    {/if}

    <button class="item" onclick={() => handle(ontogglepin)}>
      <span class="ico">{prompt.meta.pinned ? "☆" : "★"}</span>
      {prompt.meta.pinned ? "取消置顶" : "置顶"}
    </button>

    <div class="sep"></div>

    <button class="item danger" onclick={() => handle(ondelete)}>
      <span class="ico">🗑</span> 删除…
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
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 8px;
    box-shadow: 0 8px 28px rgba(0, 0, 0, 0.16);
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
    border-radius: 5px;
    cursor: pointer;
    text-align: left;
    font-family: inherit;
  }
  .item:hover {
    background: var(--bg-hover);
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
    border-radius: 5px;
    cursor: pointer;
    text-align: left;
    font-family: inherit;
  }
  .sub-item:hover {
    background: var(--bg-hover);
  }
  .check {
    color: var(--fg);
    font-size: 11px;
  }

  .sep {
    height: 1px;
    background: var(--border);
    margin: 3px 6px;
  }
</style>
