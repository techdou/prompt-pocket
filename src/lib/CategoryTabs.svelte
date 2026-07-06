<script lang="ts">
  import {
    ALL_CATEGORY_TAB_INDEX,
    getHorizontalCategoryDropTarget,
  } from "./reorder";
  import type { CategoryCount } from "./types";

  let {
    categories,
    total,
    selected = $bindable("__all__"),
    oncreate,
    onrename,
    oncontextmenu,
    onreorder,
  }: {
    categories: CategoryCount[];
    total: number;
    selected: string;
    oncreate: (name: string) => void;
    onrename: (oldName: string) => void;
    oncontextmenu: (name: string, x: number, y: number) => void;
    /** 拖拽结束回调：把 fromIndex 的分类移到 toIndex 前 */
    onreorder: (fromIndex: number, toIndex: number) => void;
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

  // ── 水平拖拽排序（Pointer Events，与 PromptList 同模式但走 X 坐标）──
  // 和 PromptList 保持一致：只从左侧手柄开始拖，pointerdown 后立即进入拖动。
  // 这样不会和 tab 的 click 选中、横向滚动区、Tauri 窗口拖动手势互相抢事件。
  let scrollEl: HTMLDivElement | null = null;
  let dragFromIndex = $state(-1); // 正在拖的分类索引（按 categories 数组，不含"全部"）
  let isDragging = $state(false);
  let dropToIndex = $state(-1); // 松手插入到哪个 index 之前（按 categories 数组）
  let dropLineIndex = $state(-1); // 落点指示线挂在哪项上（-1 = 不显示）
  let dropLineBefore = $state(true); // 指示线画在该项左侧还是右侧
  let activePointerId = -1;

  function onHandlePointerDown(e: PointerEvent, index: number) {
    if (e.button !== 0) return;
    e.preventDefault();
    e.stopPropagation();

    activePointerId = e.pointerId;
    dragFromIndex = index;
    isDragging = true;
    updateDropTarget(e.clientX, e.clientY);

    window.addEventListener("pointermove", onWindowPointerMove, { passive: false });
    window.addEventListener("pointerup", onWindowPointerUp, { passive: false });
    window.addEventListener("pointercancel", onWindowPointerCancel);
  }

  function onWindowPointerMove(e: PointerEvent) {
    if (e.pointerId !== activePointerId || dragFromIndex < 0) return;
    e.preventDefault();
    updateDropTarget(e.clientX, e.clientY);
  }

  function onWindowPointerUp(e: PointerEvent) {
    if (e.pointerId !== activePointerId) return;
    e.preventDefault();
    finishPointerDrag(true);
  }

  function onWindowPointerCancel(e: PointerEvent) {
    if (e.pointerId !== activePointerId) return;
    finishPointerDrag(false);
  }

  function updateDropTarget(clientX: number, clientY: number) {
    const scrollRect = scrollEl?.getBoundingClientRect();
    if (
      !scrollEl ||
      !scrollRect ||
      clientX < scrollRect.left ||
      clientX > scrollRect.right ||
      clientY < scrollRect.top ||
      clientY > scrollRect.bottom
    ) {
      dropLineIndex = -1;
      dropToIndex = -1;
      return;
    }

    const tabs = Array.from(scrollEl.querySelectorAll<HTMLElement>("[data-tab-idx]"))
      .map((el) => {
        const rect = el.getBoundingClientRect();
        return {
          tabIdx: Number(el.dataset.tabIdx),
          left: rect.left,
          right: rect.right,
        };
      })
      .filter((tab) => Number.isFinite(tab.tabIdx));
    const target = getHorizontalCategoryDropTarget(tabs, clientX);
    dropLineIndex = target?.lineIndex ?? -1;
    dropLineBefore = target?.lineBefore ?? true;
    dropToIndex = target?.toIndex ?? -1;
  }

  function finishPointerDrag(commit: boolean) {
    const from = dragFromIndex;
    const to = dropToIndex;
    resetDrag();

    if (!commit || from < 0 || to < 0) return;
    // 落在原位（自身左侧或自身右侧）→ 无变化
    if (to === from || to === from + 1) return;
    onreorder(from, to);
  }

  function resetDrag() {
    window.removeEventListener("pointermove", onWindowPointerMove);
    window.removeEventListener("pointerup", onWindowPointerUp);
    window.removeEventListener("pointercancel", onWindowPointerCancel);
    activePointerId = -1;
    isDragging = false;
    dragFromIndex = -1;
    dropToIndex = -1;
    dropLineIndex = -1;
    dropLineBefore = true;
  }

  function onTabClick(e: MouseEvent, name: string) {
    pick(name);
  }

  // 防原生 DnD 干扰
  function onNativeDragStart(e: DragEvent) {
    e.preventDefault();
  }

  function showDropLineBefore(tabIdx: number): boolean {
    return isDragging && dropLineIndex === tabIdx && dropLineBefore && tabIdx !== dragFromIndex;
  }
  function showDropLineAfter(tabIdx: number): boolean {
    return isDragging && dropLineIndex === tabIdx && !dropLineBefore && tabIdx !== dragFromIndex;
  }
</script>

<div class="tabs-row">
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="tabs-scroll" bind:this={scrollEl} ondragstart={onNativeDragStart}>
    <!-- "全部"：固定首位，不可拖，但可作为落点（拖到它右侧 = 排第一） -->
    <button
      type="button"
      class="tab"
      class:active={selected === "__all__"}
      data-tab-idx={ALL_CATEGORY_TAB_INDEX}
      class:drop-before={showDropLineBefore(ALL_CATEGORY_TAB_INDEX)}
      class:drop-after={showDropLineAfter(ALL_CATEGORY_TAB_INDEX)}
      draggable="false"
      onclick={(e) => onTabClick(e, "__all__")}
    >
      全部<span class="num">{total}</span>
    </button>

    {#each categories as cat, i (cat.name)}
      <button
        type="button"
        class="tab"
        class:active={selected === cat.name}
        class:dragging={isDragging && i === dragFromIndex}
        class:drop-before={showDropLineBefore(i)}
        class:drop-after={showDropLineAfter(i)}
        draggable="false"
        data-tab-idx={i}
        onclick={(e) => onTabClick(e, cat.name)}
        oncontextmenu={(e) => {
          e.preventDefault();
          oncontextmenu(cat.name, e.clientX, e.clientY);
        }}
        title={cat.name}
      >
        <span
          class="drag-handle"
          title="拖拽排序"
          aria-label="拖拽排序"
          role="button"
          tabindex="-1"
          onpointerdown={(e) => onHandlePointerDown(e, i)}
          onclick={(e) => e.stopPropagation()}
        >
          ⠿
        </span>
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
      <button type="button" class="mini" onclick={submitCreate}>✓</button>
      <button
        type="button"
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
      type="button"
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
    flex-shrink: 0;
    transition:
      background 0.12s,
      border-color 0.12s,
      color 0.12s,
      opacity 0.12s;
    user-select: none;
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
  .tab:hover .drag-handle,
  .tab.active .drag-handle {
    opacity: 0.55;
  }
  /* 拖动中的源项半透明 */
  .tab.dragging {
    opacity: 0.4;
  }
  /* 落点指示线：垂直 2px 线，画在 tab 左/右两侧 */
  .tab.drop-before::before,
  .tab.drop-after::after {
    content: "";
    position: absolute;
    top: 3px;
    bottom: 3px;
    width: 2px;
    background: var(--accent);
    border-radius: 1px;
    pointer-events: none;
  }
  .tab.drop-before::before {
    left: -4px;
  }
  .tab.drop-after::after {
    right: -4px;
  }

  .name {
    max-width: 120px;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .drag-handle {
    width: 12px;
    height: 16px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    color: var(--muted);
    opacity: 0;
    cursor: grab;
    user-select: none;
    touch-action: none;
    line-height: 1;
  }
  .drag-handle:active {
    cursor: grabbing;
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
