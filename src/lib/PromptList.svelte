<script lang="ts">
  import type { Prompt } from "./types";

  let {
    prompts,
    selectedPath,
    selectedIndex,
    onselect,
    oncontextmenu,
    onmounted,
  }: {
    prompts: Prompt[];
    selectedPath: string | null;
    selectedIndex: number;
    onselect: (path: string) => void;
    oncontextmenu: (prompt: Prompt, x: number, y: number) => void;
    onmounted: (scrollToIndex: (i: number) => void) => void;
  } = $props();

  // 内部持有 DOM 引用，通过回调暴露滚动能力给父组件
  let itemEls: HTMLLIElement[] = $state([]);

  function scrollToIndex(i: number) {
    itemEls[i]?.scrollIntoView({ block: "nearest" });
  }

  // 挂载时把滚动函数上报
  $effect(() => {
    onmounted(scrollToIndex);
  });
</script>

<ul class="list" role="listbox">
  {#each prompts as p, i (p.path)}
    <li
      bind:this={itemEls[i]}
      role="option"
      tabindex="-1"
      aria-selected={p.path === selectedPath}
      class="item"
      class:active={p.path === selectedPath}
      class:pinned={p.meta.pinned}
      onclick={() => onselect(p.path)}
      onkeydown={(e) => {
        if (e.key === " " || e.key === "Enter") {
          e.preventDefault();
          onselect(p.path);
        }
      }}
      oncontextmenu={(e) => {
        e.preventDefault();
        oncontextmenu(p, e.clientX, e.clientY);
      }}
    >
      <div class="main">
        <div class="title-row">
          {#if p.meta.pinned}<span class="pin">★</span>{/if}
          <span class="title">{p.title}</span>
        </div>
        <div class="sub">
          <span class="cat">{p.category}</span>
          {#if p.meta.tags.length > 0}
            <span class="tags">{p.meta.tags.map((t) => "#" + t).join(" ")}</span>
          {/if}
        </div>
      </div>
      <button
        class="more-btn"
        title="更多操作"
        aria-label="更多操作"
        onclick={(e) => {
          e.stopPropagation();
          const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
          oncontextmenu(p, rect.right, rect.bottom);
        }}
      >
        ⋯
      </button>
    </li>
  {:else}
    <li class="empty">没有匹配的提示词</li>
  {/each}
</ul>

<style>
  .list {
    list-style: none;
    margin: 0;
    padding: 6px;
    border-right: 1px solid var(--border);
    overflow-y: auto;
    min-height: 0;
  }

  .item {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 9px 10px;
    border-radius: 6px;
    cursor: pointer;
    transition: background 0.1s;
  }
  .item:hover {
    background: var(--bg-hover);
  }
  .item:hover .more-btn {
    opacity: 1;
  }
  .item.active {
    background: var(--bg-active);
  }
  .item.active .more-btn {
    opacity: 0.7;
  }

  .main {
    flex: 1;
    min-width: 0;
  }
  .title-row {
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .pin {
    font-size: 10px;
    color: var(--fg);
  }
  .title {
    font-size: 13.5px;
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sub {
    display: flex;
    gap: 8px;
    margin-top: 2px;
    font-size: 11px;
    color: var(--muted);
    overflow: hidden;
  }
  .cat {
    flex-shrink: 0;
  }
  .tags {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .more-btn {
    flex-shrink: 0;
    background: transparent;
    border: none;
    color: var(--muted);
    font-size: 16px;
    line-height: 1;
    padding: 2px 4px;
    border-radius: 4px;
    cursor: pointer;
    opacity: 0;
    transition: opacity 0.12s, background 0.12s;
  }
  .more-btn:hover {
    background: var(--bg-active);
    color: var(--fg);
  }

  .empty {
    padding: 32px 12px;
    text-align: center;
    color: var(--muted);
    font-size: 13px;
  }
</style>
