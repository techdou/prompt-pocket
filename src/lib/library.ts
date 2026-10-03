export interface LibraryEntry { favorite: boolean; useCount: number; lastUsed: number }
export type Library = Record<string, LibraryEntry>;
type LibraryStorage = Pick<Storage, "getItem" | "setItem">;
const KEY = "prompt-pocket.library.v1";
const emptyEntry = (): LibraryEntry => ({ favorite: false, useCount: 0, lastUsed: 0 });
export function readLibrary(storage: Pick<LibraryStorage, "getItem"> | null): Library {
  try {
    const parsed: unknown = JSON.parse(storage?.getItem(KEY) ?? "{}");
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return {};
    return Object.fromEntries(Object.entries(parsed).filter(([, entry]) => entry &&
      typeof entry.favorite === "boolean" && Number.isSafeInteger(entry.useCount) && entry.useCount >= 0 &&
      Number.isFinite(entry.lastUsed) && entry.lastUsed >= 0));
  } catch { return {}; }
}
export function writeLibrary(storage: Pick<LibraryStorage, "setItem"> | null, library: Library): void {
  try { storage?.setItem(KEY, JSON.stringify(library)); } catch { /* Restricted storage leaves this session usable. */ }
}
export function toggleFavorite(library: Library, path: string): Library {
  const entry = Object.hasOwn(library, path) ? library[path] : emptyEntry();
  return { ...library, [path]: { ...entry, favorite: !entry.favorite } };
}
export function recordUse(library: Library, path: string, now = Date.now()): Library {
  const entry = Object.hasOwn(library, path) ? library[path] : emptyEntry();
  return { ...library, [path]: { ...entry, useCount: Math.min(Number.MAX_SAFE_INTEGER, entry.useCount + 1), lastUsed: now } };
}
export function moveLibraryEntry(library: Library, from: string, to: string): Library {
  if (from === to || !Object.hasOwn(library, from)) return library;
  const next = { ...library, [to]: library[from] };
  delete next[from];
  return next;
}
