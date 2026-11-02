import { describe, expect, it, vi } from "vitest";
import { AddAccountFlow, defaultServers } from "./add-account.svelte";
import type {
  AccountDto,
  AddAccountInput,
  AddAccountResult,
  AccountApi,
  Discovery,
  ServerDto,
} from "./ipc/api";

const imap: ServerDto = {
  host: "imap.example.com",
  port: 993,
  security: "tls",
  username: "alice@example.com",
};
const smtp: ServerDto = {
  host: "smtp.example.com",
  port: 465,
  security: "tls",
  username: "alice@example.com",
};
const account: AccountDto = {
  id: 1,
  address: "alice@example.com",
  display_name: "alice",
  imap,
  smtp,
};
const found: Discovery = {
  imap: { server: imap, source: "mozilla_ispdb", requires_confirmation: false },
  smtp: { server: smtp, source: "mozilla_ispdb", requires_confirmation: false },
};
const certificate: AddAccountResult = {
  kind: "certificate",
  problem: {
    protocol: "imap",
    host: "imap.example.com",
    fingerprint: "AA:BB",
    issue: "unknown_issuer",
    reason: "autorité inconnue",
    fingerprint_changed: false,
    can_trust: true,
  },
};

function fakeApi(discovery: Discovery, results: AddAccountResult[]) {
  const calls: AddAccountInput[] = [];
  const api: AccountApi = {
    listAccounts: vi.fn(async () => []),
    discoverSettings: vi.fn(async (_address, onProgress) => {
      onProgress("provider_autoconfig");
      onProgress("mozilla_ispdb");
      return discovery;
    }),
    addAccount: vi.fn(async (input: AddAccountInput) => {
      calls.push(structuredClone(input));
      const next = results.shift();
      if (next === undefined) throw new Error("appel inattendu");
      return next;
    }),
    trustCertificate: vi.fn(async () => undefined),
    removeAccount: vi.fn(async () => undefined),
    allowFileSecrets: vi.fn(async () => undefined),
  };
  return { api, calls };
}

function flow(api: AccountApi): AddAccountFlow {
  const f = new AddAccountFlow(api);
  f.address = " alice@example.com ";
  f.password = "secret";
  return f;
}

describe("parcours d'ajout de compte", () => {
  it("découverte complète : test puis ajout, mot de passe effacé", async () => {
    const { api, calls } = fakeApi(found, [{ kind: "added", account }]);
    const f = flow(api);
    await f.submit();
    expect(f.progress).toEqual(["provider_autoconfig", "mozilla_ispdb"]);
    expect(f.step).toBe("done");
    expect(f.added).toEqual(account);
    expect(f.password).toBe("");
    expect(calls[0]).toMatchObject({
      address: "alice@example.com",
      imap,
      smtp,
      plaintext_consents: [],
    });
  });

  it("champs vides : rien n'est envoyé", async () => {
    const { api } = fakeApi(found, []);
    const f = new AddAccountFlow(api);
    await f.submit();
    expect(f.step).toBe("form");
    expect(f.message).not.toBeNull();
    expect(api.discoverSettings).not.toHaveBeenCalled();
  });

  it("découverte incomplète : saisie manuelle pré-remplie", async () => {
    const { api } = fakeApi({ imap: found.imap, smtp: null }, []);
    const f = flow(api);
    await f.submit();
    expect(f.step).toBe("manual");
    expect(f.imap).toEqual(imap);
    expect(f.smtp).toEqual(defaultServers("alice@example.com").smtp);
    expect(api.addAccount).not.toHaveBeenCalled();
  });

  it("serveur DNS hors domaine : confirmation demandée", async () => {
    const { api } = fakeApi(
      { imap: { server: imap, source: "dns_srv", requires_confirmation: true }, smtp: found.smtp },
      [],
    );
    const f = flow(api);
    await f.submit();
    expect(f.step).toBe("manual");
    expect(f.message).toMatch(/DNS/);
  });

  it("certificat refusé : avertissement, confiance explicite, nouvel essai", async () => {
    const { api, calls } = fakeApi(found, [certificate, { kind: "added", account }]);
    const f = flow(api);
    await f.submit();
    expect(f.step).toBe("certificate");
    expect(f.certificate?.fingerprint).toBe("AA:BB");
    await f.trustCertificate();
    expect(api.trustCertificate).toHaveBeenCalledWith("imap.example.com", "AA:BB");
    expect(f.step).toBe("done");
    expect(calls).toHaveLength(2);
  });

  it("certificat révoqué : aucune confiance possible", async () => {
    const revoked: AddAccountResult = {
      kind: "certificate",
      problem: {
        ...(certificate.kind === "certificate" ? certificate.problem : ({} as never)),
        can_trust: false,
      },
    };
    const { api } = fakeApi(found, [revoked]);
    const f = flow(api);
    await f.submit();
    await f.trustCertificate();
    expect(api.trustCertificate).not.toHaveBeenCalled();
    expect(f.step).toBe("certificate");
  });

  it("refus de l'avertissement : retour à la saisie, rien n'est accepté", async () => {
    const { api } = fakeApi(found, [certificate]);
    const f = flow(api);
    await f.submit();
    f.decline();
    expect(f.step).toBe("manual");
    expect(api.trustCertificate).not.toHaveBeenCalled();
  });

  it("clair : consentement explicite transmis pour ce serveur seulement", async () => {
    const endpoint = { host: "mail.legacy.example", port: 143 };
    const { api, calls } = fakeApi(found, [
      { kind: "plaintext_consent_required", endpoint },
      { kind: "added", account },
    ]);
    const f = flow(api);
    await f.submit();
    expect(f.step).toBe("plaintext");
    expect(calls[0]?.plaintext_consents).toEqual([]);
    await f.allowPlaintext();
    expect(calls[1]?.plaintext_consents).toEqual([endpoint]);
    expect(f.step).toBe("done");
  });

  it("sans trousseau : consentement au fichier, puis ajout", async () => {
    const { api, calls } = fakeApi(found, [
      {
        kind: "secret_storage_consent_required",
        path: "/home/a/.local/share/x/mots-de-passe.json",
      },
      { kind: "added", account },
    ]);
    const f = flow(api);
    await f.submit();
    expect(f.step).toBe("secrets");
    expect(f.secretsFile).toContain("mots-de-passe.json");
    expect(api.allowFileSecrets).not.toHaveBeenCalled();
    await f.allowFileSecrets();
    expect(api.allowFileSecrets).toHaveBeenCalledOnce();
    expect(calls).toHaveLength(2);
    expect(f.step).toBe("done");
  });

  it("sans trousseau : refus, rien n'est enregistré", async () => {
    const { api } = fakeApi(found, [
      { kind: "secret_storage_consent_required", path: "/x/mots-de-passe.json" },
    ]);
    const f = flow(api);
    await f.submit();
    f.decline();
    expect(f.step).toBe("manual");
    expect(api.allowFileSecrets).not.toHaveBeenCalled();
  });

  it("échec d'authentification : message et saisie manuelle", async () => {
    const { api } = fakeApi(found, [
      { kind: "failed", protocol: "smtp", failure: "authentication", message: "535 refusé" },
    ]);
    const f = flow(api);
    await f.submit();
    expect(f.step).toBe("manual");
    expect(f.message).toMatch(/SMTP/);
    expect(f.message).toMatch(/identifiants refusés/);
  });

  it("compte existant", async () => {
    const { api } = fakeApi(found, [{ kind: "already_exists" }]);
    const f = flow(api);
    await f.submit();
    expect(f.step).toBe("form");
    expect(f.message).toMatch(/existe déjà/);
  });

  it("reset efface le mot de passe", () => {
    const { api } = fakeApi(found, []);
    const f = flow(api);
    f.reset();
    expect(f.password).toBe("");
    expect(f.step).toBe("form");
  });
});

describe("serveurs DNS hors domaine", () => {
  const hostile: ServerDto = { ...imap, host: "imap.attaquant.net" };

  it("avertit même si la découverte est partielle, et bloque sans confirmation", async () => {
    const { api, calls } = fakeApi(
      { imap: { server: hostile, source: "dns_srv", requires_confirmation: true }, smtp: null },
      [{ kind: "added", account }],
    );
    const f = flow(api);
    await f.submit();
    expect(f.step).toBe("manual");
    expect(f.message).toContain("imap.attaquant.net");
    expect(f.pendingConfirmations).toEqual(["imap.attaquant.net"]);
    await f.submitManual();
    expect(api.addAccount).not.toHaveBeenCalled();

    f.setConfirmed("imap.attaquant.net", true);
    await f.submitManual();
    expect(calls[0]?.confirmed_hosts).toEqual(["imap.attaquant.net"]);
    expect(f.step).toBe("done");
  });

  it("remplacer l'hôte lève l'exigence de confirmation", async () => {
    const { api, calls } = fakeApi(
      {
        imap: { server: hostile, source: "dns_srv", requires_confirmation: true },
        smtp: found.smtp,
      },
      [{ kind: "added", account }],
    );
    const f = flow(api);
    await f.submit();
    f.imap.host = "imap.example.com";
    await f.submitManual();
    expect(calls[0]?.confirmed_hosts).toEqual([]);
  });

  it("refus côté Rust : retour à la saisie avec l'hôte à confirmer", async () => {
    const { api } = fakeApi(found, [{ kind: "confirmation_required", host: "imap.example.com" }]);
    const f = flow(api);
    await f.submit();
    expect(f.step).toBe("manual");
    expect(f.pendingConfirmations).toEqual(["imap.example.com"]);
  });
});
