// A library that cannot be opened because another C1 Studio window holds it.

/** The error redb gives for a library file another process (or window) has open. */
export const isLockError = (text: string | null | undefined): boolean =>
  !!text && /already open|cannot acquire lock|locked by another C1 Studio/i.test(text);

/** What to tell the user instead of (or beside) the raw error. */
export const LOCK_HELP =
  "This library is open in another C1 Studio window. Close that window, then press Try again.";

// A catalog whose writes wait for the program that holds it.

/** Writes wait because nobody can tell whether Capture One is open (WSL without Windows
 *  interop). The reason says so in words, `Capture One, which may be running (…)`, where
 *  a running one is named by its process (`CaptureOne.exe`). */
export const ownerUnknown = (by: string): boolean => /^capture one\b/i.test(by);

/** The footer's word on autosave waiting for `by`. */
export function waitingText(by: string, owner: string | null, noun: string): string {
  if (ownerUnknown(by)) return `autosave waits for ${by}`;
  if (/captureone|imgcore/i.test(by)) return `autosave waits for ${owner ?? by} to close`;
  return `autosave waits: the ${noun} is open in another program`;
}
