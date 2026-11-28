const english = new URLSearchParams(location.search).get("lang") === "en";
const language = english ? "en" : "zh";

if (english) {
  document.documentElement.lang = "en";
  document.title = "Prompt Pocket · Desktop prompt library";
  document.querySelectorAll("[data-en]").forEach((element) => { element.textContent = element.dataset.en; });
  document.querySelectorAll("[data-label-en]").forEach((element) => { element.setAttribute("aria-label", element.dataset.labelEn); });
  const switchLanguage = document.querySelector("#language");
  switchLanguage.textContent = "简体中文";
  switchLanguage.href = "./";
  switchLanguage.lang = "zh-CN";
  switchLanguage.hreflang = "zh-CN";
}

const scenes = {
  "quick-search": {
    size: [1020, 750],
    zh: ["输入标题或正文的关键词，从结果里选一条。", "Windows 桌面版快速搜索「复盘」，选中项目复盘"],
    en: ["Search titles or content and choose a matching prompt.", "Windows quick launcher with the Project review prompt selected"],
  },
  template: {
    size: [1440, 960],
    zh: ["填入项目、周期和工作记录，右侧会预览本次内容。", "Windows 桌面版模板变量窗口，右侧预览填写后的项目复盘"],
    en: ["Fill in the project, period, and notes. The right pane previews the result.", "Windows template dialog showing variables and the filled Project review prompt"],
  },
  manage: {
    size: [1440, 960],
    zh: ["切到整理模式，在分类列表旁阅读和编辑 Markdown。", "Windows 桌面版双栏整理界面，预览项目复盘 Markdown"],
    en: ["Open the library view to read and edit Markdown alongside your categories.", "Windows two-column library with the Project review Markdown preview"],
  },
};

const buttons = [...document.querySelectorAll("[data-view][type=button]")];
const screen = document.querySelector("#product-view");
const frame = document.querySelector("#capture-link");
const original = document.querySelector("#original-link");
const description = document.querySelector("#view-detail");
const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)");

function selectView(name, animate = false) {
  const scene = scenes[name];
  const source = "assets/" + name + (english ? "-en" : "") + ".png";
  buttons.forEach((button) => button.setAttribute("aria-pressed", String(button.dataset.view === name)));
  frame.dataset.view = name;
  frame.href = source;
  original.href = source;
  screen.width = scene.size[0];
  screen.height = scene.size[1];
  screen.alt = scene[language][1];
  screen.src = source;
  description.textContent = scene[language][0];
  screen.getAnimations().forEach((motion) => motion.cancel());
  if (animate && !reducedMotion.matches) {
    screen.decode().then(() => {
      if (frame.dataset.view === name && !reducedMotion.matches) {
        screen.animate([{ opacity: 0.65 }, { opacity: 1 }], {
          duration: 220, easing: "cubic-bezier(.18,.71,.21,1)",
        });
      }
    }).catch(() => {});
    description.getAnimations().forEach((motion) => motion.cancel());
    description.animate([{ opacity: 0.6, translate: "0 4px" }, { opacity: 1, translate: "0 0" }], {
      duration: 200, easing: "cubic-bezier(.18,.71,.21,1)",
    });
  }
}

buttons.forEach((button, index) => {
  button.addEventListener("click", () => selectView(button.dataset.view, true));
  button.addEventListener("keydown", (event) => {
    const offsets = { ArrowLeft: -1, ArrowUp: -1, ArrowRight: 1, ArrowDown: 1 };
    const offset = Object.hasOwn(offsets, event.key) ? offsets[event.key] : 0;
    if (!offset && event.key !== "Home" && event.key !== "End") return;
    event.preventDefault();
    const next = event.key === "Home" ? 0 : event.key === "End" ? buttons.length - 1
      : (index + offset + buttons.length) % buttons.length;
    buttons[next].focus();
    selectView(buttons[next].dataset.view, true);
  });
});

if (english) {
  document.querySelector("#file-label").textContent = "Reviews/01-Project review.md";
  const fragment = "# Project review\n\nUse the work notes for {{Project}} during {{Period|this month}} to write a review.\n\n## Work notes\n\n{{Notes}}";
  const fileCode = document.querySelector("#file-fragment");
  fileCode.replaceChildren();
  fragment.split(/(\{\{[^}]+\}\})/g).forEach((part) => {
    if (part.startsWith("{{")) {
      const mark = document.createElement("mark");
      mark.textContent = part;
      fileCode.append(mark);
    } else {
      fileCode.append(part);
    }
  });
}
selectView("quick-search");
