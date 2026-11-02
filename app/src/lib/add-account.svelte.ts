// Parcours d'ajout de compte : adresse + mot de passe, découverte,
// test de connexion, avertissements (certificat, clair), saisie manuelle.
//
// Ce contrôleur ne décide de rien côté sécurité : il relaie les choix
// explicites de l'utilisateur (faire confiance à un certificat, accepter le
// clair) au Rust, qui revalide tout.

import type {
  AccountDto,
  AddAccountResult,
  AccountApi,
  CertificateProblemDto,
  Discovery,
  DiscoverySourceDto,
  EndpointDto,
  FailureKindDto,
  ServerDto,
} from "./ipc/api";
import { m } from "./paraglide/messages.js";
import { errorText, withDetail } from "./errors";

export type Step =
  "form" | "discovering" | "testing" | "manual" | "certificate" | "plaintext" | "secrets" | "done";

/** Libellé d'une source de découverte. */
export function sourceLabel(source: DiscoverySourceDto): string {
  switch (source) {
    case "provider_autoconfig":
      return m.add_source_provider_autoconfig();
    case "mozilla_ispdb":
      return m.add_source_mozilla_ispdb();
    case "dns_srv":
      return m.add_source_dns_srv();
    case "probe":
      return m.add_source_probe();
    case "manual":
      return m.add_source_manual();
  }
}

/** Explication d'un défaut de certificat. */
export function issueText(issue: string): string {
  switch (issue) {
    case "unknown_issuer":
      return m.add_issue_unknown_issuer();
    case "expired":
      return m.add_issue_expired();
    case "not_yet_valid":
      return m.add_issue_not_yet_valid();
    case "hostname_mismatch":
      return m.add_issue_hostname_mismatch();
    case "revoked":
      return m.add_issue_revoked();
    case "invalid_signature":
      return m.add_issue_invalid_signature();
    case "malformed":
      return m.add_issue_malformed();
    default:
      return m.add_issue_other();
  }
}

function domainOf(address: string): string {
  const at = address.lastIndexOf("@");
  return at >= 0 ? address.slice(at + 1).trim() : "";
}

/** Serveurs par défaut proposés en saisie manuelle. */
export function defaultServers(address: string): { imap: ServerDto; smtp: ServerDto } {
  const domain = domainOf(address);
  const username = address.trim();
  return {
    imap: { host: domain ? `imap.${domain}` : "", port: 993, security: "tls", username },
    smtp: { host: domain ? `smtp.${domain}` : "", port: 465, security: "tls", username },
  };
}

export class AddAccountFlow {
  step = $state<Step>("form");
  address = $state("");
  password = $state("");
  displayName = $state("");
  progress = $state<DiscoverySourceDto[]>([]);
  imap = $state<ServerDto>(defaultServers("").imap);
  smtp = $state<ServerDto>(defaultServers("").smtp);
  /** Message affiché à l'étape courante (erreur ou explication). */
  message = $state<string | null>(null);
  certificate = $state<CertificateProblemDto | null>(null);
  plaintextEndpoint = $state<EndpointDto | null>(null);
  /** Fichier proposé pour les mots de passe, faute de trousseau. */
  secretsFile = $state<string | null>(null);
  added = $state<AccountDto | null>(null);
  /** Hôtes signalés par la découverte (DNS hors domaine), à confirmer. */
  unconfirmed = $state<{ host: string; source: DiscoverySourceDto }[]>([]);
  /** Hôtes confirmés explicitement par l'utilisateur. */
  confirmed = $state<string[]>([]);
  #consents: EndpointDto[] = [];
  #api: AccountApi;

  constructor(api: AccountApi) {
    this.#api = api;
  }

  get busy(): boolean {
    return this.step === "discovering" || this.step === "testing";
  }

  /** Efface le mot de passe et revient au début. */
  reset(): void {
    this.step = "form";
    this.password = "";
    this.progress = [];
    this.message = null;
    this.certificate = null;
    this.plaintextEndpoint = null;
    this.added = null;
    this.unconfirmed = [];
    this.confirmed = [];
    this.#consents = [];
  }

  /** Hôtes à confirmer encore présents dans les champs et non confirmés. */
  get pendingConfirmations(): string[] {
    const inUse = [this.imap.host.trim(), this.smtp.host.trim()];
    return this.unconfirmed
      .map((u) => u.host)
      .filter((h) => inUse.includes(h) && !this.confirmed.includes(h));
  }

  /** Coche ou décoche la confirmation d'un hôte. */
  setConfirmed(host: string, value: boolean): void {
    const others = this.confirmed.filter((h) => h !== host);
    this.confirmed = value ? [...others, host] : others;
  }

  /** Étape 1 : découverte, puis test et ajout si tout est trouvé. */
  async submit(): Promise<void> {
    this.message = null;
    if (!this.address.trim() || !this.password) {
      this.message = m.add_msg_missing_fields();
      return;
    }
    this.step = "discovering";
    this.progress = [];
    let discovery: Discovery;
    try {
      discovery = await this.#api.discoverSettings(this.address.trim(), (source) => {
        this.progress = [...this.progress, source];
      });
    } catch (e) {
      this.step = "form";
      this.message = errorText(e);
      return;
    }
    const defaults = defaultServers(this.address);
    this.imap = discovery.imap?.server ?? defaults.imap;
    this.smtp = discovery.smtp?.server ?? defaults.smtp;
    // Avertissement de sécurité d'abord, que la découverte soit complète ou
    // non : un hôte hors domaine trouvé par le DNS peut être falsifié.
    this.unconfirmed = [discovery.imap, discovery.smtp].flatMap((d) =>
      d?.requires_confirmation === true ? [{ host: d.server.host, source: d.source }] : [],
    );
    if (this.unconfirmed.length > 0) {
      const hosts = this.unconfirmed.map((u) => u.host).join(", ");
      this.step = "manual";
      this.message = m.add_msg_dns_outside({ hosts });
      return;
    }
    if (!discovery.imap || !discovery.smtp) {
      this.step = "manual";
      this.message = m.add_msg_incomplete();
      return;
    }
    await this.connect();
  }

  /** Saisie manuelle validée. */
  async submitManual(): Promise<void> {
    this.message = null;
    const pending = this.pendingConfirmations;
    if (pending.length > 0) {
      this.message = m.add_msg_confirm_first({ hosts: pending.join(", ") });
      return;
    }
    await this.connect();
  }

  /** L'utilisateur fait confiance au certificat présenté. */
  async trustCertificate(): Promise<void> {
    const problem = this.certificate;
    if (problem === null || !problem.can_trust) return;
    try {
      await this.#api.trustCertificate(problem.host, problem.fingerprint);
    } catch (e) {
      this.step = "manual";
      this.message = errorText(e);
      return;
    }
    this.certificate = null;
    await this.connect();
  }

  /** L'utilisateur accepte une connexion non chiffrée vers ce serveur. */
  async allowPlaintext(): Promise<void> {
    const endpoint = this.plaintextEndpoint;
    if (endpoint === null) return;
    this.#consents = [...this.#consents, { host: endpoint.host, port: endpoint.port }];
    this.plaintextEndpoint = null;
    await this.connect();
  }

  /** Faute de trousseau, l'utilisateur accepte le fichier non chiffré. */
  async allowFileSecrets(): Promise<void> {
    try {
      await this.#api.allowFileSecrets();
    } catch (e) {
      this.step = "manual";
      this.message = errorText(e);
      return;
    }
    this.secretsFile = null;
    await this.connect();
  }

  /** Refus d'un avertissement : retour à la saisie manuelle. */
  decline(): void {
    this.certificate = null;
    this.plaintextEndpoint = null;
    this.secretsFile = null;
    this.step = "manual";
    this.message = m.add_msg_cancelled();
  }

  async connect(): Promise<void> {
    this.step = "testing";
    let result: AddAccountResult;
    try {
      result = await this.#api.addAccount({
        address: this.address.trim(),
        display_name: this.displayName.trim() || null,
        password: this.password,
        imap: { ...this.imap, port: Number(this.imap.port) },
        smtp: { ...this.smtp, port: Number(this.smtp.port) },
        plaintext_consents: this.#consents,
        confirmed_hosts: [...this.confirmed],
      });
    } catch (e) {
      this.step = "manual";
      this.message = errorText(e);
      return;
    }
    this.apply(result);
  }

  apply(result: AddAccountResult): void {
    switch (result.kind) {
      case "added":
        this.added = result.account;
        this.password = "";
        this.step = "done";
        break;
      case "already_exists":
        this.step = "form";
        this.message = m.add_msg_already_exists();
        break;
      case "needs_review":
        this.step = "manual";
        this.message = m.add_msg_needs_review();
        break;
      case "certificate":
        this.certificate = result.problem;
        this.step = "certificate";
        break;
      case "confirmation_required":
        if (!this.unconfirmed.some((u) => u.host === result.host)) {
          this.unconfirmed = [...this.unconfirmed, { host: result.host, source: "dns_srv" }];
        }
        this.step = "manual";
        this.message = m.add_msg_must_confirm({ host: result.host });
        break;
      case "secret_storage_consent_required":
        this.secretsFile = result.path;
        this.step = "secrets";
        break;
      case "plaintext_consent_required":
        this.plaintextEndpoint = result.endpoint;
        this.step = "plaintext";
        break;
      case "failed":
        this.step = "manual";
        this.message = failureMessage(result.failure, result.protocol, result.detail);
        break;
    }
  }
}

function failureMessage(
  failure: FailureKindDto,
  protocol: string | null,
  detail: string | null,
): string {
  const prefix =
    protocol === "imap" ? m.add_server_imap() : protocol === "smtp" ? m.add_server_smtp() : "";
  const text = (() => {
    switch (failure) {
      case "authentication":
        return m.add_fail_authentication({ prefix });
      case "network":
        return m.add_fail_network({ prefix });
      case "security":
        return m.add_fail_security({ prefix });
      case "invalid_input":
        return m.add_fail_invalid_input();
      case "secret_store":
        return m.add_fail_secret_store();
      case "storage":
        return m.add_fail_storage();
      case "protocol":
      case "internal":
        return m.add_fail_other({ prefix });
    }
  })();
  // Le détail technique (réponse du serveur) est cité tel quel, jamais traduit.
  return withDetail(text, detail);
}
