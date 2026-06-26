<script lang="ts">
  import type { Prompt } from "./types";

  let {
    prompts,
    selectedPath,
    selectedIndex,
    listRefs = $bindable([]),
    onselect,
  }: {
    prompts: Prompt[];
    selectedPath: string | null;
    selectedIndex: number;
    listRefs: HTMLLIElement[];
    onselect: (path: string) => void;
  } = $props();
</script>

<ul class="list" role="listbox">
  {#each prompts as p, i (p.path)}
    <li
      bind:this={listRefs[i]}
      role="option"
      tabindex="-1"
      aria-selected={p.path === selectedPath}
      class="item"
      class:active={p.path === selectedPath}
      onclick={() => onselect(p.path)}
      onkeydown={(e) => {
        if (e.key === " " || e.key === "Enter") {
          e.preventDefault();
          onselect(p.path);
        }
      }}
    >
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
    padding: 9px 10px;
    border-radius: 6px;
    cursor: pointer;
    transition: background 0.1s;
  }
  .item:hover {
    background: var(--bg-hover);
  }
  /* 选中：浅灰填充（参考稿的克制感） */
  .item.active {
    background: var(--bg-active);
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

  .empty {
    padding: 32px 12px;
    text-align: center;
    color: var(--muted);
    font-size: 13px;
  }
</style>
