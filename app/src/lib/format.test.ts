import { describe, expect, it } from "vitest";
import { fileSize, personName, shortDate } from "./format";

describe("mise en forme", () => {
  it("date courte selon l'ancienneté", () => {
    const now = new Date(2026, 10, 2, 15, 0);
    const today = new Date(2026, 10, 2, 9, 5).getTime() / 1000;
    expect(shortDate(today, now)).toBe("09:05");
    expect(shortDate(new Date(2026, 2, 12).getTime() / 1000, now)).toMatch(/12 mars/);
    expect(shortDate(new Date(2024, 2, 12).getTime() / 1000, now)).toBe("12/03/2024");
  });

  it("tailles et noms", () => {
    expect(fileSize(512)).toBe("512 o");
    expect(fileSize(2048)).toBe("2 Ko");
    expect(fileSize(3.5 * 1024 * 1024)).toBe("3,5 Mo");
    expect(personName({ name: "", address: "a@b.c" })).toBe("a@b.c");
    expect(personName(null)).toBe("(expéditeur inconnu)");
  });
});
