const english = new URLSearchParams(location.search).get("lang") === "en";

if (english) {
  document.documentElement.lang = "en";
  document.title = "Prompt Pocket · Desktop prompt library";
  document.querySelectorAll("[data-en]").forEach((el) => { el.textContent = el.dataset.en; });
  document.querySelectorAll("[data-alt-en]").forEach((el) => { el.alt = el.dataset.altEn; });
  const languageLink = document.querySelector("#language");
  languageLink.textContent = "简体中文";
  languageLink.href = "./";
  languageLink.lang = "zh-CN";
  languageLink.hreflang = "zh-CN";
}

const views = {
  manage: {
    alt: ["双栏整理窗口", "Two-column library and editor"],
    description: ["按分类、收藏和最近使用整理库。在列表旁预览 Markdown、公式和图表。", "Organize by category, pin favorites, or find recent prompts. Preview Markdown, formulas, and diagrams alongside the list."],
  },
  template: {
    alt: ["模板变量填写窗口", "Prompt template variable editor"],
    description: ["用带默认值的变量复用同一条提示词。填写后用 Ctrl+Enter 应用，源模板保持原样。", "Reuse a prompt with named variables and defaults. Press Ctrl+Enter to apply your values. The original template stays intact."],
  },
};
const buttons = document.querySelectorAll("[data-view]");
const preview = document.querySelector("#product-view");
const detail = document.querySelector("#view-detail");
const languageIndex = english ? 1 : 0;
preview.alt = views.manage.alt[languageIndex];

buttons.forEach((button) => {
  button.addEventListener("click", () => {
    const name = button.dataset.view;
    buttons.forEach((item) => item.setAttribute("aria-pressed", String(item === button)));
    preview.src = "assets/" + name + ".png";
    preview.alt = views[name].alt[languageIndex];
    detail.textContent = views[name].description[languageIndex];
  });
});
