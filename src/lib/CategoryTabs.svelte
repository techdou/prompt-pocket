<script lang="ts">
  import type { CategoryCount } from "./types";

  let {
    categories,
    total,
    selected = $bindable("__all__"),
    oncreate,
  }: {
    categories: CategoryCount[];
    total: number;
    selected: string;
    oncreate: (name: string) => void;
  } = $props();

  function pick(name: string) {
    selected = name;
  }

  let creating = $state(false);
  let newName = $state("");

  function submitCreate() {
    const name = newName.trim();
    if (name) {
      oncreate(name);
      newName = "";
      creating = false;
    }
  }
</script>

<div class="tabs-row">
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

  {#if creating}
    <div class="new-cat-input">
      <input
        type="text"
        bind:value={newName}
        placeholder="分类名"
        onkeydown={(e) => {
          if (e.key === "Enter") submitCreate();
          if (e.key === "Escape") {
            creating = false;
            newName = "";
          }
        }}
      />
      <button class="mini" onclick={submitCreate}>✓</button>
      <button
        class="mini"
        onclick={() => {
          creating = false;
          newName = "";
        }}
      >
        ×
      </button>
    </div>
  {:else}
    <button
      class="add-btn"
      title="新建分类"
      onclick={() => {
        creating = true;
        newName = "";
      }}
    >
      +
    </button>
  {/if}
</div>

<style>
  .tabs-row {
    display: flex;
    align-items: center;
    height: 100%;
    gap: 4px;
  }

  .tabs-scroll {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: 1;
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

  .add-btn {
    flex-shrink: 0;
    width: 26px;
    height: 26px;
    border-radius: 6px;
    border: none;
    background: var(--bg-hover);
    color: var(--muted);
    font-size: 18px;
    line-height: 1;
    cursor: pointer;
    transition: all 0.12s;
  }
  .add-btn:hover {
    background: var(--bg-active);
    color: var(--fg);
  }

  .new-cat-input {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .new-cat-input input {
    width: 100px;
    height: 26px;
    background: var(--bg-elevated);
    border: 1px solid var(--border-strong);
    color: var(--fg);
    border-radius: 6px;
    padding: 0 8px;
    font-size: 12.5px;
    outline: none;
  }
  .new-cat-input input:focus {
    border-color: var(--fg);
  }
  .mini {
    width: 26px;
    height: 26px;
    border-radius: 6px;
    border: none;
    background: var(--bg-hover);
    color: var(--fg);
    font-size: 14px;
    cursor: pointer;
  }
</style>
