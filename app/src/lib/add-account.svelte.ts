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
  OAuthSignInStepDto,
  SecretsBackendDto,
  ServerDto,
} from "./ipc/api";
import { m } from "./paraglide/messages.js";
import { IpcFailure, errorText, withDetail } from "./errors";

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
  #appPasswordHelp = $state(false);
  /** Serveurs affichés quand l'aide a été donnée. */
  #helpHosts = $state<[string, string]>(["", ""]);

  /** Compte Google refusé : étapes du mot de passe d'application. L'aide
   *  vaut pour les serveurs qui ont refusé : elle disparaît dès que l'un
   *  d'eux est modifié. */
  get appPasswordHelp(): boolean {
    const [imap, smtp] = this.#helpHosts;
    return (
      this.#appPasswordHelp && this.imap.host.trim() === imap && this.smtp.host.trim() === smtp
    );
  }

  set appPasswordHelp(value: boolean) {
    this.#appPasswordHelp = value;
    this.#helpHosts = [this.imap.host.trim(), this.smtp.host.trim()];
  }
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
  /** Dernière étape de « Se connecter avec … » annoncée par Rust. */
  #signInStep = $state<OAuthSignInStepDto | null>(null);
  /** Numéro de l'essai en cours : une réponse d'un essai abandonné
   *  (fenêtre fermée, annulation) est ignorée. */
  #attempt = 0;
  #consents: EndpointDto[] = [];
  #api: AccountApi;

  constructor(api: AccountApi) {
    this.#api = api;
  }

  /** Fenêtre verrouillée : un essai est en cours. L'attente du navigateur
   *  ne verrouille pas : l'utilisateur peut fermer ou annuler. */
  get busy(): boolean {
    return this.step === "discovering" || (this.step === "testing" && !this.waitingForBrowser);
  }

  /** Efface le mot de passe et revient au début. Un essai en cours est
   *  abandonné : sa réponse sera ignorée. */
  reset(): void {
    this.#attempt += 1;
    this.#signInStep = null;
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
    this.nameInManual = false;
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

  /** Nom affiché refusé par Rust, montré sous le champ ; `null` : accepté
   *  ou vide (nom par défaut). */
  displayNameError = $state<string | null>(null);
  /** Le nom a été refusé pendant la saisie manuelle : son champ y est
   *  affiché, pour le corriger sans perdre les serveurs saisis. */
  nameInManual = $state(false);
  /** Une validation (Continuer, Tester et ajouter) est en cours : un
   *  second clic est ignoré. */
  #submitting = false;

  /** Message du refus de `name` par Rust, ou `null`. Seul un refus du nom
   *  compte : une autre erreur (vérification impossible) ne bloque rien,
   *  Rust revalidant le nom à l'ajout. */
  async #nameError(name: string): Promise<string | null> {
    if (!name) return null;
    try {
      await this.#api.checkDisplayName(name);
      return null;
    } catch (e) {
      return e instanceof IpcFailure && e.field === "display_name" ? errorText(e) : null;
    }
  }

  /** Vérification pendant la saisie : une réponse périmée (nom changé
   *  pendant l'appel) ne touche pas au message. */
  async checkDisplayName(): Promise<void> {
    const name = this.displayName.trim();
    const error = await this.#nameError(name);
    if (this.displayName.trim() === name) this.displayNameError = error;
  }

  /** Vérification avant de poursuivre : vrai si le nom affiché à la fin
   *  de la vérification est accepté (revérifié s'il a changé entre-temps). */
  async #nameAccepted(): Promise<boolean> {
    for (;;) {
      const name = this.displayName.trim();
      const error = await this.#nameError(name);
      if (this.displayName.trim() === name) {
        this.displayNameError = error;
        return error === null;
      }
    }
  }

  /** Nom affiché refusé à l'ajout : retour à la saisie, où le champ et son
   *  message sont visibles (saisie manuelle : serveurs gardés). */
  #nameRefused(e: unknown): boolean {
    if (!(e instanceof IpcFailure && e.code === "display_name_looks_like_address")) return false;
    if (this.#oauth) {
      this.step = "form";
    } else {
      this.step = "manual";
      this.nameInManual = true;
    }
    this.message = null;
    this.displayNameError = errorText(e);
    return true;
  }

  /** Exécute `run` sauf si une validation est déjà en cours. */
  async #once(run: () => Promise<void>): Promise<void> {
    if (this.#submitting) return;
    this.#submitting = true;
    try {
      await run();
    } finally {
      this.#submitting = false;
    }
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
    // Rust se connecte chez le fournisseur de l'adresse envoyée : celle du
    // clic, dont le fournisseur est redétecté. Adresse ou fournisseur
    // changé entre-temps : rien n'est ouvert, l'utilisateur clique sur le
    // bouton mis à jour.
    const shown = this.signInProvider;
    const address = this.address.trim();
    const attempt = ++this.#attempt;
    // Une annulation encore en route arrêterait ce nouvel essai : Rust la
    // reçoit d'abord.
    await this.#cancelling;
    await this.detectProvider();
    const provider = this.signInProvider;
    if (
      attempt !== this.#attempt ||
      provider === null ||
      provider !== shown ||
      this.address.trim() !== address
    ) {
      return;
    }
    if (
      !(await this.#nameAccepted()) ||
      attempt !== this.#attempt ||
      this.address.trim() !== address
    ) {
      return;
    }
    this.message = null;
    this.appPasswordHelp = false;
    this.#oauth = true;
    this.#manual = false;
    this.#signInStep = null;
    this.step = "testing";
    let result: AddAccountResult;
    try {
      result = await this.#api.addOAuthAccount(
        {
          address,
          display_name: this.displayName.trim() || null,
        },
        // Rust annonce le navigateur seulement quand il l'ouvre vraiment.
        (step) => {
          if (attempt === this.#attempt) this.#signInStep = step;
        },
      );
    } catch (e) {
      if (attempt !== this.#attempt) return;
      this.#signInStep = null;
      if (this.#nameRefused(e)) return;
      this.step = "form";
      this.message = errorText(e);
      return;
    }
    if (attempt !== this.#attempt) return;
    this.#signInStep = null;
    this.apply(result);
  }

  /** Vrai pendant l'attente du retour du navigateur. */
  get waitingForBrowser(): boolean {
    return this.#oauth && this.step === "testing" && this.#signInStep === "browser_opened";
  }

  /** Vrai pendant un essai par le fournisseur hors attente du navigateur
   *  (connexion gardée, ou obtenue et serveurs en test). Avant la première
   *  étape annoncée par Rust, l'essai compte aussi ici, fenêtre verrouillée :
   *  une annulation arrivée avant que Rust ne l'observe serait perdue. */
  get checkingSignIn(): boolean {
    return this.#oauth && this.step === "testing" && this.#signInStep !== "browser_opened";
  }

  /** « Annuler » pendant l'attente du navigateur : Rust arrête l'attente
   *  et oublie la connexion gardée ; retour au formulaire. */
  async cancelSignIn(): Promise<void> {
    this.#attempt += 1;
    this.#signInStep = null;
    this.step = "form";
    this.message = m.add_msg_cancelled();
    await this.#cancelInRust();
  }

  /** Fenêtre fermée : l'essai en cours est abandonné et Rust oublie la
   *  connexion gardée. */
  async forget(): Promise<void> {
    this.reset();
    await this.#cancelInRust();
  }

  /** Annulation envoyée à Rust, pas encore traitée. */
  #cancelling: Promise<void> = Promise.resolve();

  #cancelInRust(): Promise<void> {
    this.#cancelling = this.#api.cancelOAuthSignIn().catch(() => {
      // Rust injoignable : l'attente s'arrêtera d'elle-même (délai).
    });
    return this.#cancelling;
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
  submit(): Promise<void> {
    return this.#once(() => this.#submit());
  }

  async #submit(): Promise<void> {
    this.message = null;
    this.appPasswordHelp = false;
    this.#manual = false;
    if (!this.address.trim() || !this.password) {
      this.message = m.add_msg_missing_fields();
      return;
    }
    if (!(await this.#nameAccepted())) return;
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
  submitManual(): Promise<void> {
    return this.#once(() => this.#submitManual());
  }

  async #submitManual(): Promise<void> {
    this.message = null;
    this.appPasswordHelp = false;
    this.#manual = true;
    const pending = this.pendingConfirmations;
    if (pending.length > 0) {
      this.message = m.add_msg_confirm_first({ hosts: pending.join(", ") });
      return;
    }
    if (!(await this.#nameAccepted())) {
      this.nameInManual = true;
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
      if (this.#nameRefused(e)) return;
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
          this.#oauth,
        );
        break;
    }
  }
}

/** Message d'un échec de connexion ; `oauth` : compte connecté chez son
 * fournisseur, sans identifiants saisis. */
export function failureMessage(
  failure: FailureKindDto,
  protocol: string | null,
  detail: string | null,
  address: string,
  oauth = false,
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
      case "closed_before_encryption":
        return m.add_fail_closed_before_encryption({ prefix });
      case "authentication_interrupted":
        return oauth
          ? m.add_fail_authentication_interrupted_oauth({ prefix })
          : m.add_fail_authentication_interrupted({ prefix });
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
