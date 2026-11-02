// Lecture sécurisée de bout en bout (ADR-004, « A+ ») contre Dovecot, avec un
// serveur canari sur le réseau de test :
// - zéro requête vers le canari (images, CSS, scripts, formulaires,
//   redirections du message piégé) ;
// - aucun script du mail exécuté, seul l'agent est présent ;
// - IPC Tauri inaccessible depuis mailview:// (origine opaque, CSP) ;
// - un lien javascript: n'est jamais ouvert ;
// - les touches frappées dans le corps du message sont renvoyées à
//   l'interface (j : conversation suivante).
import { $, browser, expect } from "@wdio/globals";
import {
  addTestAccount,
  address,
  appendMessages,
  canaryHits,
  removeTestAccount,
} from "./helpers.mjs";

const CANARY = "canary.norelly.test";

const hostile = `From: Attaquant <attaquant@norelly.test>
To: ${address}
Subject: Message piégé
Date: Mon, 2 Nov 2026 12:00:00 +0000
Message-ID: <piege@norelly.test>
MIME-Version: 1.0
Content-Type: text/html; charset=utf-8

<html><head>
<meta http-equiv="refresh" content="0;url=http://${CANARY}/refresh">
<link rel="stylesheet" href="http://${CANARY}/lien.css">
<base href="http://${CANARY}/">
<style>@import url(http://${CANARY}/import.css); body { background: url(http://${CANARY}/fond.png) }</style>
<script>fetch("http://${CANARY}/script"); document.body.dataset.pwned = "script";</script>
</head><body onload="fetch('http://${CANARY}/onload')">
<p>Contenu du message piégé</p>
<img src="http://${CANARY}/pixel.gif" width="1" height="1">
<img src="x" onerror="fetch('http://${CANARY}/onerror'); document.body.dataset.pwned = 'onerror'">
<img src="relatif.png" srcset="http://${CANARY}/srcset.png 2x">
<div style="background-image: url('http://${CANARY}/div.png')">fond</div>
<iframe src="http://${CANARY}/iframe"></iframe>
<object data="http://${CANARY}/objet"></object>
<video poster="http://${CANARY}/poster.png" src="http://${CANARY}/video.mp4"></video>
<svg><image href="http://${CANARY}/svg.png"></image></svg>
<form action="http://${CANARY}/formulaire"><input name="q" value="x"><button>Envoyer</button></form>
<p><a href="javascript:fetch('http://${CANARY}/javascript')">lien javascript</a></p>
</body></html>
`;

const ordinary = `From: Collègue <collegue@norelly.test>
To: ${address}
Subject: Message ordinaire
Date: Mon, 2 Nov 2026 09:00:00 +0000
Message-ID: <ordinaire@norelly.test>
MIME-Version: 1.0
Content-Type: text/plain; charset=utf-8

Bonjour, ceci est un message ordinaire.
`;

async function intoMessageFrame() {
  const frame = $('iframe[title="Contenu du message"]');
  await frame.waitForDisplayed({ timeout: 30000 });
  await browser.switchToFrame(frame);
}

describe("lecture sécurisée", () => {
  before(async () => {
    await appendMessages([hostile, ordinary]);
    expect(await canaryHits()).toEqual([]);
  });

  it("synchronise et affiche la boîte unifiée", async () => {
    await addTestAccount();
    await $("span=Message piégé").waitForDisplayed({ timeout: 90000 });
    await expect($("span=Message ordinaire")).toBeDisplayed();
  });

  it("rend le message piégé sans exécuter son code ni rien charger", async () => {
    await $("span=Message piégé").click();
    await expect($("h1=Message piégé")).toBeDisplayed({ wait: 30000 });
    await expect($("span*=Images distantes bloquées")).toBeDisplayed();

    await intoMessageFrame();
    await $("p=Contenu du message piégé").waitForDisplayed({ timeout: 30000 });
    const probe = await browser.execute(() => ({
      scripts: document.scripts.length,
      tauriInternals: typeof window.__TAURI_INTERNALS__,
      tauri: typeof window.__TAURI__,
      origin: window.origin,
      pwned: document.body.dataset.pwned ?? null,
      frames: document.querySelectorAll(
        "iframe, object, embed, form, input, base, link, meta[http-equiv]",
      ).length,
    }));
    expect(probe).toEqual({
      scripts: 1,
      tauriInternals: "undefined",
      tauri: "undefined",
      origin: "null",
      pwned: null,
      frames: 0,
    });
  });

  it("n'offre aucun accès à l'IPC ni au réseau depuis mailview://", async () => {
    const attempts = await browser.executeAsync((canary, done) => {
      const attempt = (url, init) =>
        fetch(url, init).then(
          () => "réussi",
          () => "refusé",
        );
      Promise.all([
        attempt("ipc://localhost/list_accounts", { method: "POST", body: "{}" }),
        attempt("http://ipc.localhost/list_accounts", { method: "POST", body: "{}" }),
        attempt(`http://${canary}/fetch`),
      ]).then(done);
    }, CANARY);
    expect(attempts).toEqual(["refusé", "refusé", "refusé"]);
  });

  it("n'ouvre jamais un lien javascript:", async () => {
    await $("a=lien javascript").click();
    // Même relayé tel quel par un agent compromis, le schéma est refusé en Rust.
    await browser.execute((canary) => {
      window.parent.postMessage(
        { type: "norelly:link", href: `javascript:fetch('http://${canary}/postmessage')` },
        "*",
      );
    }, CANARY);
    await browser.switchToParentFrame();
    await expect($("p*=Lien non ouvert")).toBeDisplayed({ wait: 10000 });
  });

  it("renvoie les touches du corps du message à l'interface", async () => {
    await intoMessageFrame();
    await $("p=Contenu du message piégé").click();
    await browser.keys("j");
    await browser.switchToParentFrame();
    await expect($("h1=Message ordinaire")).toBeDisplayed({ wait: 20000 });
    await browser.keys("Escape");
    await expect($("h1=Message ordinaire")).not.toBeDisplayed({ wait: 10000 });
  });

  it("n'a envoyé aucune requête au serveur canari", async () => {
    await browser.pause(2000);
    expect(await canaryHits()).toEqual([]);
  });

  after(async () => {
    await browser.switchToParentFrame().catch(() => undefined);
    await removeTestAccount();
  });
});
