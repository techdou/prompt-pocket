import { marked, type Token, type Tokens } from "marked";

/** Convert parsed Markdown without executing/rendering HTML or loading a DOM. */
export function markdownToPlain(source: string): string {
  function inline(tokens: Token[]): string {
    return tokens.map((token): string => {
      if (token.type === "image") return token.text;
      if (token.type === "br") return "\n";
      if ("tokens" in token && Array.isArray(token.tokens)) return inline(token.tokens as Token[]);
      if ("text" in token && typeof token.text === "string") return token.text;
      return token.raw ?? "";
    }).join("");
  }
  function blocks(tokens: Token[]): string {
    return tokens.filter((token) => token.type !== "space").map((token): string => {
      if (token.type === "hr") return "---";
      if (token.type === "code") return token.text;
      if (token.type === "blockquote") return blocks(token.tokens ?? []);
      if (token.type === "list") return (token as Tokens.List).items.map((item, index) => `${token.ordered ? `${Number(token.start) + index}.` : "•"} ${blocks(item.tokens)}`).join("\n");
      if (token.type === "table") return [(token as Tokens.Table).header, ...(token as Tokens.Table).rows].map((row) => row.map((cell) => inline(cell.tokens)).join("\t")).join("\n");
      if ("tokens" in token && Array.isArray(token.tokens)) return inline(token.tokens as Token[]);
      return "text" in token && typeof token.text === "string" ? token.text : token.raw;
    }).join("\n\n");
  }
  return blocks(marked.lexer(source)).trim();
}
