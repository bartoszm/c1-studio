// Variants of one image (Capture One keeps several versions of a photo): the grid marks them
// and can show one per image.

type Key = { key: string; repeat?: boolean; ctrlKey?: boolean; metaKey?: boolean; altKey?: boolean; shiftKey?: boolean };

/** `v` on a photo whose image has `count` variants: show them all. Held, it fires once;
 *  Caps Lock on, it still works. */
export function opensVariants(e: Key, count: number): boolean {
  return e.key.toLowerCase() === "v" && !e.repeat && !e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && count > 1;
}

/** What a cell's mark says when pointed at. */
export function markTitle(n: number, collapsed: boolean): string {
  const what = collapsed ? `One of ${n} variants of this image, the others collapsed` : `One of ${n} variants of this image`;
  return `${what}: click or press V to see them all`;
}
