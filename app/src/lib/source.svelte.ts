// Which kind of source is open, for wording that names it ("Write to Capture One", "Write to
// the photo folders"). Set when the status is read; until then the wording is general.

import { sourceName, sourceWord } from "./format";

class OpenSource {
  provider = $state("");
  /** In a sentence: "Capture One", "the photo folders". */
  get word(): string {
    return sourceWord(this.provider);
  }
  /** As a label: "Capture One", "Photo folders". */
  get name(): string {
    return sourceName(this.provider);
  }
  get isCaptureOne(): boolean {
    return this.provider === "capture-one";
  }
}

export const source = new OpenSource();
