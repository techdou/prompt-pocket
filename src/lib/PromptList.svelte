<script lang="ts">
  import type { Prompt } from "./types";
  import { dndzone } from "svelte-dnd-action";
  import type { DndEvent } from "svelte-dnd-action";

  let {
    prompts,
    selectedPath,
    selectedIndex,
    onselect,
    oncontextmenu,
    onreorder,
    draggable = true,
  }: {
    prompts: Prompt[];
    selectedPath: string | null;
    selectedIndex: number;
    onselect: (path: string) => void;
    oncontextmenu: (prompt: Prompt, x: number, y: number) => void;
    onreorder: (from: number, to: number) => void;
    draggable?: boolean;
  } = $props();

  // dnd-action 需要带 id 的项
  type Item = { id: string; data: Prompt };
  let items = $state<Item[]>([]);
  $effect(() => {
    items = prompts.map((p) => ({ id: p.path, data: p }));
  });

  // dndzone 配置
  const flipDurationMs = 200;

  function onConsidered(e: CustomEvent<DndEvent>) {
    const newItems = e.detail.items as Item[];
    items = newItems;
  }

  function onDropped(e: CustomEvent<DndEvent>) {
    const newItems = e.detail.items as Item[];
    // 找到被移动项的原始位置和新位置
    const oldPaths = prompts.map((p) => p.path);
    const newPaths = newItems.map((i) => i.id);
    // 找第一个不同的位置
    let from = -1;
    let to = -1;
    for (let i = 0; i < oldPaths.length; i++) {
      if (oldPaths[i] !== newPaths[i]) {
        from = i;
        break;
      }
    }
    if (from < 0) return;
    const movedId = oldPaths[from];
    to = newPaths.indexOf(movedId);
    if (to >= 0 && to !== from) {
      items = newItems;
      onreorder(from, to);
    }
  }
</script>

<ul
  class="list"
  role="listbox"
  use:dndzone={{ items, flipDurationMs }}
  onconsidered={onConsidered}
  ondropped={onDropped}
>
  {#each items as item, i (item.id)}
    <li
      role="option"
      tabindex="-1"
      aria-selected={item.id === selectedPath}
      class="item"
      class:active={item.id === selectedPath}
      onclick={() => onselect(item.id)}
      onkeydown={(e) => {
        if (e.key === " " || e.key === "Enter") {
          e.preventDefault();
          onselect(item.id);
        }
      }}
      oncontextmenu={(e) => {
        e.preventDefault();
        oncontextmenu(item.data, e.clientX, e.clientY);
      }}
    >
      <div class="main">
        <div class="title-row">
          <span class="drag-handle" title="拖拽排序">⠿</span>
          <span class="title">{item.data.title}</span>
        </div>
        <div class="sub">
          <span class="cat">{item.data.category}</span>
        </div>
      </div>
      <button
        class="more-btn"
        title="更多操作"
        aria-label="更多操作"
        onclick={(e) => {
          e.stopPropagation();
          const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
          oncontextmenu(item.data, rect.right, rect.bottom);
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
    position: relative;
  }
  .item:hover {
    background: var(--bg-hover);
  }
  .item:hover .more-btn {
    opacity: 1;
  }
  .item:hover .drag-handle {
    opacity: 0.4;
  }
  .item.active {
    background: var(--bg-active);
  }
  .item.active .more-btn {
    opacity: 0.7;
  }
  .item.active .drag-handle {
    opacity: 0.4;
  }

  /* svelte-dnd-action 拖拽中的项 */
  .item:global(.monaco-dragged) {
    opacity: 0.4;
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
  .drag-handle {
    font-size: 13px;
    color: var(--muted);
    opacity: 0;
    cursor: grab;
    user-select: none;
    line-height: 1;
  }
  .drag-handle:active {
    cursor: grabbing;
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
    margin-left: 17px;
    font-size: 11px;
    color: var(--muted);
    overflow: hidden;
  }
  .cat {
    flex-shrink: 0;
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
