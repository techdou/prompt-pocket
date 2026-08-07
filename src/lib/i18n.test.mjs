import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  LANGUAGE_STORAGE_KEY,
  createTranslator,
  getStoredLanguage,
  isLanguage,
  nextLanguage,
  setStoredLanguage,
  translate,
} from "./i18n.ts";

function memoryStorage(initial = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem(key) {
      return values.has(key) ? values.get(key) : null;
    },
    setItem(key, value) {
      values.set(key, String(value));
    },
    removeItem(key) {
      values.delete(key);
    },
  };
}

describe("i18n language helpers", () => {
  it("accepts only supported language codes", () => {
    assert.equal(isLanguage("zh"), true);
    assert.equal(isLanguage("en"), true);
    assert.equal(isLanguage("fr"), false);
    assert.equal(isLanguage(undefined), false);
  });

  it("falls back to Chinese when stored language is missing or unsupported", () => {
    assert.equal(getStoredLanguage(memoryStorage()), "zh");
    assert.equal(
      getStoredLanguage(memoryStorage({ [LANGUAGE_STORAGE_KEY]: "fr" })),
      "zh",
    );
  });

  it("persists a supported language and clears unsupported values", () => {
    const storage = memoryStorage();

    setStoredLanguage(storage, "en");
    assert.equal(getStoredLanguage(storage), "en");

    setStoredLanguage(storage, "fr");
    assert.equal(storage.getItem(LANGUAGE_STORAGE_KEY), null);
    assert.equal(getStoredLanguage(storage), "zh");
  });

  it("switches between Chinese and English", () => {
    assert.equal(nextLanguage("zh"), "en");
    assert.equal(nextLanguage("en"), "zh");
  });

  it("translates fixed labels and interpolated copy", () => {
    assert.equal(translate("en", "app.searchPlaceholder"), "Search prompts...");
    assert.equal(translate("zh", "app.searchPlaceholder"), "搜索提示词...");
    assert.equal(
      createTranslator("en")("app.deleteConfirm", { title: "Example" }),
      'Delete "Example"? The file will be moved to the .trash backup folder.',
    );
  });

  it("falls back to the raw key when the key is missing in both languages", () => {
    // 不存在的 key：两种语言表都查不到 → 原样返回 key 本身，不抛错、不返回 undefined
    assert.equal(translate("zh", "app.__missing__"), "app.__missing__");
    assert.equal(translate("en", "app.__missing__"), "app.__missing__");
  });

  it("keeps unmatched placeholders as-is when values are missing", () => {
    // 插值缺参：{title} 没有对应值 → 占位符原样保留，方便排查文案漏配
    assert.equal(
      translate("en", "app.deleteConfirm"),
      'Delete "{title}"? The file will be moved to the .trash backup folder.',
    );
  });
});
