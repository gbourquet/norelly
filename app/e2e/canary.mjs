// Serveur canari des tests end-to-end (ADR-004) : compte toute connexion
// reçue sur les ports 80 (HTTP) et 443 (n'importe quel protocole). Le
// compteur est lu sur le port de contrôle 8080 par les scénarios (et
// jamais par l'application) : il doit rester à zéro.
import http from "node:http";
import net from "node:net";

const hits = [];

http
  .createServer((req, res) => {
    hits.push(`http ${req.method} ${req.url}`);
    res.writeHead(200, { "Content-Type": "image/gif" });
    res.end();
  })
  .listen(80);

net
  .createServer((socket) => {
    hits.push("tcp 443");
    socket.destroy();
  })
  .listen(443);

http
  .createServer((req, res) => {
    res.writeHead(200, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ hits }));
  })
  .listen(8080);

console.log("canari prêt");
