// The app's own preferences, the same for every catalog: kept in the webview's storage
// next to the sidebar's layout.

import { mergeSections, type SectionPref } from "./sections";

export type Settings = {
  /** The sidebar's sections, in order, and which are hidden. */
  sections: SectionPref[];
  /** Map tiles, place search and place hints from the internet; off, nothing is asked. */
  onlineMap: boolean;
};

function load(): Settings {
  let saved: Partial<Settings> = {};
  try {
    saved = JSON.parse(localStorage.getItem("settings") ?? "{}") ?? {};
  } catch {
    // Not saved, or not readable: the defaults.
  }
  return { sections: mergeSections(saved.sections), onlineMap: saved.onlineMap !== false };
}

export const settings = $state<Settings>(load());

$effect.root(() => {
  $effect(() => {
    try {
      localStorage.setItem("settings", JSON.stringify(settings));
    } catch {
      // Storage unavailable: the settings last until the app closes.
    }
  });
});
