// Faux serveur CardDAV des tests end-to-end (ADR-014), sur le réseau
// interne : HTTPS sur le port 443 (certificat signé par une autorité de
// test limitée à carddav.norelly.test), identifiants en HTTP Basic
// (utilisateur « norelly », mot de passe NORELLY_E2E_PASSWORD). Même
// chemin de découverte qu'un serveur réel : /.well-known/carddav redirige
// vers /dav/, principal /dav/principals/norelly/, dossier
// /dav/books/norelly/, carnet contacts/. Un carnet de départ porte une
// fiche, « Camille Durand ». Le port de contrôle 8080 (HTTP, lu par les
// scénarios et jamais par l'application) rend les fiches du carnet.
import fs from "node:fs";
import http from "node:http";
import https from "node:https";

const user = "norelly";
const password = process.env.NORELLY_E2E_PASSWORD ?? "";
const expected = `Basic ${Buffer.from(`${user}:${password}`).toString("base64")}`;
const home = `/dav/books/${user}/`;
const book = `${home}contacts/`;
const principal = `/dav/principals/${user}/`;

let version = 0;
/** Fiches du carnet, par chemin. */
const cards = new Map();
function put(path, data) {
  version += 1;
  const etag = `"v${version}"`;
  cards.set(path, { etag, data });
  return etag;
}
put(
  `${book}camille.vcf`,
  [
    "BEGIN:VCARD",
    "VERSION:3.0",
    "UID:camille",
    "FN:Camille Durand",
    "N:Durand;Camille;;;",
    "EMAIL;TYPE=INTERNET:camille.durand@norelly.test",
    "TEL;TYPE=CELL:06 11 22 33 44",
    "BDAY:19850412",
    "END:VCARD",
    "",
  ].join("\r\n"),
);

const escape = (text) => text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
const response = (href, props) =>
  `<d:response><d:href>${escape(href)}</d:href><d:propstat><d:prop>${props}</d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>`;
const multistatus = (body) =>
  `<?xml version="1.0" encoding="utf-8"?><d:multistatus xmlns:d="DAV:" xmlns:card="urn:ietf:params:xml:ns:carddav">${body}</d:multistatus>`;
const bookProps =
  "<d:resourcetype><d:collection/><card:addressbook/></d:resourcetype><d:displayname>Contacts</d:displayname>";

function reply(res, status, body = "", headers = {}) {
  res.writeHead(status, { "Content-Length": Buffer.byteLength(body), ...headers });
  res.end(body);
}

function handle(req, res, body) {
  if (req.headers.authorization !== expected) {
    return reply(res, 401, "", { "WWW-Authenticate": 'Basic realm="norelly-e2e"' });
  }
  const path = decodeURI(new URL(req.url, "https://carddav.norelly.test").pathname);
  const xml = (text) =>
    reply(res, 207, multistatus(text), { "Content-Type": "application/xml; charset=utf-8" });
  const depth = req.headers.depth ?? "0";
  switch (req.method) {
    case "PROPFIND":
      if (path === "/.well-known/carddav") return reply(res, 301, "", { Location: "/dav/" });
      if (path === "/dav/" || path === "/")
        return xml(
          response(
            path,
            `<d:current-user-principal><d:href>${principal}</d:href></d:current-user-principal><d:resourcetype><d:collection/></d:resourcetype>`,
          ),
        );
      if (path === principal)
        return xml(
          response(
            path,
            `<card:addressbook-home-set><d:href>${home}</d:href></card:addressbook-home-set>`,
          ),
        );
      if (path === home)
        return xml(
          response(home, "<d:resourcetype><d:collection/></d:resourcetype>") +
            (depth === "1" ? response(book, bookProps) : ""),
        );
      if (path === book) {
        let text = response(book, bookProps);
        if (depth === "1")
          for (const [href, card] of cards)
            text += response(href, `<d:getetag>${escape(card.etag)}</d:getetag><d:resourcetype/>`);
        return xml(text);
      }
      return reply(res, 404);
    case "REPORT": {
      if (path !== book) return reply(res, 404);
      let text = "";
      for (const match of body.matchAll(/href>([^<]+)</g)) {
        const href = match[1].replace(/&amp;/g, "&");
        const card = cards.get(href);
        text += card
          ? response(
              href,
              `<d:getetag>${escape(card.etag)}</d:getetag><card:address-data>${escape(card.data)}</card:address-data>`,
            )
          : `<d:response><d:href>${escape(href)}</d:href><d:status>HTTP/1.1 404 Not Found</d:status></d:response>`;
      }
      return xml(text);
    }
    case "PUT": {
      if (!path.startsWith(book) || path === book) return reply(res, 404);
      const current = cards.get(path);
      const ifMatch = req.headers["if-match"];
      const create = req.headers["if-none-match"] === "*";
      if (create ? current !== undefined : current === undefined || current.etag !== ifMatch) {
        return reply(res, 412);
      }
      return reply(res, create ? 201 : 204, "", { ETag: put(path, body) });
    }
    case "DELETE": {
      const current = cards.get(path);
      if (current === undefined) return reply(res, 404);
      if (current.etag !== req.headers["if-match"]) return reply(res, 412);
      cards.delete(path);
      return reply(res, 204);
    }
    default:
      return reply(res, 405);
  }
}

https
  .createServer(
    { cert: fs.readFileSync("/certs/tls.crt"), key: fs.readFileSync("/certs/tls.key") },
    (req, res) => {
      const chunks = [];
      req.on("data", (c) => chunks.push(c));
      req.on("end", () => handle(req, res, Buffer.concat(chunks).toString("utf8")));
    },
  )
  .listen(443);

http
  .createServer((req, res) => {
    const body = JSON.stringify([...cards.values()].map((c) => c.data));
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(body);
  })
  .listen(8080);

console.log("serveur CardDAV prêt");
