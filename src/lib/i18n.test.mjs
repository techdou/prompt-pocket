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
      'Delete "Example"? This cannot be undone.',
    );
  });
});
