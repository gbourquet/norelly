// Parcours d'ajout de compte : adresse + mot de passe, découverte,
// test de connexion, avertissements (certificat, clair), saisie manuelle.
//
// Ce contrôleur ne décide de rien côté sécurité : il relaie les choix
// explicites de l'utilisateur (faire confiance à un certificat, accepter le
// clair) au Rust, qui revalide tout.

import type {
  AccountDto,
  AddAccountResult,
  Api,
  CertificateProblemDto,
  Discovery,
  DiscoverySourceDto,
  EndpointDto,
  ServerDto,
} from "./ipc/api";

export type Step =
  "form" | "discovering" | "testing" | "manual" | "certificate" | "plaintext" | "done";

export const SOURCE_LABELS: Record<DiscoverySourceDto, string> = {
  provider_autoconfig: "Configuration publiée par le fournisseur",
  mozilla_ispdb: "Base de fournisseurs de Mozilla",
  dns_srv: "Enregistrements DNS",
  probe: "Sondage des ports usuels",
  manual: "Saisie manuelle",
};

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
  added = $state<AccountDto | null>(null);
  #consents: EndpointDto[] = [];
  #api: Api;

  constructor(api: Api) {
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
    this.#consents = [];
  }

  /** Étape 1 : découverte, puis test et ajout si tout est trouvé. */
  async submit(): Promise<void> {
    this.message = null;
    if (!this.address.trim() || !this.password) {
      this.message = "Saisissez l'adresse et le mot de passe.";
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
      this.message = errorMessage(e);
      return;
    }
    const defaults = defaultServers(this.address);
    this.imap = discovery.imap?.server ?? defaults.imap;
    this.smtp = discovery.smtp?.server ?? defaults.smtp;
    if (!discovery.imap || !discovery.smtp) {
      this.step = "manual";
      this.message =
        "Les serveurs n'ont pas tous été trouvés automatiquement : vérifiez ou complétez les paramètres.";
      return;
    }
    if (discovery.imap.requires_confirmation || discovery.smtp.requires_confirmation) {
      this.step = "manual";
      this.message =
        "Un serveur a été trouvé par le DNS en dehors du domaine de votre adresse. Vérifiez qu'il appartient bien à votre fournisseur.";
      return;
    }
    await this.connect();
  }

  /** Saisie manuelle validée. */
  async submitManual(): Promise<void> {
    this.message = null;
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
      this.message = errorMessage(e);
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

  /** Refus d'un avertissement : retour à la saisie manuelle. */
  decline(): void {
    this.certificate = null;
    this.plaintextEndpoint = null;
    this.step = "manual";
    this.message = "Connexion annulée. Vous pouvez modifier les paramètres.";
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
      });
    } catch (e) {
      this.step = "manual";
      this.message = errorMessage(e);
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
        this.message = "Un compte existe déjà pour cette adresse.";
        break;
      case "needs_review":
        this.step = "manual";
        this.message = "Vérifiez les paramètres des serveurs.";
        break;
      case "certificate":
        this.certificate = result.problem;
        this.step = "certificate";
        break;
      case "plaintext_consent_required":
        this.plaintextEndpoint = result.endpoint;
        this.step = "plaintext";
        break;
      case "failed":
        this.step = "manual";
        this.message = failureMessage(result.failure, result.protocol, result.message);
        break;
    }
  }
}

function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

function failureMessage(failure: string, protocol: string | null, detail: string): string {
  const server =
    protocol === "imap"
      ? "Serveur de réception (IMAP)"
      : protocol === "smtp"
        ? "Serveur d'envoi (SMTP)"
        : "";
  const prefix = server ? `${server} : ` : "";
  switch (failure) {
    case "authentication":
      return `${prefix}identifiants refusés. Vérifiez le nom d'utilisateur et le mot de passe (un mot de passe d'application est parfois nécessaire). Détail : ${detail}`;
    case "network":
      return `${prefix}serveur injoignable. Vérifiez le nom et le port. Détail : ${detail}`;
    case "security":
      return `${prefix}connexion refusée par sécurité. Détail : ${detail}`;
    default:
      return `${prefix}${detail}`;
  }
}
