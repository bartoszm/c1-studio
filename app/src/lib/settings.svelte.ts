// The app's own preferences, the same for every catalog: kept in the webview's storage
// next to the sidebar's layout.

import { neighbourGap, SESSION_GAP, sessionGap } from "./around";
import { mergeSections, type SectionPref } from "./sections";

export type Settings = {
  /** The sidebar's sections, in order, and which are hidden. */
  sections: SectionPref[];
  /** Map tiles, place search and place hints from the internet; off, nothing is asked. */
  onlineMap: boolean;
  /** The map also shows photos that have only a place (city, country), at that place. */
  approxPlaces: boolean;
  /** Photos given a position get their empty place fields filled from it, without asking. */
  fillPlaces: boolean;
  /** A gap longer than this (minutes) between two photos ends a shoot: "shoot" under Captured. */
  sessionGap: number;
  /** Placing photos from their neighbours in time: what was chosen last. */
  neighbours: Neighbours;
};

/** Positions from neighbours with GPS taken at most `positionGap` minutes apart; places
 *  from a neighbour within `placeGap` minutes. */
export type Neighbours = { positions: boolean; positionGap: number; places: boolean; placeGap: number };

function neighbours(saved: Partial<Neighbours> | undefined): Neighbours {
  return {
    positions: saved?.positions !== false,
    positionGap: neighbourGap(saved?.positionGap) ?? 30,
    places: saved?.places !== false,
    placeGap: neighbourGap(saved?.placeGap) ?? 15,
  };
}

function load(): Settings {
  let saved: Partial<Settings> = {};
  try {
    saved = JSON.parse(localStorage.getItem("settings") ?? "{}") ?? {};
  } catch {
    // Not saved, or not readable: the defaults.
  }
  return {
    sections: mergeSections(saved.sections),
    onlineMap: saved.onlineMap !== false,
    approxPlaces: saved.approxPlaces !== false,
    fillPlaces: saved.fillPlaces === true,
    sessionGap: typeof saved.sessionGap === "number" ? (sessionGap(saved.sessionGap) ?? SESSION_GAP) : SESSION_GAP,
    neighbours: neighbours(saved.neighbours),
  };
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
