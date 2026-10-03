import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { extractVariables, applyVariables } from "./variables.ts";

describe("extractVariables - 占位符提取", () => {
  it("无占位符返回空数组", () => {
    assert.deepEqual(extractVariables("普通提示词，没有变量"), []);
    assert.deepEqual(extractVariables(""), []);
  });

  it("提取单个变量", () => {
    assert.deepEqual(extractVariables("翻译成 {{目标语言}}"), ["目标语言"]);
  });

  it("提取多个变量并保持首次出现顺序", () => {
    const body = "把 {{b}} 翻译成 {{a}}，风格 {{风格}}";
    assert.deepEqual(extractVariables(body), ["b", "a", "风格"]);
  });

  it("同名变量去重", () => {
    assert.deepEqual(extractVariables("{{a}} 和 {{a}} 与 {{ a }}"), ["a"]);
  });

  it("容忍变量名两侧空白", () => {
    assert.deepEqual(extractVariables("{{  语言  }}"), ["语言"]);
  });

  it("空或纯空白占位符被忽略", () => {
    assert.deepEqual(extractVariables("{{}} 和 {{   }}"), []);
  });

  it("单花括号不是占位符", () => {
    assert.deepEqual(extractVariables("{a} 和 {b}"), []);
  });

  it("未闭合花括号不是占位符", () => {
    assert.deepEqual(extractVariables("{{a 和 a} 结尾"), []);
    assert.deepEqual(extractVariables("{a}} 开头"), []);
  });

  it("变量值替换进 prompt 后再提取可为空（替换幂等链路）", () => {
    assert.deepEqual(extractVariables(applyVariables("x {{a}} y", { a: "1" })), []);
  });
});

describe("applyVariables - 变量替换", () => {
  it("替换已填写的变量", () => {
    assert.equal(
      applyVariables("翻译成 {{目标语言}}", { 目标语言: "Python" }),
      "翻译成 Python",
    );
  });

  it("同名变量全部替换", () => {
    assert.equal(applyVariables("{{a}}-{{a}}", { a: "X" }), "X-X");
  });

  it("留空的变量保留原样", () => {
    assert.equal(applyVariables("{{a}} {{b}}", { b: "1" }), "{{a}} 1");
  });

  it("未提供的变量保留原样（含原空白格式）", () => {
    assert.equal(applyVariables("x {{ a }} y", {}), "x {{ a }} y");
  });

  it("无占位符的文本原样返回", () => {
    assert.equal(applyVariables("plain text", { a: "1" }), "plain text");
  });

  it("替换值为空字符串时同样视为已填写（清空占位符）", () => {
    assert.equal(applyVariables("x {{a}} y", { a: "" }), "x  y");
  });

  it("替换值本身含花括号时不被二次展开", () => {
    assert.equal(applyVariables("{{a}}", { a: "{{b}}" }), "{{b}}");
  });
});
