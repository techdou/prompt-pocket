<script lang="ts">
  import type { Prompt } from "./types";

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

  // 拖拽状态：用一个简单的外部变量（不依赖响应式，避免渲染时机问题）
  let dragFromIndex = -1;
  let dragOverIndex = $state(-1);
  let dragBefore = $state(true);

  function onDragStart(e: DragEvent, i: number) {
    if (!draggable) {
      e.preventDefault();
      return;
    }
    dragFromIndex = i;
    // 必须设置 dataTransfer，否则部分浏览器不触发后续事件
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = "move";
      e.dataTransfer.setData("text/plain", String(i));
    }
  }

  // 关键：dragover 必须无条件 preventDefault，否则 drop 永远不触发
  function onDragOver(e: DragEvent, i: number) {
    if (!draggable) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";

    const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
    dragOverIndex = i;
    dragBefore = e.clientY < rect.top + rect.height / 2;
  }

  function onDrop(e: DragEvent, i: number) {
    e.preventDefault();
    if (!draggable || dragFromIndex < 0) {
      resetDrag();
      return;
    }
    let to = dragBefore ? i : i + 1;
    if (to === dragFromIndex || to === dragFromIndex + 1) {
      resetDrag();
      return;
    }
    if (to > dragFromIndex) to -= 1;
    onreorder(dragFromIndex, to);
    resetDrag();
  }

  function onDragEnd() {
    resetDrag();
  }

  // 容器级 dragover：拖到空白处也允许 preventDefault
  function onListDragOver(e: DragEvent) {
    if (!draggable) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "move";
  }

  function resetDrag() {
    dragFromIndex = -1;
    dragOverIndex = -1;
  }

  function showLineBefore(i: number): boolean {
    return draggable && dragFromIndex >= 0 && dragOverIndex === i && dragBefore && i !== dragFromIndex;
  }
  function showLineAfter(i: number): boolean {
    return draggable && dragFromIndex >= 0 && dragOverIndex === i && !dragBefore && i !== dragFromIndex;
  }
</script>

<ul
  class="list"
  role="listbox"
  ondragover={onListDragOver}
>
  {#each prompts as p, i (p.path)}
    <li
      role="option"
      tabindex="-1"
      aria-selected={p.path === selectedPath}
      class="item"
      class:active={p.path === selectedPath}
      class:dragging={dragFromIndex === i}
      class:line-before={showLineBefore(i)}
      class:line-after={showLineAfter(i)}
      draggable={draggable}
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
      ondragstart={(e) => onDragStart(e, i)}
      ondragover={(e) => onDragOver(e, i)}
      ondrop={(e) => onDrop(e, i)}
      ondragend={onDragEnd}
    >
      <div class="main">
        <div class="title-row">
          <span class="drag-handle" title="拖拽排序">⠿</span>
          <span class="title">{p.title}</span>
        </div>
        <div class="sub">
          <span class="cat">{p.category}</span>
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

  .item.dragging {
    opacity: 0.4;
  }

  .item.line-before::before {
    content: "";
    position: absolute;
    left: 6px;
    right: 6px;
    top: -2px;
    height: 2px;
    background: #4a7cf7;
    border-radius: 1px;
  }
  .item.line-after::after {
    content: "";
    position: absolute;
    left: 6px;
    right: 6px;
    bottom: -2px;
    height: 2px;
    background: #4a7cf7;
    border-radius: 1px;
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
