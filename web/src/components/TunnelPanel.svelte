<script lang="ts">
  import { onMount } from 'svelte';
  import Dot from './Dot.svelte';
  import { api, type TunnelMode, type TunnelState } from '../lib/api';

  // ───── live tunnel state ─────
  const initial: TunnelState = {
    installed: false,
    running: false,
    mode: 'quick',
    url: null,
    forwarding: null,
    started_at: null,
    error: null,
    named: { configured: false, hostname: null },
  };
  let tunnel = $state<TunnelState>(initial);

  // ───── UI-local state ─────
  let busy = $state(false);
  let copied = $state(false);
  let mcpCopied = $state(false);
  let lastError: string | undefined = $state(undefined);
  let selectedMode: TunnelMode = $state('quick');
  let editingNamed = $state(false);

  // Form fields (named-tunnel config). The token never round-trips
  // back from the server — paste once, save, gone from the form.
  let formHostname = $state('');
  let formToken = $state('');
  let saving = $state(false);

  async function refresh() {
    try {
      tunnel = await api.tunnelStatus();
      // Mirror the running tunnel's mode in the toggle so the UI stays
      // coherent across page refreshes that hit a live tunnel.
      if (tunnel.running) selectedMode = tunnel.mode;
    } catch {
      // backend unavailable — leave state alone
    }
  }

  let timer: number | undefined;
  onMount(() => {
    refresh();
    timer = window.setInterval(refresh, 1500);
    return () => {
      if (timer) clearInterval(timer);
    };
  });

  async function start() {
    if (busy) return;
    busy = true;
    lastError = undefined;
    try {
      const r = await api.tunnelStart(selectedMode);
      tunnel = r as TunnelState;
      const errorish = (r as { error?: string }).error;
      if (errorish) lastError = errorish;
    } catch (e) {
      lastError = (e as Error).message;
    } finally {
      busy = false;
    }
  }

  async function stop() {
    if (busy) return;
    busy = true;
    try {
      tunnel = await api.tunnelStop();
    } catch (e) {
      lastError = (e as Error).message;
    } finally {
      busy = false;
    }
  }

  async function saveNamed() {
    if (saving) return;
    saving = true;
    lastError = undefined;
    try {
      const r = await api.tunnelConfigSet({
        token: formToken.trim(),
        hostname: formHostname.trim(),
      });
      if (r.error) {
        lastError = r.error;
      } else {
        editingNamed = false;
        formToken = ''; // wipe the form-level copy of the token
        await refresh();
      }
    } catch (e) {
      lastError = (e as Error).message;
    } finally {
      saving = false;
    }
  }

  async function clearNamed() {
    if (saving) return;
    saving = true;
    try {
      await api.tunnelConfigClear();
      formToken = '';
      formHostname = '';
      editingNamed = false;
      await refresh();
    } catch (e) {
      lastError = (e as Error).message;
    } finally {
      saving = false;
    }
  }

  function startEdit() {
    formHostname = tunnel.named?.hostname ?? '';
    formToken = '';
    editingNamed = true;
    lastError = undefined;
  }

  async function copy(text: string, which: 'public' | 'mcp') {
    try {
      await navigator.clipboard.writeText(text);
      if (which === 'public') {
        copied = true;
        setTimeout(() => (copied = false), 1500);
      } else {
        mcpCopied = true;
        setTimeout(() => (mcpCopied = false), 1500);
      }
    } catch {}
  }

  let mcpUrl = $derived(tunnel.url ? tunnel.url.replace(/\/$/, '') + '/mcp' : null);
  let namedConfigured = $derived(tunnel.named?.configured ?? false);
  let canStart = $derived(
    !busy &&
      !tunnel.running &&
      tunnel.installed &&
      (selectedMode === 'quick' || namedConfigured)
  );
</script>

<div class="tunnel">
  <div class="head">
    <span class="title">PUBLIC TUNNEL</span>
    <span class="badge">Cloudflare</span>
    {#if tunnel.running && tunnel.url}
      <Dot tone="teal" />
      <span class="status running">live · {tunnel.mode}</span>
    {:else if tunnel.running}
      <Dot tone="amber" pulse />
      <span class="status pending">starting…</span>
    {:else if tunnel.installed}
      <Dot tone="dim" />
      <span class="status idle">idle</span>
    {:else}
      <Dot tone="amber" />
      <span class="status missing">cloudflared not bundled / not in PATH</span>
    {/if}
  </div>

  {#if !tunnel.installed}
    <p class="desc">
      <code>cloudflared</code> isn't on this host. Release downloads bundle it
      automatically — if you built from source, install it via your package
      manager and refresh this tab:
    </p>
    <pre class="install"># macOS
brew install cloudflared

# Linux (Debian/Ubuntu)
curl -L --output cloudflared.deb https://github.com/cloudflare/cloudflared/releases/latest/download/cloudflared-linux-amd64.deb \
  && sudo dpkg -i cloudflared.deb

# Windows
winget install --id Cloudflare.cloudflared</pre>
  {:else if tunnel.running && tunnel.url}
    <!-- ─── Live tunnel ─── -->
    <p class="desc">
      Public URL is live — anyone with it can hit your vault. Use it as the
      Custom Connector URL in claude.ai or paste <code>{'<url>/mcp'}</code> into any
      MCP client that supports HTTP transport.
    </p>
    <div class="row">
      <span class="label">PUBLIC</span>
      <a class="url" href={tunnel.url} target="_blank" rel="noreferrer">{tunnel.url}</a>
      <button class="btn btn-secondary btn-sm" type="button" onclick={() => copy(tunnel.url!, 'public')}>
        {copied ? 'COPIED' : 'COPY'}
      </button>
    </div>
    {#if mcpUrl}
      <div class="row">
        <span class="label">MCP</span>
        <a class="url" href={mcpUrl} target="_blank" rel="noreferrer">{mcpUrl}</a>
        <button class="btn btn-secondary btn-sm" type="button" onclick={() => copy(mcpUrl!, 'mcp')}>
          {mcpCopied ? 'COPIED' : 'COPY'}
        </button>
      </div>
    {/if}
    <div class="btn-row">
      <button class="btn btn-danger btn-sm" type="button" onclick={stop} disabled={busy}>
        {busy ? 'STOPPING…' : 'STOP TUNNEL'}
      </button>
    </div>
    {#if tunnel.error}<p class="err">{tunnel.error}</p>{/if}
  {:else if tunnel.running}
    <!-- ─── Starting (waiting for URL / connection) ─── -->
    <p class="desc">Waiting for cloudflared to {tunnel.mode === 'named' ? 'register at the edge' : 'publish the URL'}…</p>
  {:else}
    <!-- ─── Idle ─── pick mode + START ─── -->
    <div class="mode-tabs" role="tablist">
      <button class="mode-tab" class:active={selectedMode === 'quick'} type="button"
        role="tab" aria-selected={selectedMode === 'quick'}
        onclick={() => (selectedMode = 'quick')}>
        Quick
        <span class="sub">random URL · anonymous</span>
      </button>
      <button class="mode-tab" class:active={selectedMode === 'named'} type="button"
        role="tab" aria-selected={selectedMode === 'named'}
        onclick={() => (selectedMode = 'named')}>
        Named
        <span class="sub">stable URL · your account</span>
      </button>
    </div>

    {#if selectedMode === 'quick'}
      <p class="desc">
        One click and SATCHEL gets a public <code>https://*.trycloudflare.com</code>
        URL pointing at <code>{tunnel.forwarding ?? 'http://localhost:7428'}</code>.
        No Cloudflare account needed — these are anonymous quick tunnels. <strong>The URL
        is public; anyone with it can hit your vault</strong>, so close the tunnel
        when you're done.
      </p>
    {:else}
      <p class="desc">
        Persistent tunnel on a hostname you control (your domain or
        <code>*.cfargotunnel.com</code>). Configure it once in Cloudflare Zero Trust →
        Networks → Tunnels, then paste the connector token below. The hostname
        you set in the dashboard is your stable URL across restarts.
      </p>
    {/if}

    {#if selectedMode === 'named'}
      {#if namedConfigured && !editingNamed}
        <div class="row">
          <span class="label">HOST</span>
          <span class="url" title="Configured public hostname">{tunnel.named?.hostname}</span>
          <button class="btn btn-secondary btn-sm" type="button" onclick={startEdit}>EDIT</button>
          <button class="btn btn-secondary btn-sm" type="button" onclick={clearNamed} disabled={saving}>CLEAR</button>
        </div>
      {:else}
        <div class="form">
          <label class="field">
            <span class="field-label">Public hostname</span>
            <input type="text" class="input" placeholder="vault.example.com"
              bind:value={formHostname} autocomplete="off" spellcheck="false" />
          </label>
          <label class="field">
            <span class="field-label">Connector token</span>
            <input type="password" class="input" placeholder="eyJh… (from Cloudflare Zero Trust → Tunnels)"
              bind:value={formToken} autocomplete="off" spellcheck="false" />
            <span class="hint">
              Stored at <code>&lt;vault&gt;/tunnel.toml</code> with 0600 permissions. Never
              sent back to the browser after save.
            </span>
          </label>
          <div class="btn-row">
            <button class="btn btn-primary btn-sm" type="button" onclick={saveNamed}
              disabled={saving || !formToken.trim() || !formHostname.trim()}>
              {saving ? 'SAVING…' : 'SAVE'}
            </button>
            {#if editingNamed && namedConfigured}
              <button class="btn btn-secondary btn-sm" type="button" onclick={() => (editingNamed = false)} disabled={saving}>CANCEL</button>
            {/if}
          </div>
        </div>
      {/if}
    {/if}

    <div class="btn-row">
      <button class="btn btn-primary btn-sm" type="button" onclick={start} disabled={!canStart}>
        {busy ? 'STARTING…' : selectedMode === 'quick' ? 'START QUICK TUNNEL' : 'START NAMED TUNNEL'}
      </button>
      {#if selectedMode === 'named' && !namedConfigured}
        <span class="hint">Save a token + hostname above first.</span>
      {/if}
    </div>

    {#if lastError}<p class="err">{lastError}</p>{/if}
  {/if}
</div>

<style>
  .tunnel {
    border: 1px solid var(--border);
    background: var(--surface);
    padding: 16px 18px;
    margin-bottom: 24px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    font-size: 11px;
    letter-spacing: 2px;
    text-transform: uppercase;
  }
  .title { color: var(--amber); font-weight: 700; }
  .badge {
    color: var(--text-dim);
    border: 1px solid var(--border);
    padding: 2px 7px;
    font-size: 9px;
    letter-spacing: 1.5px;
  }
  .status { font-weight: 700; font-size: 10px; }
  .status.running { color: var(--teal); }
  .status.pending { color: var(--amber); }
  .status.missing { color: var(--amber); }
  .status.idle    { color: var(--text-dim); }

  .desc {
    font-size: 12px;
    color: var(--text-dim);
    line-height: 1.6;
    margin: 0;
  }
  .desc code {
    color: var(--teal);
    background: var(--teal-soft);
    padding: 1px 6px;
    font-size: 11px;
  }
  .desc strong { color: var(--danger); font-weight: 700; }

  .install {
    background: var(--bg-deep);
    border: 1px solid var(--border);
    padding: 10px 14px;
    font-family: inherit;
    font-size: 11px;
    line-height: 1.6;
    color: var(--text-dim);
    overflow-x: auto;
    margin: 0;
    white-space: pre;
  }

  .mode-tabs {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 8px;
  }
  .mode-tab {
    background: var(--bg-deep);
    border: 1px solid var(--border);
    color: var(--text);
    cursor: pointer;
    padding: 10px 14px;
    font-family: inherit;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 2px;
    text-transform: uppercase;
    text-align: left;
    display: flex;
    flex-direction: column;
    gap: 4px;
    transition: 120ms ease;
  }
  .mode-tab:hover { border-color: var(--border-strong); }
  .mode-tab.active {
    border-color: var(--amber);
    color: var(--amber);
    background: var(--amber-soft);
  }
  .mode-tab .sub {
    font-size: 9px;
    letter-spacing: 1.5px;
    color: var(--text-dim);
    font-weight: 500;
    text-transform: none;
  }
  .mode-tab.active .sub { color: var(--text); }

  .form {
    display: flex;
    flex-direction: column;
    gap: 12px;
    background: var(--bg-deep);
    border: 1px solid var(--border);
    padding: 14px;
  }
  .field { display: flex; flex-direction: column; gap: 6px; }
  .field-label {
    font-size: 9px;
    letter-spacing: 2px;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .hint {
    font-size: 10px;
    line-height: 1.5;
    color: var(--text-dim);
  }
  .hint code {
    color: var(--teal);
    background: var(--teal-soft);
    padding: 1px 5px;
    font-size: 10px;
  }

  .row {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
    background: var(--bg-deep);
    border: 1px solid var(--border);
    padding: 8px 10px;
  }
  .label {
    font-size: 9px;
    letter-spacing: 2px;
    text-transform: uppercase;
    color: var(--text-dim);
    flex-shrink: 0;
    width: 50px;
  }
  .url {
    color: var(--teal);
    text-decoration: none;
    border-bottom: 1px dotted var(--teal-soft);
    flex: 1;
    min-width: 0;
    word-break: break-all;
    font-size: 12px;
  }
  .url:hover { border-bottom-style: solid; }
  .err {
    color: var(--danger);
    font-size: 11px;
    margin: 0;
  }
  .btn-row { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
</style>
