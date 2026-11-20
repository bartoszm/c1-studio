// The photo menu (right-click on photos): where it opens, and its album finder.

import type { Collection } from "./api";

type Size = { width: number; height: number };

/** Where a menu of `menu`'s size opens for a click at `at`: there, or moved left or up as
 *  far as it needs to stay `margin` inside the window. A menu taller than the window
 *  starts at the top (it scrolls: see `.photo-menu`). */
export function fitMenu(at: { x: number; y: number }, menu: Size, window: Size, margin = 8): { left: number; top: number } {
  return {
    left: Math.max(margin, Math.min(at.x, window.width - menu.width - margin)),
    top: Math.max(margin, Math.min(at.y, window.height - menu.height - margin)),
  };
}

/** The albums (not smart albums, groups or the system's) whose path holds `query`, at
 *  most `max` of them, in the catalog's order. */
export function findAlbums(collections: Collection[], query: string, max = 12): Collection[] {
  const q = query.trim().toLowerCase();
  return collections.filter((c) => c.kind === "Album" && !c.system && c.path.toLowerCase().includes(q)).slice(0, max);
}
