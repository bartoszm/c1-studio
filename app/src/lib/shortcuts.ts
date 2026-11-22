// Every key the app answers to, in one table: the `?` list and the hints in menus are read
// from here. The keys themselves are handled in App.svelte's `onkey`: a key added there
// goes here too, and in the guide's Keyboard section (a test checks the two agree).

export type Place = "Photos" | "Viewer" | "Compare" | "Cull" | "Anywhere";

export type Shortcut = {
  /** As shown: `Ctrl+K`, `0–5`. Several ways to do it are split by " · ". */
  keys: string;
  what: string;
  where: Place;
};

export const SHORTCUTS: Shortcut[] = [
  { keys: "Arrows · Page Up/Down · Home · End", what: "Move the focus; with Shift, pick a range", where: "Photos" },
  { keys: "Click · Ctrl+click · Shift+click", what: "Focus one; add or remove one; add a range", where: "Photos" },
  { keys: "Ctrl+A", what: "Pick the whole result", where: "Photos" },
  { keys: "Ctrl+I", what: "Invert the pick: the other photos of the result", where: "Photos" },
  { keys: "Ctrl+Shift+A · Esc", what: "Back to the focused photo alone", where: "Photos" },
  { keys: "Enter · double-click", what: "Open the viewer", where: "Photos" },
  { keys: "t · Shift+T", what: "Show the focused photo's shoot; pick it among those shown", where: "Photos" },
  { keys: "v", what: "Show every variant of the focused photo's image", where: "Photos" },
  { keys: "c", what: "Compare the picked photos", where: "Photos" },
  { keys: "b", what: "Choose the best of each series or duplicate group among the photos shown", where: "Photos" },
  { keys: "Ctrl+wheel · Ctrl+plus · Ctrl+minus · Ctrl+0", what: "Bigger, smaller, default tiles", where: "Photos" },
  { keys: "0–5", what: "Rating (0 clears)", where: "Anywhere" },
  { keys: "- · + · *", what: "Red, green, yellow tag", where: "Anywhere" },
  { keys: "h", what: "Hide the picked photos; with hidden photos shown, unhide them", where: "Anywhere" },
  { keys: "Delete", what: "Move to the trash; in an album, take out of it", where: "Anywhere" },
  { keys: "Ctrl+Delete", what: "Move to the trash, in an album too", where: "Anywhere" },
  { keys: "Backspace", what: "In an album, take the picked photos out of it", where: "Photos" },
  { keys: "Ctrl+K", what: "Add keywords (opens the details panel)", where: "Anywhere" },
  { keys: "Ctrl+F", what: "Go to the search box", where: "Photos" },
  { keys: "i", what: "Show or hide the details panel", where: "Anywhere" },
  { keys: "Ctrl+C · Ctrl+Shift+C", what: "Copy… from the focused photo", where: "Anywhere" },
  { keys: "Ctrl+V · Ctrl+Alt+V · Ctrl+Shift+V", what: "Paste (overwrite), merge, Paste… (choose)", where: "Anywhere" },
  { keys: "Ctrl+Shift+S", what: "Copy the focused photo's fields onto the other picked photos", where: "Anywhere" },
  { keys: "Ctrl+Z · Ctrl+Y · Ctrl+Shift+Z", what: "Undo; redo", where: "Anywhere" },
  { keys: "Alt+← · Alt+→", what: "Back to the filter before, forward again", where: "Anywhere" },
  { keys: "? · F1", what: "This list", where: "Anywhere" },
  { keys: "← · →", what: "Previous, next photo", where: "Viewer" },
  { keys: "g · Shift+G", what: "Next, previous guide over the photo (thirds, golden ratio…)", where: "Viewer" },
  { keys: "o", what: "Turn the golden triangles or the spiral", where: "Viewer" },
  { keys: "Esc", what: "Close the viewer, a dialog or a menu", where: "Viewer" },
  { keys: "Wheel · z", what: "Zoom; 1:1 and back", where: "Compare" },
  { keys: "Enter", what: "Keep the active photo (and the ones marked), deal with the rest, go to the next group", where: "Cull" },
  { keys: "k", what: "Mark the active photo to keep it too, or unmark it", where: "Cull" },
  { keys: "↑ · ↓", what: "Previous, next group, leaving this one as it is", where: "Cull" },
  { keys: "← · → · Tab", what: "Previous, next photo of the group", where: "Cull" },
  { keys: "g", what: "Go to the suggested photo", where: "Cull" },
  { keys: "r · Shift+R", what: "Have the local model look at this group; at every group, in the background", where: "Cull" },
];

export const PLACES: Place[] = ["Photos", "Anywhere", "Viewer", "Compare", "Cull"];

/** What the photo menu's items do, as the table above says it (its start): their key hints
 *  are found by these, and a test checks each still finds one. */
export const MENU_ITEMS = {
  keywords: "Add keywords",
  hide: "Hide the picked photos",
  compare: "Compare the picked photos",
  copy: "Copy… from",
  paste: "Paste (overwrite)",
  takeOut: "Move to the trash; in an album, take out of it",
  trash: "Move to the trash; in an album",
  trashInAlbum: "Move to the trash, in an album too",
} as const;

/** The keys of the shortcut that does `what` (its start), for a menu's hint. */
export function keysFor(what: string): string {
  return SHORTCUTS.find((s) => s.what.startsWith(what))?.keys.split(" · ")[0] ?? "";
}
