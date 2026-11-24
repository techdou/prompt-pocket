/**
 * Keep keyboard focus in an open dialog and return it to its trigger on close.
 * `initial` (optional CSS selector, looked up inside the dialog) names the element
 * that gets focus on open instead of the first focusable element.
 */
export function dialogFocus(node: HTMLElement, params: { initial?: string } = {}) {
  const previous = document.activeElement as HTMLElement | null;
  const visible = (element: HTMLElement) => element.getClientRects().length > 0;
  const focusables = () => Array.from(node.querySelectorAll<HTMLElement>(
    'button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), [tabindex="0"]',
  )).filter(visible);
  queueMicrotask(() => {
    const preferred = params.initial
      ? Array.from(node.querySelectorAll<HTMLElement>(params.initial)).find(visible)
      : undefined;
    (preferred ?? focusables()[0] ?? node).focus();
  });
  const trap = (event: KeyboardEvent) => {
    if (event.key !== "Tab") return;
    const items = focusables();
    const index = items.indexOf(document.activeElement as HTMLElement);
    if (!items.length) { event.preventDefault(); node.focus(); }
    else if (event.shiftKey && index <= 0) { event.preventDefault(); items[items.length - 1].focus(); }
    else if (!event.shiftKey && (index < 0 || index === items.length - 1)) { event.preventDefault(); items[0].focus(); }
  };
  node.addEventListener("keydown", trap);
  return { destroy() { node.removeEventListener("keydown", trap); if (previous?.isConnected) previous.focus(); } };
}
