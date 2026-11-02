<script lang="ts">
  // Parcours d'ajout de compte (voir lib/add-account.svelte.ts).
  import CheckIcon from "@lucide/svelte/icons/check";
  import LoaderIcon from "@lucide/svelte/icons/loader-circle";
  import ShieldAlertIcon from "@lucide/svelte/icons/shield-alert";
  import LockOpenIcon from "@lucide/svelte/icons/lock-open";
  import Modal from "./Modal.svelte";
  import ServerFields from "./ServerFields.svelte";
  import { AddAccountFlow, SOURCE_LABELS } from "../lib/add-account.svelte";
  import type { AccountApi, AccountDto } from "../lib/ipc/api";

  interface Props {
    open: boolean;
    api: AccountApi;
    onClose: () => void;
    onAdded: (account: AccountDto) => void;
  }

  let { open, api, onClose, onAdded }: Props = $props();

  // L'API ne change pas pendant la vie du composant.
  // svelte-ignore state_referenced_locally
  const flow = new AddAccountFlow(api);
  /** Confirmation renforcée exigée quand l'empreinte a changé. */
  let acknowledgeChange = $state(false);

  function close(): void {
    flow.reset();
    onClose();
  }

  $effect(() => {
    if (flow.step === "done" && flow.added !== null) {
      const account = flow.added;
      flow.reset();
      onAdded(account);
    }
  });

  const ISSUES: Record<string, string> = {
    unknown_issuer:
      "Le certificat n'est pas signé par une autorité reconnue (certificat autosigné ?).",
    expired: "Le certificat a expiré.",
    not_yet_valid: "Le certificat n'est pas encore valide.",
    hostname_mismatch: "Le certificat ne correspond pas au nom du serveur.",
    revoked: "Le certificat a été révoqué.",
    invalid_signature: "La signature du certificat est invalide.",
    malformed: "Le certificat est illisible.",
    other: "Le certificat a été refusé.",
  };
</script>

<Modal {open} title="Ajouter un compte" onClose={close} locked={flow.busy} width="560px">
  {#if flow.message}
    <p class="message" role="alert">{flow.message}</p>
  {/if}

  {#if flow.step === "form"}
    <form
      onsubmit={(e) => {
        e.preventDefault();
        void flow.submit();
      }}
    >
      <label for="account-address">Adresse électronique</label>
      <input
        id="account-address"
        class="input"
        type="email"
        bind:value={flow.address}
        autocomplete="email"
        spellcheck="false"
        required
      />
      <label for="account-password">Mot de passe</label>
      <input
        id="account-password"
        class="input"
        type="password"
        bind:value={flow.password}
        autocomplete="current-password"
        required
      />
      <label for="account-name">Nom affiché (facultatif)</label>
      <input id="account-name" class="input" bind:value={flow.displayName} autocomplete="name" />
      <p class="hint">
        Le mot de passe est conservé dans le trousseau du système, jamais dans les fichiers de
        Norelly.
      </p>
      <div class="actions">
        <button
          class="btn"
          type="button"
          onclick={() => {
            flow.step = "manual";
          }}>Saisie manuelle</button
        >
        <button class="btn btn-primary" type="submit">Continuer</button>
      </div>
    </form>
  {:else if flow.step === "discovering" || flow.step === "testing"}
    <div class="progress" aria-live="polite">
      {#if flow.step === "discovering"}
        <p>Recherche des paramètres de {flow.address}…</p>
        <ul>
          {#each flow.progress as source, i (source)}
            <li>
              {#if i === flow.progress.length - 1}
                <LoaderIcon size={14} class="spin" aria-hidden="true" />
              {:else}
                <CheckIcon size={14} aria-hidden="true" />
              {/if}
              {SOURCE_LABELS[source]}
            </li>
          {/each}
        </ul>
      {:else}
        <p>
          <LoaderIcon size={14} class="spin" aria-hidden="true" /> Test de la connexion à {flow.imap
            .host} et {flow.smtp.host}…
        </p>
      {/if}
    </div>
  {:else if flow.step === "manual"}
    <form
      onsubmit={(e) => {
        e.preventDefault();
        void flow.submitManual();
      }}
    >
      {#if !flow.address || !flow.password}
        <label for="manual-address">Adresse électronique</label>
        <input id="manual-address" class="input" type="email" bind:value={flow.address} required />
        <label for="manual-password">Mot de passe</label>
        <input
          id="manual-password"
          class="input"
          type="password"
          bind:value={flow.password}
          required
        />
      {/if}
      {#each flow.unconfirmed as item (item.host)}
        {#if [flow.imap.host.trim(), flow.smtp.host.trim()].includes(item.host)}
          <div class="danger confirm" role="group" aria-label="Confirmation du serveur {item.host}">
            <p>
              <strong>{item.host}</strong> a été trouvé par le DNS ({SOURCE_LABELS[item.source]}) en
              dehors du domaine de votre adresse. Une réponse DNS falsifiée pourrait envoyer votre
              mot de passe à un tiers.
            </p>
            <label>
              <input
                type="checkbox"
                checked={flow.confirmed.includes(item.host)}
                onchange={(e) => flow.setConfirmed(item.host, e.currentTarget.checked)}
              />
              Je confirme que {item.host} est un serveur de mon fournisseur
            </label>
          </div>
        {/if}
      {/each}
      <ServerFields legend="Réception (IMAP)" idPrefix="imap" bind:server={flow.imap} />
      <ServerFields legend="Envoi (SMTP)" idPrefix="smtp" bind:server={flow.smtp} />
      <div class="actions">
        <button class="btn" type="button" onclick={close}>Annuler</button>
        <button class="btn btn-primary" type="submit">Tester et ajouter</button>
      </div>
    </form>
  {:else if flow.step === "certificate" && flow.certificate}
    {@const problem = flow.certificate}
    <div class="warning" role="alertdialog" aria-labelledby="cert-title">
      <h3 id="cert-title">
        <ShieldAlertIcon size={18} aria-hidden="true" /> Certificat non reconnu
      </h3>
      {#if problem.fingerprint_changed}
        <p class="danger">
          <strong>Attention :</strong> ce serveur présente un certificat différent de celui que vous aviez
          accepté. Cela peut indiquer une interception de la connexion.
        </p>
      {/if}
      <p>{ISSUES[problem.issue] ?? problem.reason}</p>
      <dl>
        <dt>Serveur</dt>
        <dd>{problem.host} ({problem.protocol.toUpperCase()})</dd>
        <dt>Empreinte SHA-256</dt>
        <dd class="fingerprint">{problem.fingerprint}</dd>
      </dl>
      {#if problem.can_trust && problem.fingerprint_changed}
        <label class="acknowledge">
          <input type="checkbox" bind:checked={acknowledgeChange} />
          J'ai vérifié auprès de l'administrateur que ce nouveau certificat est légitime
        </label>
      {/if}
      {#if problem.can_trust}
        <p>
          Ne faites confiance à ce serveur que si vous savez qu'il utilise ce certificat (serveur
          personnel, réseau d'entreprise) et que l'empreinte correspond à celle communiquée par son
          administrateur.
        </p>
      {:else}
        <p class="danger">Ce certificat ne peut pas être accepté.</p>
      {/if}
      <div class="actions">
        <button class="btn" type="button" onclick={() => flow.decline()}>Annuler</button>
        {#if problem.can_trust}
          <button
            class="btn btn-danger"
            type="button"
            disabled={problem.fingerprint_changed && !acknowledgeChange}
            onclick={() => {
              acknowledgeChange = false;
              void flow.trustCertificate();
            }}
          >
            Faire confiance à ce serveur
          </button>
        {/if}
      </div>
    </div>
  {:else if flow.step === "plaintext" && flow.plaintextEndpoint}
    {@const endpoint = flow.plaintextEndpoint}
    <div class="warning" role="alertdialog" aria-labelledby="plain-title">
      <h3 id="plain-title"><LockOpenIcon size={18} aria-hidden="true" /> Connexion non chiffrée</h3>
      <p class="danger">
        La connexion à {endpoint.host}:{endpoint.port} ne serait pas chiffrée : votre mot de passe et
        vos messages circuleraient en clair et pourraient être lus ou modifiés sur le réseau.
      </p>
      <p>Si le serveur le propose, Norelly passera tout de même en TLS (STARTTLS).</p>
      <div class="actions">
        <button class="btn btn-primary" type="button" onclick={() => flow.decline()}
          >Modifier les paramètres</button
        >
        <button class="btn btn-danger" type="button" onclick={() => void flow.allowPlaintext()}>
          Autoriser la connexion non chiffrée
        </button>
      </div>
    </div>
  {:else if flow.step === "secrets" && flow.secretsFile}
    <div class="warning" role="alertdialog" aria-labelledby="secrets-title">
      <h3 id="secrets-title"><LockOpenIcon size={18} aria-hidden="true" /> Aucun trousseau</h3>
      <p class="danger">
        Aucun trousseau du système (GNOME Keyring, KWallet…) ne répond. Norelly peut enregistrer vos
        mots de passe dans un fichier <strong>non chiffré</strong>, lisible par votre seul compte
        utilisateur : quiconque accède à vos fichiers pourra les lire.
      </p>
      <p class="path">{flow.secretsFile}</p>
      <div class="actions">
        <button class="btn btn-primary" type="button" onclick={() => flow.decline()}>Annuler</button
        >
        <button class="btn btn-danger" type="button" onclick={() => void flow.allowFileSecrets()}>
          Enregistrer dans ce fichier
        </button>
      </div>
    </div>
  {/if}
</Modal>

<style>
  form {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  label {
    font-weight: var(--font-weight-medium);
  }

  .hint {
    margin: var(--space-1) 0 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }

  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-3);
  }

  .message {
    margin: 0 0 var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius);
    background: var(--color-warning-bg);
    color: var(--color-warning);
  }

  .progress ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .progress li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) 0;
  }

  :global(.spin) {
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  .warning h3 {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0 0 var(--space-3);
    color: var(--color-danger);
    font-size: var(--font-size-md);
  }

  .danger {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius);
    background: var(--color-danger-bg);
    color: var(--color-danger);
  }

  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: var(--space-1) var(--space-3);
  }

  dt {
    color: var(--color-text-muted);
  }

  dd {
    margin: 0;
  }

  .acknowledge,
  .confirm label {
    display: flex;
    gap: var(--space-2);
    align-items: center;
  }

  .fingerprint {
    font-family: var(--font-mono);
    font-size: var(--font-size-sm);
    word-break: break-all;
  }
</style>
