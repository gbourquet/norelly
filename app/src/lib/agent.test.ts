import { describe, expect, it } from "vitest";
import { MAX_FRAME_HEIGHT, mailviewUrl, parseAgentMessage } from "./agent";

const frame = {} as Window;
const other = {} as Window;

function msg(data: unknown, origin = "null", source: unknown = frame) {
  return parseAgentMessage({ data, origin, source }, frame);
}

describe("messages de l'agent", () => {
  it("accepte hauteur, lien et touche bien formés", () => {
    expect(msg({ type: "norelly:height", height: 812.4 })).toEqual({ type: "height", height: 813 });
    expect(msg({ type: "norelly:link", href: "https://exemple.fr" })).toEqual({
      type: "link",
      href: "https://exemple.fr",
    });
    expect(
      msg({ type: "norelly:key", key: "J", ctrl: false, alt: false, shift: true, meta: false }),
    ).toEqual({
      type: "key",
      shortcut: { key: "j", ctrl: false, alt: false, shift: true, meta: false },
    });
  });

  it("ignore toute autre source ou origine", () => {
    const ok = { type: "norelly:height", height: 10 };
    expect(msg(ok, "null", other)).toBeNull();
    expect(msg(ok, "tauri://localhost")).toBeNull();
    expect(msg(ok, "mailview://localhost")).toBeNull();
    expect(parseAgentMessage({ data: ok, origin: "null", source: frame }, null)).toBeNull();
  });

  it("refuse les messages malformés ou démesurés", () => {
    for (const bad of [
      null,
      "norelly:height",
      [],
      { type: "inconnu" },
      { type: "norelly:height", height: "10" },
      { type: "norelly:height", height: -1 },
      { type: "norelly:height", height: Number.NaN },
      { type: "norelly:link", href: "" },
      { type: "norelly:link", href: "a".repeat(5000) },
      { type: "norelly:link", href: 42 },
      { type: "norelly:key", key: "j" },
      {
        type: "norelly:key",
        key: "x".repeat(40),
        ctrl: false,
        alt: false,
        shift: false,
        meta: false,
      },
      { type: "norelly:key", key: "j", ctrl: "oui", alt: false, shift: false, meta: false },
    ]) {
      expect(msg(bad)).toBeNull();
    }
    expect(msg({ type: "norelly:height", height: 1e12 })).toEqual({
      type: "height",
      height: MAX_FRAME_HEIGHT,
    });
  });

  it("construit l'adresse mailview selon la plateforme", () => {
    expect(mailviewUrl(12, 3, false)).toBe("mailview://localhost/12?v=3");
    expect(mailviewUrl(12, 0, true)).toBe("http://mailview.localhost/12?v=0");
  });
});
