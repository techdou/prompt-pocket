// CDN 资源加载层：动态 import ESM / 注入 script / 注入 CSS。
// 统一带超时与失败清理，失败抛 RemoteLoadError，上层据此刻意降级。

/** 加载失败统一错误类型，上层 catch 后降级显示源码 */
export class RemoteLoadError extends Error {
  constructor(
    public readonly lib: string,
    message: string,
  ) {
    super(`[${lib}] ${message}`);
    this.name = "RemoteLoadError";
  }
}

// 模块缓存：同一 URL 多次调用只发一次请求；失败时清除以便重试
const moduleCache = new Map<string, Promise<unknown>>();
const scriptCache = new Map<string, Promise<unknown>>();
const cssLoaded = new Set<string>();

/**
 * 动态 import 一个 ESM 模块（mermaid 用）。
 * 先 fetch 探测连通性，再 import()，避免某些 webview 吞掉 import 错误。
 */
export async function importEsm<T = unknown>(
  url: string,
  lib: string,
  timeoutMs = 15_000,
): Promise<T> {
  if (moduleCache.has(url)) return moduleCache.get(url) as Promise<T>;

  const p = (async () => {
    const controller = new AbortController();
    const timer = setTimeout(() => controller.abort(), timeoutMs);
    try {
      const probe = await fetch(url, {
        method: "GET",
        signal: controller.signal,
        mode: "cors",
      });
      if (!probe.ok) throw new RemoteLoadError(lib, `HTTP ${probe.status}`);
      const mod = await import(/* @vite-ignore */ url);
      return (mod.default ?? mod) as T;
    } catch (e) {
      moduleCache.delete(url);
      if (e instanceof RemoteLoadError) throw e;
      throw new RemoteLoadError(
        lib,
        e instanceof Error ? e.message : String(e),
      );
    } finally {
      clearTimeout(timer);
    }
  })();

  moduleCache.set(url, p);
  return p as Promise<T>;
}

/**
 * 注入 <script src> 加载 UMD bundle（katex / highlight.js），返回挂载到 window 的全局对象。
 */
export function loadScript<T = unknown>(
  url: string,
  lib: string,
  globalKey: string,
  timeoutMs = 15_000,
): Promise<T> {
  if (scriptCache.has(url)) return scriptCache.get(url) as Promise<T>;

  const p = new Promise<T>((resolve, reject) => {
    const el = document.createElement("script");
    el.src = url;
    el.async = true;
    const timer = setTimeout(() => {
      el.remove();
      scriptCache.delete(url);
      reject(new RemoteLoadError(lib, `加载超时 (${timeoutMs}ms)`));
    }, timeoutMs);

    el.onload = () => {
      clearTimeout(timer);
      const g = (window as Record<string, unknown>)[globalKey];
      if (!g) {
        scriptCache.delete(url);
        reject(new RemoteLoadError(lib, `全局对象 window.${globalKey} 不存在`));
        return;
      }
      resolve(g as T);
    };
    el.onerror = () => {
      clearTimeout(timer);
      el.remove();
      scriptCache.delete(url);
      reject(new RemoteLoadError(lib, "网络/CSP/404 失败"));
    };
    document.head.appendChild(el);
  });

  scriptCache.set(url, p);
  return p;
}

/**
 * 注入 <link rel="stylesheet">，幂等。
 * CSS 加载失败不致命（代码仍可渲染，只是没样式），仅 console.warn。
 */
export function loadCss(href: string): void {
  if (cssLoaded.has(href)) return;
  cssLoaded.add(href);
  const link = document.createElement("link");
  link.rel = "stylesheet";
  link.href = href;
  link.onerror = () => {
    console.warn(`[loadRemote] CSS 加载失败: ${href}`);
    cssLoaded.delete(href);
  };
  document.head.appendChild(link);
}
