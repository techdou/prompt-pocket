export interface Draft { title: string; body: string; category: string; copy_mode: string }
export function draftChanged(a: Draft, b: Draft): boolean {
  return a.title !== b.title || a.body !== b.body || a.category !== b.category || a.copy_mode !== b.copy_mode;
}
export function createRequestGate() {
  let revision = 0;
  let currentPath: string | null = null;
  return {
    begin(path: string) { currentPath = path; return ++revision; },
    accepts(token: number, path: string) { return token === revision && path === currentPath; },
    invalidate() { currentPath = null; revision++; },
  };
}
