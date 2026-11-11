// A dialog's backdrop closes it on a click that starts and ends on the backdrop itself.
// A press and a release on different elements send their click to the element holding
// both: a drag from inside the dialog (selecting a field's text) released outside it, or
// one from outside released inside, clicks the backdrop. Neither is a click on it.

/** Handlers for a backdrop: spread them on it. `close` runs on a click wholly on it. */
export function backdrop(close: () => void) {
  let down = false;
  let up = false;
  return {
    onpointerdown: (e: PointerEvent) => {
      down = e.target === e.currentTarget;
      up = false;
    },
    onpointerup: (e: PointerEvent) => {
      up = e.target === e.currentTarget;
    },
    onclick: (e: MouseEvent) => {
      const here = down && up && e.target === e.currentTarget;
      down = up = false;
      if (here) close();
    },
  };
}
