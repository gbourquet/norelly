// Agent de Norelly, seul script autorisé dans le rendu d'un mail
// (mailview://, origine opaque, CSP par empreinte SHA-256). Il ne fait
// que signaler au parent la hauteur du document, les clics sur les liens
// et les touches de raccourci. Toute décision est prise côté Rust.
(function () {
  "use strict";
  var post = function (message) {
    window.parent.postMessage(message, "*");
  };
  var lastHeight = -1;
  var report = function () {
    var height = Math.ceil(document.documentElement.scrollHeight);
    if (height !== lastHeight) {
      lastHeight = height;
      post({ type: "norelly:height", height: height });
    }
  };
  if (typeof ResizeObserver === "function") {
    new ResizeObserver(report).observe(document.documentElement);
  }
  window.addEventListener("load", report);
  document.addEventListener("DOMContentLoaded", report);
  document.addEventListener(
    "click",
    function (event) {
      var target = event.target;
      var link = target && target.closest ? target.closest("a[href], area[href]") : null;
      if (link) {
        event.preventDefault();
        post({ type: "norelly:link", href: String(link.getAttribute("href") || "") });
      }
    },
    true,
  );
  document.addEventListener(
    "keydown",
    function (event) {
      if (event.isComposing) return;
      post({
        type: "norelly:key",
        key: String(event.key),
        ctrl: event.ctrlKey,
        alt: event.altKey,
        shift: event.shiftKey,
        meta: event.metaKey,
      });
    },
    true,
  );
  // Formulaires : aucun envoi (ils sont retirés par l'assainisseur, défense
  // en profondeur).
  document.addEventListener(
    "submit",
    function (event) {
      event.preventDefault();
    },
    true,
  );
  report();
})();
