// A dialog's backdrop closes it on a click that starts and ends on the backdrop itself.
// A drag that starts inside the dialog (selecting a field's text) and ends outside it
// sends its click to the backdrop too: that one is not a click on it.

/** Handlers for a backdrop: spread them on it. `close` runs on a click wholly on it. */
export function backdrop(close: () => void) {
  let downHere = false;
  return {
    onpointerdown: (e: PointerEvent) => {
      downHere = e.target === e.currentTarget;
    },
    onclick: (e: MouseEvent) => {
      const here = downHere && e.target === e.currentTarget;
      downHere = false;
      if (here) close();
    },
  };
}
