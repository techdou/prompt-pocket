// Svelte action：挂载后自动聚焦输入框。
// 用 use:autofocus 而非 autofocus 属性——后者在 Svelte 里是编译期属性，
// 对条件渲染（{#if}）里后挂载的元素不生效。

import { tick } from "svelte";

export function autofocus(el: HTMLElement) {
  // 等过渡动画（fly/scale）开始后再聚焦，避免焦点被 transition 抢占
  void tick().then(() => el.focus());
}
