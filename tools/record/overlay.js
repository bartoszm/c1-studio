// Injected by drive.mjs: what a recording adds to the page. The screencast has no mouse
// pointer, so this draws one that follows the (synthetic) mouse, with a ring on each
// click, and a caption band at the bottom. Nothing here takes pointer events.
(() => {
  if (window.__rec) return;
  const style = document.createElement("style");
  style.textContent = `
    #rec-pointer { position: fixed; left: 0; top: 0; z-index: 2147483647; pointer-events: none;
      width: 22px; height: 22px; margin: -2px 0 0 -2px; transition: opacity .2s; opacity: 0;
      filter: drop-shadow(0 1px 2px rgba(0,0,0,.6)); }
    .rec-ring { position: fixed; z-index: 2147483646; pointer-events: none; width: 34px; height: 34px;
      margin: -17px 0 0 -17px; border-radius: 50%; border: 3px solid #ffb648;
      animation: rec-ring .45s ease-out forwards; }
    @keyframes rec-ring { from { transform: scale(.3); opacity: 1 } to { transform: scale(1.3); opacity: 0 } }
    #rec-caption { position: fixed; left: 50%; bottom: 46px; transform: translateX(-50%);
      z-index: 2147483645; pointer-events: none; max-width: 80%;
      padding: 9px 18px; border-radius: 8px; background: rgba(18,18,20,.86); color: #fff;
      font: 500 17px/1.35 "Segoe UI", system-ui, sans-serif; letter-spacing: .01em;
      box-shadow: 0 4px 18px rgba(0,0,0,.35); transition: opacity .25s; opacity: 0; }
  `;
  document.head.append(style);

  const pointer = document.createElement("div");
  pointer.id = "rec-pointer";
  pointer.innerHTML = `<svg viewBox="0 0 22 22" width="22" height="22"><path d="M2 1.5 L2 17.5 L6.4 13.6 L9.4 20.2 L12.2 19 L9.3 12.5 L15.2 12.5 Z" fill="#fff" stroke="#111" stroke-width="1.3" stroke-linejoin="round"/></svg>`;
  const caption = document.createElement("div");
  caption.id = "rec-caption";
  document.body.append(pointer, caption);

  addEventListener("mousemove", (e) => {
    pointer.style.opacity = "1";
    pointer.style.transform = `translate(${e.clientX}px, ${e.clientY}px)`;
  }, true);
  addEventListener("mousedown", (e) => {
    const ring = document.createElement("div");
    ring.className = "rec-ring";
    ring.style.left = e.clientX + "px";
    ring.style.top = e.clientY + "px";
    document.body.append(ring);
    setTimeout(() => ring.remove(), 500);
  }, true);

  window.__rec = {
    caption(text) {
      if (text) caption.textContent = text;
      caption.style.opacity = text ? "1" : "0";
    },
  };
})();
