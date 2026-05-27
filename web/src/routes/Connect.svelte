<script lang="ts">
  import { onMount } from 'svelte';
  import ViewHead from '../components/ViewHead.svelte';
  import TunnelPanel from '../components/TunnelPanel.svelte';
  import { api, type ConnectInfo, type MdnsState } from '../lib/api';

  // The Connect tab is the first place a new user lands after setup,
  // so it has to answer one question fast: "what address do I point my
  // AI tool at?" We lead with the local MCP URL, surface the
  // satchel.local hostname (when mDNS is up), and only then fan out
  // into per-client setup snippets. The Cloudflare tunnel sits at the
  // bottom because remote access is the secondary use case.

  let info = $state<ConnectInfo | null>(null);
  let infoLoading = $state(true);
  let infoError = $state<string | undefined>();
  let mdns = $state<MdnsState | null>(null);
  let mdnsBusy = $state(false);

  // Per-client snippet tab.
  type Client = 'claude-desktop' | 'claude-code' | 'cursor' | 'browser';
  let active = $state<Client>('claude-desktop');
  let copyTokens = $state<Record<string, number>>({});

  async function refresh() {
    try {
      const ci = await api.connectInfo();
      if ((ci as { error?: string }).error) {
        infoError = (ci as { error?: string }).error;
      } else {
        info = ci;
        infoError = undefined;
      }
    } catch (e) {
      infoError = (e as Error).message;
    } finally {
      infoLoading = false;
    }
    try {
      const m = await api.mdnsGet();
      if (!(m as { error?: string }).error) mdns = m;
    } catch {
      /* leave the previous value */
    }
  }

  async function toggleMdns(next: boolean) {
    if (mdnsBusy) return;
    mdnsBusy = true;
    try {
      const r = await api.mdnsSet(next);
      if (!r.error) {
        mdns = r;
        await refresh();
      }
    } finally {
      mdnsBusy = false;
    }
  }

  async function copy(text: string, key: string) {
    try {
      await navigator.clipboard.writeText(text);
      copyTokens = { ...copyTokens, [key]: (copyTokens[key] ?? 0) + 1 };
      const token = copyTokens[key];
      setTimeout(() => {
        if (copyTokens[key] === token) {
          const { [key]: _, ...rest } = copyTokens;
          copyTokens = rest;
        }
      }, 1500);
    } catch {
      /* clipboard blocked; the URL is still visible on screen */
    }
  }

  function isCopied(key: string): boolean {
    return (copyTokens[key] ?? 0) > 0;
  }

  onMount(refresh);

  // Build the per-client snippet from the live connect-info payload so
  // every snippet has the real binary path baked in. Falls back to a
  // safe placeholder while the first /api/connect-info request is in
  // flight.
  const safeBinary = $derived(info?.binary_path ?? '/path/to/satchel');

  const snippets = $derived.by(() => {
    const bin = safeBinary;
    const list: Record<
      Client,
      { title: string; instructions: string; code: string; note: string }
    > = {
      'claude-desktop': {
        title: 'Claude Desktop',
        instructions:
          'In Claude Desktop, open Settings, choose Developer, then Edit Config. Paste this block into the JSON file and restart Claude Desktop.',
        code: JSON.stringify(
          { mcpServers: { satchel: { command: bin, args: ['serve'] } } },
          null,
          2,
        ),
        note: 'Claude Desktop talks to SATCHEL over stdio. No HTTP server has to be running for this client to work.',
      },
      'claude-code': {
        title: 'Claude Code',
        instructions: 'Run this once in a terminal. Claude Code remembers the entry afterwards.',
        code: `claude mcp add satchel -- ${bin} serve`,
        note: 'Same stdio transport as Claude Desktop, just registered through the Claude Code CLI.',
      },
      'cursor': {
        title: 'Cursor',
        instructions:
          'In Cursor, open Settings, then MCP, then Add New Server. Paste this block.',
        code: JSON.stringify(
          { mcpServers: { satchel: { command: bin, args: ['serve'] } } },
          null,
          2,
        ),
        note: 'Cursor also uses stdio. The HTTP server below is for remote clients only.',
      },
      'browser': {
        title: 'claude.ai or ChatGPT (web)',
        instructions:
          'claude.ai and ChatGPT in the browser cannot reach a local MCP endpoint directly. The tunnel panel further down publishes a public HTTPS URL that you can paste into claude.ai as a Custom Connector or use as a ChatGPT custom-action target.',
        code: '',
        note: 'For local desktop use, prefer Claude Desktop. Tunnels expose your vault publicly while they are running.',
      },
    };
    return list;
  });

  let activeSnippet = $derived(snippets[active]);
</script>

<ViewHead num="07" title={`CONNECT <span class="slash">/</span> AI CLIENTS`}
  desc="One page, three layers. Local clients talk to SATCHEL over stdio. Other devices on this network use the LAN address or satchel.local. Remote tools come in over a Cloudflare tunnel." />

<!--
  Layer 1: where SATCHEL is reachable RIGHT NOW.
  This is the answer to "what URL do I use?" and gets top billing.
-->
<section class="addr">
  <h2 class="sect-title">LOCAL MCP ENDPOINT</h2>
  {#if infoLoading}
    <p class="loading">Loading addresses...</p>
  {:else if infoError}
    <p class="err">{infoError}</p>
  {:else if info}
    <p class="primer">
      This SATCHEL server is reachable at the addresses below. Local AI tools on this
      machine should use the loopback URL. Other devices on the same network should
      use satchel.local (when mDNS is on) or the LAN IP.
    </p>

    <div class="addr-row" data-tone="primary">
      <span class="addr-label">LOCAL MCP</span>
      <span class="addr-value">{info.mcp_url}</span>
      <button class="copy" type="button" onclick={() => copy(info!.mcp_url, 'mcp')}>
        {isCopied('mcp') ? 'COPIED' : 'COPY'}
      </button>
    </div>
    <div class="addr-row">
      <span class="addr-label">WEB UI</span>
      <span class="addr-value">{info.local_url}</span>
      <button class="copy" type="button" onclick={() => copy(info!.local_url, 'web')}>
        {isCopied('web') ? 'COPIED' : 'COPY'}
      </button>
    </div>

    {#if info.mdns.url}
      <div class="addr-row" data-tone="teal">
        <span class="addr-label">SATCHEL.LOCAL</span>
        <span class="addr-value">{info.mdns.mcp_url}</span>
        <button class="copy" type="button"
          onclick={() => copy(info!.mdns.mcp_url ?? '', 'mdns')}>
          {isCopied('mdns') ? 'COPIED' : 'COPY'}
        </button>
      </div>
      <p class="addr-hint">
        mDNS is on. Any phone, laptop, or other device on this network can reach
        {info.mdns.hostname} without knowing the IP. macOS resolves this out of
        the box. Windows 10 and newer resolve it through the built-in DNS client.
        Most Linux desktops resolve it through nss-mdns or Avahi; if it does not
        resolve there, use the LAN IP below.
      </p>
    {:else}
      <p class="addr-hint">
        mDNS is off, so satchel.local will not resolve. Turn it on below to give
        other devices on this network a stable hostname.
      </p>
    {/if}

    {#if info.lan_url}
      <div class="addr-row">
        <span class="addr-label">LAN IP</span>
        <span class="addr-value">{info.lan_mcp_url}</span>
        <button class="copy" type="button"
          onclick={() => copy(info!.lan_mcp_url ?? '', 'lan')}>
          {isCopied('lan') ? 'COPIED' : 'COPY'}
        </button>
      </div>
      <p class="addr-hint">
        The LAN address always works on the local network. It may change after a
        router reboot or when you move between Wi-Fi networks.
      </p>
    {/if}

    <div class="mdns-toggle">
      <label class="switch">
        <input type="checkbox"
          checked={mdns?.enabled ?? info.mdns.enabled}
          disabled={mdnsBusy}
          onchange={(e) => toggleMdns((e.target as HTMLInputElement).checked)} />
        <span>BROADCAST satchel.local on this network</span>
      </label>
      <p class="hint">
        When on, SATCHEL advertises itself over multicast DNS so other devices
        on the same network can reach it by hostname. When off, only the IP
        addresses above work, and only on this network. The setting is
        persisted at vault/mdns.toml.
      </p>
    </div>
  {/if}
</section>

<!--
  Layer 2: per-client setup snippets with the real binary path baked in.
-->
<section class="clients">
  <h2 class="sect-title">SET UP A LOCAL CLIENT</h2>
  <p class="primer">
    Claude Desktop, Claude Code, and Cursor talk to SATCHEL over stdio, not over
    the HTTP server above. The HTTP server stays useful for remote clients and
    the web UI. Pick a client below for the exact snippet.
  </p>

  <div class="tabs">
    {#each Object.entries(snippets) as [key, cfg] (key)}
      <button class="tab" class:active={active === key}
        type="button" onclick={() => (active = key as Client)}>
        {cfg.title}
      </button>
    {/each}
  </div>

  <p class="instructions">{activeSnippet.instructions}</p>

  {#if activeSnippet.code}
    <div class="block">
      <button class="copy" type="button"
        onclick={() => copy(activeSnippet.code, 'snippet')}>
        {isCopied('snippet') ? 'COPIED' : 'COPY'}
      </button>
      <pre>{activeSnippet.code}</pre>
    </div>
  {/if}

  <p class="instructions sm">{activeSnippet.note}</p>
</section>

<!--
  Layer 3: public access for browser-only clients and remote machines.
-->
<section class="remote">
  <h2 class="sect-title">REMOTE ACCESS</h2>
  <p class="primer">
    Need to reach SATCHEL from claude.ai in a browser, from a friend's machine,
    or from your phone off the home network? Publish a tunnel and use the
    public URL as the Custom Connector endpoint. Stop the tunnel when you are
    done; the URL is public while it is up.
  </p>
  <TunnelPanel />
</section>

<style>
  .sect-title {
    margin: 0 0 10px;
    font-size: 11px;
    letter-spacing: 3px;
    text-transform: uppercase;
    font-weight: 700;
    color: var(--amber);
  }

  section {
    margin-bottom: 40px;
  }

  .primer {
    margin: 0 0 18px;
    color: var(--text-dim);
    font-size: 12px;
    line-height: 1.7;
  }

  .loading,
  .err {
    margin: 0 0 12px;
    font-size: 12px;
  }
  .err { color: var(--danger); }
  .loading { color: var(--text-dim); }

  .addr-row {
    display: grid;
    grid-template-columns: 130px 1fr auto;
    gap: 12px;
    align-items: center;
    padding: 10px 14px;
    background: var(--bg-deep);
    border: 1px solid var(--border);
    margin-bottom: 6px;
  }
  .addr-row[data-tone='primary'] {
    border-color: var(--amber);
    background: var(--amber-soft);
  }
  .addr-row[data-tone='teal'] {
    border-color: var(--teal);
    background: var(--teal-soft);
  }
  .addr-label {
    font-size: 9px;
    letter-spacing: 2px;
    color: var(--text-dim);
    text-transform: uppercase;
    font-weight: 700;
  }
  .addr-row[data-tone='primary'] .addr-label { color: var(--amber); }
  .addr-row[data-tone='teal'] .addr-label { color: var(--teal); }
  .addr-value {
    font-family: inherit;
    font-size: 12px;
    color: var(--text-bright);
    word-break: break-all;
  }
  .addr-hint {
    margin: 4px 0 12px;
    color: var(--text-dim);
    font-size: 11px;
    line-height: 1.6;
  }

  .mdns-toggle {
    margin-top: 14px;
    padding-top: 14px;
    border-top: 1px dashed var(--border);
  }
  .switch {
    display: inline-flex;
    align-items: center;
    gap: 10px;
    font-size: 11px;
    letter-spacing: 1.5px;
    text-transform: uppercase;
    font-weight: 700;
    color: var(--text-bright);
    cursor: pointer;
  }
  .switch input { accent-color: var(--amber); }
  .hint {
    margin: 6px 0 0;
    font-size: 11px;
    line-height: 1.6;
    color: var(--text-dim);
  }

  .tabs {
    display: flex;
    gap: 0;
    margin-bottom: 14px;
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }
  .tab {
    padding: 10px 18px;
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-family: inherit;
    font-size: 10px;
    letter-spacing: 2px;
    text-transform: uppercase;
    font-weight: 700;
    margin-bottom: -1px;
  }
  .tab:hover { color: var(--text-bright); }
  .tab.active {
    color: var(--amber);
    border-bottom-color: var(--amber);
  }

  .instructions {
    margin-bottom: 12px;
    color: var(--text-dim);
    font-size: 12px;
    line-height: 1.7;
  }
  .instructions.sm { font-size: 11px; margin-top: 12px; }

  .block {
    background: var(--bg-deep);
    border: 1px solid var(--border);
    padding: 16px 18px;
    position: relative;
  }
  .block pre {
    font-family: inherit;
    font-size: 12px;
    line-height: 1.6;
    overflow-x: auto;
    white-space: pre-wrap;
    color: var(--text);
    margin: 0;
  }
  .copy {
    background: var(--surface);
    border: 1px solid var(--border);
    color: var(--text-dim);
    cursor: pointer;
    font-family: inherit;
    font-size: 9px;
    letter-spacing: 1.5px;
    font-weight: 700;
    text-transform: uppercase;
    padding: 5px 10px;
  }
  .addr-row .copy { padding: 4px 10px; }
  .block .copy {
    position: absolute;
    top: 10px;
    right: 10px;
  }
  .copy:hover { color: var(--amber); border-color: var(--amber-line); }
</style>
