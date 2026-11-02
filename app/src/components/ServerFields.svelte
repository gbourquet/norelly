<script lang="ts">
  import { m } from "../lib/paraglide/messages.js";
  // Saisie d'un serveur (hôte, port, sécurité, utilisateur).
  import type { SecurityDto, ServerDto } from "../lib/ipc/api";

  interface Props {
    legend: string;
    idPrefix: string;
    server: ServerDto;
    disabled?: boolean;
  }

  let { legend, idPrefix, server = $bindable(), disabled = false }: Props = $props();

  const DEFAULT_PORTS: Record<string, Record<SecurityDto, number>> = {
    imap: { tls: 993, starttls: 143, plain: 143 },
    smtp: { tls: 465, starttls: 587, plain: 25 },
  };

  function onSecurityChange(): void {
    const ports = DEFAULT_PORTS[idPrefix];
    if (ports !== undefined && Object.values(ports).includes(Number(server.port))) {
      server.port = ports[server.security];
    }
  }
</script>

<fieldset {disabled}>
  <legend>{legend}</legend>
  <div class="grid">
    <label for="{idPrefix}-host">{m.server_host()}</label>
    <input
      id="{idPrefix}-host"
      class="input"
      bind:value={server.host}
      autocomplete="off"
      spellcheck="false"
      required
    />
    <label for="{idPrefix}-port">{m.server_port()}</label>
    <input
      id="{idPrefix}-port"
      class="input"
      type="number"
      min="1"
      max="65535"
      bind:value={server.port}
      required
    />
    <label for="{idPrefix}-security">{m.server_security()}</label>
    <select
      id="{idPrefix}-security"
      class="input"
      bind:value={server.security}
      onchange={onSecurityChange}
    >
      <option value="tls">{m.server_security_tls()}</option>
      <option value="starttls">{m.server_security_starttls()}</option>
      <option value="plain">{m.server_security_plain()}</option>
    </select>
    <label for="{idPrefix}-username">{m.server_username()}</label>
    <input
      id="{idPrefix}-username"
      class="input"
      bind:value={server.username}
      autocomplete="off"
      spellcheck="false"
      required
    />
  </div>
</fieldset>

<style>
  fieldset {
    margin: 0 0 var(--space-3);
    padding: var(--space-3);
    border: var(--border-width) solid var(--color-border);
    border-radius: var(--radius);
  }

  legend {
    padding: 0 var(--space-1);
    font-weight: var(--font-weight-medium);
  }

  .grid {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: var(--space-2) var(--space-3);
    align-items: center;
  }
</style>
