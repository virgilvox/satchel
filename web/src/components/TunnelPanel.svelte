<script lang="ts">
  import { onMount } from 'svelte';
  import Dot from './Dot.svelte';
  import { api, type TunnelState } from '../lib/api';

  const initial: TunnelState = {
    installed: false,
    running: false,
    url: null,
    forwarding: null,
    started_at: null,
    error: null,
  };
  let tunnel = $state(initial);
  let busy = $state(false);
  let copied = $state(false);
  let mcpCopied = $state(false);
  let lastError: string | undefined = $state(undefined);

  async function refresh() {
    try {
      tunnel = await api.tunnelStatus();
    } catch {
      // backend unavailable — leave tunnel state alone
    }
  }

  // Poll while running so the URL appears as soon as cloudflared prints it.
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
      const r = await api.tunnelStart();
      // Server returns either the new state or { error }; both shapes are
      // handled — `error` is also a field on TunnelState so this is safe.
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
</script>

<div class="tunnel">
  <div class="head">
    <span class="title">PUBLIC TUNNEL</span>
    <span class="badge">Cloudflare</span>
    {#if tunnel.running && tunnel.url}
      <Dot tone="teal" />
      <span class="status running">live</span>
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
  {:else if !tunnel.running}
    <p class="desc">
      One click and SATCHEL gets a public <code>https://*.trycloudflare.com</code>
      URL pointing at <code>{tunnel.forwarding ?? 'http://localhost:7428'}</code>.
      No Cloudflare account needed — these are anonymous quick tunnels. <strong>The URL
      is public; anyone with it can hit your vault</strong>, so close the tunnel
      when you're done.
    </p>
    <div class="btn-row">
      <button class="btn btn-primary btn-sm" type="button" onclick={start} disabled={busy}>
        {busy ? 'STARTING…' : 'START QUICK TUNNEL'}
      </button>
    </div>
    {#if lastError}
      <p class="err">{lastError}</p>
    {/if}
  {:else if tunnel.running && tunnel.url}
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
    {#if tunnel.error}
      <p class="err">{tunnel.error}</p>
    {/if}
  {:else}
    <p class="desc">Waiting for cloudflared to publish the URL…</p>
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
    gap: 10px;
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
  .title {
    color: var(--amber);
    font-weight: 700;
  }
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
</style>
