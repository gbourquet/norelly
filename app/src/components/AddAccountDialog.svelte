<script lang="ts">
  // Parcours d'ajout de compte (voir lib/add-account.svelte.ts).
  import CheckIcon from "@lucide/svelte/icons/check";
  import LoaderIcon from "@lucide/svelte/icons/loader-circle";
  import ShieldAlertIcon from "@lucide/svelte/icons/shield-alert";
  import LockOpenIcon from "@lucide/svelte/icons/lock-open";
  import KeyRoundIcon from "@lucide/svelte/icons/key-round";
  import ExternalLinkIcon from "@lucide/svelte/icons/external-link";
  import Modal from "./Modal.svelte";
  import ServerFields from "./ServerFields.svelte";
  import { AddAccountFlow, sourceLabel, issueText } from "../lib/add-account.svelte";
  import { m } from "../lib/paraglide/messages.js";
  import type { AccountApi, AccountDto, OAuthProviderDto } from "../lib/ipc/api";

  interface Props {
    open: boolean;
    api: AccountApi;
    onClose: () => void;
    /** `notice` : message à montrer après l'ajout (envoi non vérifié). */
    onAdded: (account: AccountDto, notice: string | null) => void;
  }

  let { open, api, onClose, onAdded }: Props = $props();

  // L'API ne change pas pendant la vie du composant.
  // svelte-ignore state_referenced_locally
  const flow = new AddAccountFlow(api);
  /** Libellés de « Se connecter avec … », par fournisseur. */
  const providerText: Record<
    "signIn" | "hint" | "waiting" | "testing",
    Record<OAuthProviderDto, () => string>
  > = {
    signIn: { microsoft: m.add_oauth_sign_in_microsoft, google: m.add_oauth_sign_in_google },
    hint: { microsoft: m.add_oauth_hint_microsoft, google: m.add_oauth_hint_google },
    waiting: { microsoft: m.add_oauth_waiting_microsoft, google: m.add_oauth_waiting_google },
    testing: { microsoft: m.add_oauth_testing_microsoft, google: m.add_oauth_testing_google },
  };
  /** Confirmation renforcée exigée quand l'empreinte a changé. */
  let acknowledgeChange = $state(false);

  function close(): void {
    flow.reset();
    onClose();
  }

  $effect(() => {
    if (flow.step === "done" && flow.added !== null) {
      const account = flow.added;
      const notice = flow.addedNotice;
      flow.reset();
      onAdded(account, notice);
    }
  });

  // La garantie « trousseau du système » n'est affichée que sur la
  // réponse de Rust : avec le fichier de repli, elle serait fausse.
  $effect(() => {
    if (open) void flow.loadSecretsBackend();
  });
</script>

<Modal {open} title={m.add_title()} onClose={close} locked={flow.busy} width="560px">
  {#if flow.message}
    <p class="message" role="alert">{flow.message}</p>
  {/if}

  {#if flow.appPasswordHelp && (flow.step === "form" || flow.step === "manual")}
    <section class="app-password" aria-labelledby="app-password-title">
      <h3 id="app-password-title">
        <KeyRoundIcon size={18} aria-hidden="true" />
        {m.add_app_password_title()}
      </h3>
      <ol>
        <li>{m.add_app_password_step_2fa()}</li>
        <li>
          {m.add_app_password_step_create()}
          <button class="btn" type="button" onclick={() => void flow.openAppPasswords()}>
            <ExternalLinkIcon size={14} aria-hidden="true" />
            {m.add_app_password_open()}
          </button>
        </li>
        <li>{m.add_app_password_step_paste()}</li>
      </ol>
      <p class="hint">{m.add_app_password_unavailable()}</p>
    </section>
  {/if}

  {#if flow.step === "form"}
    <form
      onsubmit={(e) => {
        e.preventDefault();
        void (flow.signInProvider ? flow.signInWithProvider() : flow.submit());
      }}
    >
      <label for="account-address">{m.add_address()}</label>
      <input
        id="account-address"
        class="input"
        type="email"
        bind:value={flow.address}
        oninput={() => void flow.detectProvider()}
        autocomplete="email"
        spellcheck="false"
        required
      />
      {#if flow.signInProvider === null}
        <label for="account-password">{m.add_password()}</label>
        <input
          id="account-password"
          class="input"
          type="password"
          bind:value={flow.password}
          autocomplete="current-password"
          required
        />
      {/if}
      <label for="account-name">{m.add_display_name()}</label>
      <input id="account-name" class="input" bind:value={flow.displayName} autocomplete="name" />
      {#if flow.signInProvider !== null}
        <p class="hint">{providerText.hint[flow.signInProvider]()}</p>
      {:else if flow.secretsBackend === "system_keyring"}
        <p class="hint">
          {m.add_password_hint()}
        </p>
      {/if}
      <div class="actions">
        <button
          class="btn"
          type="button"
          onclick={() => {
            flow.step = "manual";
          }}>{m.add_manual()}</button
        >
        {#if flow.signInProvider !== null && flow.canUsePassword}
          <button class="btn" type="button" onclick={() => flow.usePassword()}>
            <KeyRoundIcon size={14} aria-hidden="true" />
            {m.add_use_app_password()}
          </button>
        {:else if flow.signInProvider === null && flow.canUsePassword}
          <button class="btn" type="button" onclick={() => flow.useProviderSignIn()}>
            {m.add_use_google_sign_in()}
          </button>
        {/if}
        {#if flow.signInProvider !== null}
          <button class="btn btn-primary" type="submit"
            >{providerText.signIn[flow.signInProvider]()}</button
          >
        {:else}
          <button class="btn btn-primary" type="submit">{m.common_continue()}</button>
        {/if}
      </div>
    </form>
  {:else if flow.step === "discovering" || flow.step === "testing"}
    <div class="progress" aria-live="polite">
      {#if flow.step === "discovering"}
        <p>{m.add_discovering({ address: flow.address })}</p>
        <ul>
          {#each flow.progress as source, i (source)}
            <li>
              {#if i === flow.progress.length - 1}
                <LoaderIcon size={14} class="spin" aria-hidden="true" />
              {:else}
                <CheckIcon size={14} aria-hidden="true" />
              {/if}
              {sourceLabel(source)}
            </li>
          {/each}
        </ul>
      {:else if flow.waitingForBrowser}
        <p>
          <LoaderIcon size={14} class="spin" aria-hidden="true" />
          {flow.oauthProvider ? providerText.waiting[flow.oauthProvider]() : ""}
        </p>
      {:else if flow.reusingSignIn}
        <p>
          <LoaderIcon size={14} class="spin" aria-hidden="true" />
          {flow.oauthProvider ? providerText.testing[flow.oauthProvider]() : ""}
        </p>
      {:else}
        <p>
          <LoaderIcon size={14} class="spin" aria-hidden="true" />
          {m.add_testing({ imap: flow.imap.host, smtp: flow.smtp.host })}
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
        <label for="manual-address">{m.add_address()}</label>
        <input id="manual-address" class="input" type="email" bind:value={flow.address} required />
        <label for="manual-password">{m.add_password()}</label>
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
          <div
            class="danger confirm"
            role="group"
            aria-label={m.add_confirm_group({ host: item.host })}
          >
            <p>
              <strong>{item.host}</strong>
              {m.add_confirm_warning({ source: sourceLabel(item.source) })}
            </p>
            <label>
              <input
                type="checkbox"
                checked={flow.confirmed.includes(item.host)}
                onchange={(e) => flow.setConfirmed(item.host, e.currentTarget.checked)}
              />
              {m.add_confirm_checkbox({ host: item.host })}
            </label>
          </div>
        {/if}
      {/each}
      <ServerFields legend={m.add_imap_legend()} idPrefix="imap" bind:server={flow.imap} />
      <ServerFields legend={m.add_smtp_legend()} idPrefix="smtp" bind:server={flow.smtp} />
      <div class="actions">
        <button class="btn" type="button" onclick={close}>{m.common_cancel()}</button>
        <button class="btn btn-primary" type="submit">{m.add_test_and_add()}</button>
      </div>
    </form>
  {:else if flow.step === "certificate" && flow.certificate}
    {@const problem = flow.certificate}
    <div class="warning" role="alertdialog" aria-labelledby="cert-title">
      <h3 id="cert-title">
        <ShieldAlertIcon size={18} aria-hidden="true" />
        {m.add_cert_title()}
      </h3>
      {#if problem.fingerprint_changed}
        <p class="danger">
          <strong>{m.add_cert_changed_label()}</strong>
          {m.add_cert_changed()}
        </p>
      {/if}
      <p>{issueText(problem.issue)}</p>
      <dl>
        <dt>{m.add_cert_server()}</dt>
        <dd>{problem.host} ({problem.protocol.toUpperCase()})</dd>
        <dt>{m.add_cert_fingerprint()}</dt>
        <dd class="fingerprint">{problem.fingerprint}</dd>
      </dl>
      {#if problem.can_trust && problem.fingerprint_changed}
        <label class="acknowledge">
          <input type="checkbox" bind:checked={acknowledgeChange} />
          {m.add_cert_acknowledge()}
        </label>
      {/if}
      {#if problem.can_trust}
        <p>
          {m.add_cert_trust_advice()}
        </p>
      {:else}
        <p class="danger">{m.add_cert_cannot_trust()}</p>
      {/if}
      <div class="actions">
        <button class="btn" type="button" onclick={() => flow.decline()}>{m.common_cancel()}</button
        >
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
            {m.add_cert_trust()}
          </button>
        {/if}
      </div>
    </div>
  {:else if flow.step === "plaintext" && flow.plaintextEndpoint}
    {@const endpoint = flow.plaintextEndpoint}
    <div class="warning" role="alertdialog" aria-labelledby="plain-title">
      <h3 id="plain-title">
        <LockOpenIcon size={18} aria-hidden="true" />
        {m.add_plain_title()}
      </h3>
      <p class="danger">
        {m.add_plain_warning({ host: endpoint.host, port: endpoint.port })}
      </p>
      <p>{m.add_plain_starttls()}</p>
      <div class="actions">
        <button class="btn btn-primary" type="button" onclick={() => flow.decline()}
          >{m.add_plain_edit()}</button
        >
        <button class="btn btn-danger" type="button" onclick={() => void flow.allowPlaintext()}>
          {m.add_plain_allow()}
        </button>
      </div>
    </div>
  {:else if flow.step === "secrets" && flow.secretsFile}
    <div class="warning" role="alertdialog" aria-labelledby="secrets-title">
      <h3 id="secrets-title">
        <LockOpenIcon size={18} aria-hidden="true" />
        {m.add_secrets_title()}
      </h3>
      <p class="danger">
        {m.add_secrets_warning_before()}
        <strong>{m.add_secrets_warning_strong()}</strong>{m.add_secrets_warning_after()}
      </p>
      <p class="path">{flow.secretsFile}</p>
      <div class="actions">
        <button class="btn btn-primary" type="button" onclick={() => flow.decline()}
          >{m.common_cancel()}</button
        >
        <button class="btn btn-danger" type="button" onclick={() => void flow.allowFileSecrets()}>
          {m.add_secrets_allow()}
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

  .app-password {
    margin: 0 0 var(--space-3);
    padding: var(--space-3);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius);
    background: var(--color-surface);
  }

  .app-password h3 {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0 0 var(--space-2);
    font-size: var(--font-size-md);
  }

  .app-password ol {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding-left: var(--space-5);
  }

  .app-password li .btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    margin-top: var(--space-1);
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
