<script lang="ts">
  import type { CategoryCount } from "./types";

  let {
    categories,
    total,
    selected = $bindable("__all__"),
  }: {
    categories: CategoryCount[];
    total: number;
    selected: string;
  } = $props();

  function pick(name: string) {
    selected = name;
  }
</script>

<div class="tabs-scroll">
  <button
    class="tab"
    class:active={selected === "__all__"}
    onclick={() => pick("__all__")}
  >
    全部<span class="num">{total}</span>
  </button>

  {#each categories as cat (cat.name)}
    <button
      class="tab"
      class:active={selected === cat.name}
      onclick={() => pick(cat.name)}
      title={cat.name}
    >
      <span class="name">{cat.name}</span><span class="num">{cat.count}</span>
    </button>
  {/each}
</div>

<style>
  .tabs-scroll {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 0 16px;
    overflow-x: auto;
    scrollbar-width: none;
    height: 100%;
  }
  .tabs-scroll::-webkit-scrollbar {
    display: none;
  }

  .tab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0 12px;
    height: 100%;
    background: transparent;
    border: none;
    color: var(--muted);
    font-size: 13px;
    white-space: nowrap;
    cursor: pointer;
    position: relative;
    transition: color 0.12s;
  }
  .tab:hover {
    color: var(--fg);
  }
  /* 选中：文字变黑，底部出现黑线 */
  .tab.active {
    color: var(--fg);
    font-weight: 500;
  }
  .tab.active::after {
    content: "";
    position: absolute;
    left: 12px;
    right: 12px;
    bottom: 0;
    height: 2px;
    background: var(--fg);
  }

  .name {
    max-width: 120px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .num {
    font-size: 11px;
    color: var(--muted);
    background: transparent;
    padding: 0;
  }
  .tab.active .num {
    color: var(--muted);
  }
</style>
