// The app's trash: photos deleted here are left out of every view but the trash's, until
// restored, or until the trash is emptied and their source deletes them for good.
import type { Filter, Removal } from "./api";

/** The filter as the views ask it: only the trash's photos while it is shown, hidden ones
 *  too (a photo hidden, then deleted, is emptied with the rest: it must show to be restored). */
export const withBin = (f: Filter, inBin: boolean): Filter => (inBin ? { ...f, bin: "Only", hidden: "Include" } : f);

/** What a Delete key press does: moves the photos to the trash, takes them out of the album
 *  shown (Delete alone, in an album: Ctrl+Delete still trashes), or nothing (in the trash, or
 *  for a source that cannot delete). */
export function deleteAction(o: { ctrl: boolean; album: boolean; inBin: boolean; removes: Removal | undefined }): "bin" | "album" | "none" {
  if (o.album && !o.ctrl && !o.inBin) return "album";
  if (o.inBin || !o.removes || o.removes === "None") return "none";
  return "bin";
}

const photos = (n: number) => `${n.toLocaleString()} photo${n === 1 ? "" : "s"}`;

/** How the status line says it: "2 photos moved to Trash" / "1 photo restored". */
export const binNote = (n: number, binned: boolean) => `${photos(n)} ${binned ? "moved to Trash" : "restored"}`;

/** What deleting `n` photos for good does, in a sentence, for the dialog asking first. */
export function removalText(n: number, removes: Removal): string {
  switch (removes) {
    case "Files":
      return `The files of ${photos(n)} go to the Windows Recycle Bin, with their own .xmp sidecars. Their edits in C1 Studio are lost.`;
    case "MarksTrashed":
      return `${photos(n)} ${n === 1 ? "is" : "are"} marked deleted in Capture One and show${n === 1 ? "s" : ""} in its Trash; the files stay. Any other variants of the same images go too. A backup of the catalog is made first.`;
    case "None":
      return "This source cannot delete photos.";
  }
}
