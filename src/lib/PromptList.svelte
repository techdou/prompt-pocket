<script lang="ts">
  import type { Prompt } from "./types";
  import { createTranslator, type Translator } from "./i18n";

  const fallbackT = createTranslator("zh");

  let {
    prompts,
    selectedPath,
    selectedIndex,
    onselect,
    oncontextmenu,
    onreorder,
    draggable = true,
    disabledReason = "",
    t = fallbackT,
  }: {
    prompts: Prompt[];
    selectedPath: string | null;
    selectedIndex: number;
    onselect: (path: string) => void;
    oncontextmenu: (prompt: Prompt, x: number, y: number) => void;
    /** 拖拽结束回调：把 fromIndex 处的项移动到 toIndex 之前 */
    onreorder: (fromIndex: number, toIndex: number) => void;
    draggable?: boolean;
    disabledReason?: string;
    t?: Translator;
  } = $props();

  function categoryLabel(name: string): string {
    return name === "未分类" ? t("common.uncategorized") : name;
  }

  // 用 Pointer Events 实现排序，不依赖 HTML5 Drag and Drop 的 dataTransfer/drop。
  // Tauri/WebView2 里原生 DnD 容易被桌面壳和系统拖拽链路影响；指针事件只关心
  // 鼠标按下、移动、松开，落点由 elementFromPoint + getBoundingClientRect 实时计算。
  let listEl: HTMLUListElement | null = null;
  let dragFromIndex = $state(-1); // 正在拖动的项索引
  let isDragging = $state(false); // 是否在拖拽中（驱动 CSS）
  let dropLineIndex = $state(-1); // 落点指示线位置（-1 = 不显示）
  let dropLineBefore = $state(true); // 指示线画在该项之前还是之后
  let dropToIndex = $state(-1); // 松手时插入到哪个 index 之前
  let activePointerId = -1;

  function onHandlePointerDown(e: PointerEvent, index: number) {
    if (!draggable || e.button !== 0) return;
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
    const target = document.elementFromPoint(clientX, clientY);
    const itemEl =
      target instanceof Element ? target.closest<HTMLElement>("[data-idx]") : null;
    if (itemEl && listEl?.contains(itemEl)) {
      const index = Number(itemEl.dataset.idx);
      const rect = itemEl.getBoundingClientRect();
      const before = clientY - rect.top < rect.height / 2;
      dropLineIndex = index;
      dropLineBefore = before;
      dropToIndex = before ? index : index + 1;
      return;
    }

    const listRect = listEl?.getBoundingClientRect();
    if (
      listRect &&
      clientX >= listRect.left &&
      clientX <= listRect.right &&
      clientY >= listRect.top &&
      clientY <= listRect.bottom &&
      prompts.length > 0
    ) {
      // 落在列表空白区时放到末尾，并在最后一项下方显示落点线。
      dropLineIndex = prompts.length - 1;
      dropLineBefore = false;
      dropToIndex = prompts.length;
      return;
    }

    dropLineIndex = -1;
    dropLineBefore = true;
    dropToIndex = -1;
  }

  function finishPointerDrag(commit: boolean) {
    const from = dragFromIndex;
    const to = dropToIndex;
    resetDrag();

    if (!commit || from < 0 || to < 0) return;
    // 落在原位（自身上方或自身正下方）→ 无变化
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

  function onNativeDragStart(e: DragEvent) {
    // 防止图片/文字触发浏览器原生拖拽，排序统一走 pointer 事件。
    e.preventDefault();
  }

  // 指示线：在某项之前/之后显示
  function showDropLineBefore(index: number): boolean {
    return isDragging && dropLineIndex === index && dropLineBefore && index !== dragFromIndex;
  }
  function showDropLineAfter(index: number): boolean {
    return isDragging && dropLineIndex === index && !dropLineBefore && index !== dragFromIndex;
  }

  $effect(() => {
    if (!draggable && isDragging) resetDrag();
  });
</script>

<ul
  bind:this={listEl}
  class="list"
  role="listbox"
  ondragstart={onNativeDragStart}
>
  {#each prompts as p, i (p.path)}
    <li
      data-idx={i}
      role="option"
      tabindex="-1"
      aria-selected={p.path === selectedPath}
      class="item"
      class:active={p.path === selectedPath}
      class:disabled={!draggable}
      class:dragging={isDragging && i === dragFromIndex}
      class:drop-before={showDropLineBefore(i)}
      class:drop-after={showDropLineAfter(i)}
      draggable="false"
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
          <button
            type="button"
            class="drag-handle"
            title={draggable ? t("prompt.dragSort") : disabledReason}
            aria-label={draggable ? t("prompt.dragSort") : disabledReason}
            disabled={!draggable}
            onpointerdown={(e) => onHandlePointerDown(e, i)}
            onclick={(e) => e.stopPropagation()}
          >
            ⠿
          </button>
          <span class="title">{p.title}</span>
        </div>
        <div class="sub">
          <span class="cat">{categoryLabel(p.category)}</span>
        </div>
      </div>
      <button
        class="more-btn"
        title={t("prompt.moreActions")}
        aria-label={t("prompt.moreActions")}
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
    <li class="empty">{t("prompt.empty")}</li>
  {/each}
</ul>

<style>
  .list {
    list-style: none;
    margin: 0;
    padding: 8px;
    border-right: 1px solid var(--border);
    height: 100%;
    max-height: 100%;
    overflow-y: auto;
    min-height: 0;
    background: var(--bg-elevated);
  }

  .item {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 9px 10px;
    border: 1px solid transparent;
    border-radius: 9px;
    cursor: pointer;
    transition:
      background 0.12s,
      border-color 0.12s,
      box-shadow 0.12s;
    position: relative;
  }
  .item:hover {
    background: var(--bg-hover);
    border-color: var(--border);
  }
  .item:hover .more-btn {
    opacity: 1;
  }
  .item:hover .drag-handle {
    opacity: 0.4;
  }
  .item.active {
    background: var(--bg-active);
    border-color: #c9dafc;
    box-shadow: 0 1px 2px rgba(37, 99, 235, 0.08);
  }
  .item.active .more-btn {
    opacity: 0.7;
  }
  .item.active .drag-handle {
    opacity: 0.4;
  }
  .item.disabled .drag-handle {
    opacity: 0;
    cursor: default;
  }
  .item.disabled:hover .drag-handle {
    opacity: 0;
  }

  /* 拖拽中：被拖的项半透明 */
  .item.dragging {
    opacity: 0.4;
  }
  /* 落点指示线 */
  .item.drop-before::before,
  .item.drop-after::after {
    content: "";
    position: absolute;
    left: 6px;
    right: 6px;
    height: 2px;
    background: var(--accent);
    border-radius: 1px;
    pointer-events: none;
  }
  .item.drop-before::before {
    top: -3px;
  }
  .item.drop-after::after {
    bottom: -3px;
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
    width: 12px;
    height: 16px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    border: 0;
    background: transparent;
    font-family: inherit;
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
    font-weight: 600;
    color: var(--fg);
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
    border-radius: 7px;
    cursor: pointer;
    opacity: 0;
    transition: opacity 0.12s, background 0.12s;
  }
  .more-btn:hover {
    background: var(--accent-soft);
    color: var(--accent);
  }

  .empty {
    padding: 32px 12px;
    text-align: center;
    color: var(--muted);
    font-size: 13px;
  }
</style>
