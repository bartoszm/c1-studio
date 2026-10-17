// A library that cannot be opened because another C1 Studio window holds it.

/** The error redb gives for a library file another process (or window) has open. */
export const isLockError = (text: string | null | undefined): boolean =>
  !!text && /already open|cannot acquire lock/i.test(text);

/** What to tell the user instead of (or beside) the raw error. */
export const LOCK_HELP =
  "This library is open in another C1 Studio window. Close that window, then press Try again.";
