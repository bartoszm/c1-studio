// Small choices remembered in the browser between visits (a panel open, a tab chosen).
// The webview's storage may be off or full: then nothing is kept and the default stands.

/** The value kept under `key`, if it is one of `allowed`; else `fallback`. */
export function readChoice<T extends string>(key: string, allowed: readonly T[], fallback: T): T {
  try {
    const v = localStorage.getItem(key);
    return allowed.includes(v as T) ? (v as T) : fallback;
  } catch {
    return fallback;
  }
}

/** Keeps `value` under `key`, if it can. */
export function keepChoice(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Not kept: the default stands next time.
  }
}

/** A switch kept under `key`: off unless kept on. */
export const readFlag = (key: string): boolean => readChoice(key, ["1", "0"], "0") === "1";
export const keepFlag = (key: string, on: boolean): void => keepChoice(key, on ? "1" : "0");
