<script lang="ts">
  import type { CategoryCount } from "./types";

  let {
    categories,
    total,
    selected = $bindable("__all__"),
    oncreate,
    onrename,
    oncontextmenu,
  }: {
    categories: CategoryCount[];
    total: number;
    selected: string;
    oncreate: (name: string) => void;
    onrename: (oldName: string) => void;
    oncontextmenu: (name: string, x: number, y: number) => void;
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
        oncontextmenu={(e) => {
          e.preventDefault();
          oncontextmenu(cat.name, e.clientX, e.clientY);
        }}
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
    gap: 8px;
  }

  .tabs-scroll {
    display: flex;
    align-items: center;
    gap: 6px;
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
    height: 30px;
    padding: 0 11px;
    background: transparent;
    border: 1px solid transparent;
    border-radius: 999px;
    color: var(--muted);
    font-size: 13px;
    white-space: nowrap;
    cursor: pointer;
    position: relative;
    transition:
      background 0.12s,
      border-color 0.12s,
      color 0.12s;
  }
  .tab:hover {
    color: var(--fg);
    background: var(--bg-hover);
  }
  .tab.active {
    color: var(--accent);
    background: var(--accent-soft);
    border-color: #c9dafc;
    font-weight: 600;
  }

  .name {
    max-width: 120px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .num {
    font-size: 11px;
    color: inherit;
    background: rgba(255, 255, 255, 0.72);
    border: 1px solid var(--border);
    border-radius: 999px;
    min-width: 18px;
    height: 18px;
    padding: 0 5px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
  }

  .add-btn {
    flex-shrink: 0;
    width: 30px;
    height: 30px;
    border-radius: 9px;
    border: 1px solid var(--border);
    background: var(--bg-elevated);
    color: var(--accent);
    font-size: 18px;
    line-height: 1;
    cursor: pointer;
    transition: all 0.12s;
  }
  .add-btn:hover {
    background: var(--accent-soft);
    border-color: #c9dafc;
    color: var(--accent-hover);
  }

  .new-cat-input {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .new-cat-input input {
    width: 100px;
    height: 30px;
    background: var(--bg-elevated);
    border: 1px solid var(--border);
    color: var(--fg);
    border-radius: 8px;
    padding: 0 8px;
    font-size: 12.5px;
    outline: none;
  }
  .new-cat-input input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-soft);
  }
  .mini {
    width: 30px;
    height: 30px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: var(--bg-elevated);
    color: var(--accent);
    font-size: 14px;
    cursor: pointer;
  }
  .mini:hover {
    background: var(--accent-soft);
    border-color: #c9dafc;
  }
</style>
