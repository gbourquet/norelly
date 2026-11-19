import { describe, expect, it } from "vitest";
import { contactInitial, contactPhotoUrl } from "./contact-photo";

describe("photo d'un contact", () => {
  it("est servie par le protocole dédié, selon la plateforme", () => {
    expect(contactPhotoUrl(12, false)).toBe("contact-photo://localhost/12");
    expect(contactPhotoUrl(12, true)).toBe("http://contact-photo.localhost/12");
  });

  it("laisse place à l'initiale du nom", () => {
    expect(contactInitial("  alice Martin")).toBe("A");
    expect(contactInitial("élodie")).toBe("É");
    expect(contactInitial("")).toBe("");
  });
});
