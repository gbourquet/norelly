// Messages de l'agent du rendu (iframe mailview://, origine opaque) : seule
// surface de communication entre le mail et l'interface (ADR-004). Tout
// message est validé ici (source, origine, forme, tailles) ; les décisions
// (ouvrir un lien, par exemple) restent en Rust.

import type { Shortcut } from "./commands/shortcut";
import { shortcutFromEvent } from "./commands/shortcut";

export type AgentMessage =
  | { type: "height"; height: number }
  | { type: "link"; href: string }
  | { type: "key"; shortcut: Shortcut };

/** Hauteur maximale acceptée pour un message (px). */
export const MAX_FRAME_HEIGHT = 200_000;
/** Longueur maximale d'un lien transmis. */
export const MAX_LINK_LENGTH = 2048;
/** Longueur maximale d'un nom de touche (`ArrowDown`, `F12`…). */
const MAX_KEY_LENGTH = 16;

interface IncomingEvent {
  source: unknown;
  origin: string;
  data: unknown;
}

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/**
 * Message valide de l'agent de `frame`, ou `null`. Un document à origine
 * opaque (iframe sandbox sans allow-same-origin) a pour origine « null » ;
 * toute autre origine, ou une autre fenêtre émettrice, est ignorée.
 */
export function parseAgentMessage(
  event: IncomingEvent,
  frame: Window | null | undefined,
): AgentMessage | null {
  if (frame === null || frame === undefined || event.source !== frame) return null;
  if (event.origin !== "null") return null;
  const data = event.data;
  if (!isRecord(data) || typeof data["type"] !== "string") return null;
  switch (data["type"]) {
    case "norelly:height": {
      const h = data["height"];
      if (typeof h !== "number" || !Number.isFinite(h) || h < 0) return null;
      return { type: "height", height: Math.min(Math.ceil(h), MAX_FRAME_HEIGHT) };
    }
    case "norelly:link": {
      const href = data["href"];
      if (typeof href !== "string" || href.length === 0 || href.length > MAX_LINK_LENGTH)
        return null;
      return { type: "link", href };
    }
    case "norelly:key": {
      const key = data["key"];
      const flags = ["ctrl", "alt", "shift", "meta"].map((k) => data[k]);
      if (typeof key !== "string" || key.length === 0 || key.length > MAX_KEY_LENGTH) return null;
      if (!flags.every((f) => typeof f === "boolean")) return null;
      const [ctrlKey, altKey, shiftKey, metaKey] = flags as boolean[];
      return {
        type: "key",
        shortcut: shortcutFromEvent({
          key,
          ctrlKey: ctrlKey ?? false,
          altKey: altKey ?? false,
          shiftKey: shiftKey ?? false,
          metaKey: metaKey ?? false,
        }),
      };
    }
    default:
      return null;
  }
}

/** Adresse `mailview://` d'un message selon la plateforme. */
export function mailviewUrl(message: number, version = 0, windows = isWindows()): string {
  const base = windows ? "http://mailview.localhost/" : "mailview://localhost/";
  return `${base}${message}?v=${version}`;
}

function isWindows(): boolean {
  return typeof navigator !== "undefined" && /Windows/i.test(navigator.userAgent);
}
