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
  OAuthProviderDto,
  SecretsBackendDto,
  ServerDto,
} from "./ipc/api";
import { m } from "./paraglide/messages.js";
import { errorText, withDetail } from "./errors";

/** Page Google de création des mots de passe d'application. */
export const GOOGLE_APP_PASSWORDS_URL = "https://myaccount.google.com/apppasswords";

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
  /** Compte Google refusé : étapes du mot de passe d'application. */
  appPasswordHelp = $state(false);
  /** Où Rust conservera le mot de passe ; `null` tant qu'il n'a pas
   *  répondu : aucune garantie n'est affichée avant. */
  secretsBackend = $state<SecretsBackendDto | null>(null);
  /** Fournisseur OAuth de l'adresse saisie (décidé par Rust) : le
   *  formulaire propose alors « Se connecter avec … » au lieu du mot de
   *  passe. */
  oauthProvider = $state<OAuthProviderDto | null>(null);

  /** Compte Google : l'utilisateur a choisi le mot de passe d'application
   *  plutôt que « Se connecter avec Google ». */
  passwordInstead = $state(false);

  /** Fournisseur de « Se connecter avec … » proposé par le formulaire, ou
   *  `null` pour la connexion par mot de passe. */
  get signInProvider(): OAuthProviderDto | null {
    return this.canUsePassword && this.passwordInstead ? null : this.oauthProvider;
  }

  /** Le mot de passe d'application reste possible chez Google ; Microsoft
   *  refuse tout mot de passe. */
  get canUsePassword(): boolean {
    return this.oauthProvider === "google";
  }

  /** « Utiliser un mot de passe d'application » (Google). */
  usePassword(): void {
    if (this.canUsePassword) this.passwordInstead = true;
  }

  /** Retour à « Se connecter avec … » : le mot de passe saisi est effacé. */
  useProviderSignIn(): void {
    this.passwordInstead = false;
    this.appPasswordHelp = false;
    this.password = "";
  }
  /** Le dernier essai vient de la saisie manuelle des serveurs. */
  #manual = false;
  /** Le dernier essai passe par le fournisseur OAuth. */
  /** Message à montrer après l'ajout (envoi non vérifié), ou `null`. */
  addedNotice = $state<string | null>(null);
  #oauth = false;
  #browser = true;
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
    this.addedNotice = null;
    this.unconfirmed = [];
    this.confirmed = [];
    this.appPasswordHelp = false;
    this.passwordInstead = false;
    this.secretsBackend = null;
    this.#manual = false;
    this.#oauth = false;
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

  /** Demande à Rust si l'adresse relève d'un fournisseur OAuth. */
  async detectProvider(): Promise<void> {
    const address = this.address.trim();
    try {
      const provider = address ? await this.#api.oauthProviderFor(address) : null;
      // Réponse périmée : l'adresse a changé pendant l'appel.
      if (this.address.trim() === address && provider !== this.oauthProvider) {
        this.oauthProvider = provider;
        this.passwordInstead = false;
      }
    } catch {
      this.oauthProvider = null;
    }
  }

  /** « Se connecter avec … » : le navigateur du système s'ouvre, Rust
   *  attend le retour, teste les serveurs et ajoute le compte. */
  async signInWithProvider(): Promise<void> {
    // L'adresse a pu changer depuis la dernière détection : Rust refuserait
    // un fournisseur qui n'est pas celui de l'adresse. Fournisseur changé :
    // rien n'est ouvert, l'utilisateur clique sur le bouton mis à jour.
    const shown = this.signInProvider;
    await this.detectProvider();
    const provider = this.signInProvider;
    if (provider === null || provider !== shown) return;
    this.message = null;
    this.appPasswordHelp = false;
    this.#oauth = true;
    this.#manual = false;
    // Rust dit si la connexion d'un essai échoué resservira, sans navigateur.
    this.#browser = !(await this.#api
      .oauthSignInKept(this.address.trim(), provider)
      .catch(() => false));
    this.step = "testing";
    let result: AddAccountResult;
    try {
      result = await this.#api.addOAuthAccount({
        address: this.address.trim(),
        display_name: this.displayName.trim() || null,
        provider,
      });
    } catch (e) {
      this.step = "form";
      this.message = errorText(e);
      return;
    }
    this.apply(result);
  }

  /** Vrai pendant l'attente du retour du navigateur. */
  get waitingForBrowser(): boolean {
    return this.#oauth && this.#browser && this.step === "testing";
  }

  /** Vrai pendant un nouvel essai qui réutilise la connexion gardée. */
  get reusingSignIn(): boolean {
    return this.#oauth && !this.#browser && this.step === "testing";
  }

  /** Ouvre la page Google de création des mots de passe d'application. */
  async openAppPasswords(): Promise<void> {
    try {
      await this.#api.openExternal(GOOGLE_APP_PASSWORDS_URL);
    } catch (e) {
      this.message = errorText(e);
    }
  }

  /** Demande à Rust où le mot de passe sera conservé : trousseau du
   *  système ou fichier non chiffré. Sans réponse, aucune garantie n'est
   *  affichée (fausse garantie pire que silence). */
  async loadSecretsBackend(): Promise<void> {
    try {
      this.secretsBackend = await this.#api.secretsBackend();
    } catch {
      this.secretsBackend = null;
    }
  }

  /** Étape 1 : découverte, puis test et ajout si tout est trouvé. */
  async submit(): Promise<void> {
    this.message = null;
    this.appPasswordHelp = false;
    this.#manual = false;
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
    this.appPasswordHelp = false;
    this.#manual = true;
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
    if (this.#oauth) {
      await this.signInWithProvider();
    } else {
      await this.connect();
    }
  }

  /** Refus d'un avertissement : retour à la saisie manuelle. */
  decline(): void {
    this.certificate = null;
    this.plaintextEndpoint = null;
    this.secretsFile = null;
    this.step = this.#oauth ? "form" : "manual";
    this.message = m.add_msg_cancelled();
  }

  async connect(): Promise<void> {
    this.#oauth = false;
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
        this.addedNotice = result.send_unchecked ? m.notice_send_unchecked() : null;
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
        if (result.failure === "oauth_google_refused") {
          // Google a pu bloquer le nouveau compte : mot de passe
          // d'application, sur le même écran.
          this.step = "form";
          this.passwordInstead = true;
          this.appPasswordHelp = true;
        } else if (result.failure === "google_app_password") {
          // Seul le mot de passe est à changer : retour à l'écran d'où
          // vient l'essai, champ du mot de passe vidé.
          this.step = this.#manual ? "manual" : "form";
          this.password = "";
          this.appPasswordHelp = true;
        } else {
          // Connexion par le fournisseur : aucun serveur à corriger.
          this.step = this.#oauth ? "form" : "manual";
        }
        this.message = failureMessage(
          result.failure,
          result.protocol,
          result.detail,
          this.address.trim(),
        );
        break;
    }
  }
}

function failureMessage(
  failure: FailureKindDto,
  protocol: string | null,
  detail: string | null,
  address: string,
): string {
  const prefix =
    protocol === "imap" ? m.add_server_imap() : protocol === "smtp" ? m.add_server_smtp() : "";
  const text = (() => {
    switch (failure) {
      case "authentication":
        return m.add_fail_authentication({ prefix });
      case "google_app_password":
        return m.add_fail_google_app_password({ prefix });
      case "microsoft_smtp_disabled":
        return m.add_fail_microsoft_smtp_disabled({ prefix });
      case "microsoft_imap_unavailable":
        return m.add_fail_microsoft_imap_unavailable({ prefix });
      case "reauth_required":
        return m.add_fail_reauth_required();
      case "oauth_cancelled":
        return m.add_fail_oauth_cancelled();
      case "oauth_google_refused":
        return m.add_fail_oauth_google_refused();
      case "oauth_mail_access":
        return m.add_fail_oauth_mail_access();
      case "oauth_not_configured":
        return m.add_fail_oauth_not_configured();
      case "oauth_wrong_account":
        return m.add_fail_oauth_wrong_account({ address });
      case "oauth_offline_access":
        return m.add_fail_oauth_offline_access();
      case "oauth":
        return m.add_fail_oauth();
      case "network":
        return m.add_fail_network({ prefix });
      case "unresponsive":
        return m.add_fail_unresponsive({ prefix });
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
