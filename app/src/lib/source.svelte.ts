// Which kind of source is open, for wording that names it ("Write to Capture One", "Write to
// the photo folders"). Set when the status is read; until then the wording is general.

import type { Wording } from "./api";
import { GENERAL } from "./format";

class OpenSource {
  wording = $state<Wording>(GENERAL);
  /** In a sentence: "Capture One", "the photo folders". */
  get word(): string {
    return this.wording.word;
  }
  /** As a label: "Capture One", "Photo folders". */
  get name(): string {
    return this.wording.name;
  }
  /** What holds the photos, after "the": "catalog", "library". */
  get noun(): string {
    return this.wording.noun;
  }
  /** The program to close before the app writes ("Capture One"), if any. */
  get owner(): string | null {
    return this.wording.owner;
  }
}

export const source = new OpenSource();
