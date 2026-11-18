import { afterEach, describe, expect, it } from "vitest";
import { contactLabel, labelKindText } from "./contact-label";
import { setLocale } from "./paraglide/runtime.js";

describe("étiquette d'un contact", () => {
  afterEach(() => setLocale("fr", { reload: false }));

  it("traduit une étiquette reconnue d'après son genre, pas son texte", () => {
    setLocale("fr", { reload: false });
    expect(contactLabel({ label: "work", label_kind: "work" })).toBe("Travail");
    expect(contactLabel({ label: "cell", label_kind: "mobile" })).toBe("Mobile");
    expect(contactLabel({ label: "maison", label_kind: "home" })).toBe("Domicile");
    expect(labelKindText("pager")).toBe("Bipeur");
    expect(labelKindText("main")).toBe("Principal");
    expect(labelKindText("other")).toBe("Autre");
    expect(labelKindText("fax")).toBe("Fax");
  });

  it("suit la langue de l'interface", () => {
    setLocale("en", { reload: false });
    expect(contactLabel({ label: "Travail", label_kind: "work" })).toBe("Work");
    expect(contactLabel({ label: "maison", label_kind: "home" })).toBe("Home");
  });

  it("garde le texte d'une étiquette inconnue", () => {
    expect(contactLabel({ label: "bureau", label_kind: null })).toBe("bureau");
    expect(contactLabel({ label: "chalet" })).toBe("chalet");
    expect(contactLabel({ label: "" })).toBe("");
  });
});
