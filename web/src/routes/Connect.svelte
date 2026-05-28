<script lang="ts">
  import { onMount } from 'svelte';
  import ViewHead from '../components/ViewHead.svelte';
  import TunnelPanel from '../components/TunnelPanel.svelte';
  import { api, type ConnectInfo, type MdnsState } from '../lib/api';

  // The Connect tab answers three different questions, so it's split
  // into three top tabs:
  //
  //   1. PUBLIC TUNNEL  — "How do I reach SATCHEL from claude.ai or
  //                       another machine off-network?" (default tab,
  //                       per user request that this be top-billed)
  //   2. LOCAL ADDRESSES — "What URL do I use from this machine, or
  //                        from my phone on the same Wi-Fi?"
  //   3. CLIENT SETUP   — "How do I wire up Claude Desktop / Code /
  //                        Cursor to this server?"
  //
  // Inside each tab, every COPY button copies EXACTLY the URL shown
  // next to it; no mixing of base vs. /mcp URLs under one label.

  type Tab = 'tunnel' | 'local' | 'clients';
  let activeTab = $state<Tab>('tunnel');

  let info = $state<ConnectInfo | null>(null);
  let infoLoading = $state(true);
  let infoError = $state<string | undefined>();
  let mdns = $state<MdnsState | null>(null);
  let mdnsBusy = $state(false);

  // Per-client snippet sub-tab.
  type Client = 'claude-desktop' | 'claude-code' | 'cursor' | 'browser';
  let activeClient = $state<Client>('claude-desktop');
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
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      copyTokens = { ...copyTokens, [key]: (copyTokens[key] ?? 0) + 1 };
      const token = copyTokens[key];
      setTimeout(() => {
        if (copyTokens[key] === token) {
          const { [key]: _drop, ...rest } = copyTokens;
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

  const safeBinary = $derived(info?.binary_path ?? '/path/to/satchel');

  // Loopback "this machine" URLs. Browser sees `window.location.host`
  // as authoritative; for the on-disk localhost label we use the
  // computed port from /api/connect-info so the value matches exactly
  // what the binary printed at startup (and what auto-open used).
  const localWeb = $derived(info ? `http://localhost:${info.port}` : '');
  const localMcp = $derived(info ? `http://localhost:${info.port}/mcp` : '');

  // Hostname URLs for "other devices on this network". Only render
  // when mDNS is on (otherwise satchel.local won't resolve).
  const hostWeb = $derived(info?.mdns.url ?? null);
  const hostMcp = $derived(info?.mdns.mcp_url ?? null);

  // LAN IP fallback. Always show when we could detect one — works
  // even when mDNS is off or the client OS can't resolve .local.
  const lanWeb = $derived(info?.lan_url ?? null);
  const lanMcp = $derived(info?.lan_mcp_url ?? null);

  // Same-machine browser is on the loopback origin (window.isSecureContext
  // is true). When the user visits via satchel.local or LAN IP, this
  // becomes false and WebGPU stops working. We surface this on the
  // Local Addresses tab next to the hostname row so the cause is
  // visible without having to debug Chat.
  let insecureNow = $state(false);
  $effect(() => {
    insecureNow = typeof window !== 'undefined' && !window.isSecureContext;
  });
  const currentHost = $derived(typeof window !== 'undefined' ? window.location.host : '');

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
        note: 'Cursor also uses stdio. The HTTP endpoints are for remote clients only.',
      },
      'browser': {
        title: 'claude.ai or ChatGPT (web)',
        instructions:
          'Browser-based AI products cannot reach a local stdio server directly. The PUBLIC TUNNEL tab publishes an HTTPS URL you can paste into claude.ai as a Custom Connector, or use as a ChatGPT custom-action target.',
        code: '',
        note: 'For local desktop use, prefer Claude Desktop. Tunnels expose your vault publicly while they are running.',
      },
    };
    return list;
  });

  let activeSnippet = $derived(snippets[activeClient]);
</script>

<ViewHead num="07" title={`CONNECT <span class="slash">/</span> AI CLIENTS`}
  desc="Three tabs, three jobs. PUBLIC TUNNEL publishes an HTTPS URL for tools that can only reach the internet. LOCAL ADDRESSES is what to type from this machine or another device on the same network. CLIENT SETUP has the exact snippets for Claude Desktop, Claude Code, and Cursor." />

<div class="tabs" role="tablist">
  <button class="tab" type="button" role="tab"
    aria-selected={activeTab === 'tunnel'}
    class:active={activeTab === 'tunnel'}
    onclick={() => (activeTab = 'tunnel')}>
    Public Tunnel
  </button>
  <button class="tab" type="button" role="tab"
    aria-selected={activeTab === 'local'}
    class:active={activeTab === 'local'}
    onclick={() => (activeTab = 'local')}>
    Local Addresses
  </button>
  <button class="tab" type="button" role="tab"
    aria-selected={activeTab === 'clients'}
    class:active={activeTab === 'clients'}
    onclick={() => (activeTab = 'clients')}>
    Client Setup
  </button>
</div>

{#if activeTab === 'tunnel'}
  <p class="primer">
    Publish an HTTPS URL anyone with the link can hit. Use this for claude.ai
    in a browser, for a friend's machine, or for your phone off the home
    network. Stop the tunnel when you are done; the URL is public while it
    is up.
  </p>
  <TunnelPanel />
{:else if activeTab === 'local'}
  {#if infoLoading}
    <p class="loading">Loading addresses...</p>
  {:else if infoError}
    <p class="err">{infoError}</p>
  {:else if info}
    <!-- Block 1: same machine -->
    <section class="block">
      <h3 class="sect">This machine</h3>
      <p class="primer">
        What to use from this laptop. Local AI tools on this machine should
        hit the loopback URL; WebGPU and the local-LLM Chat tab need this
        secure-context origin to work.
      </p>
      <div class="row" data-tone="primary">
        <span class="lbl">WEB UI</span>
        <span class="val">{localWeb}</span>
        <button class="copy" type="button" onclick={() => copy(localWeb, 'lwebui')}>
          {isCopied('lwebui') ? 'COPIED' : 'COPY'}
        </button>
      </div>
      <div class="row" data-tone="primary">
        <span class="lbl">MCP</span>
        <span class="val">{localMcp}</span>
        <button class="copy" type="button" onclick={() => copy(localMcp, 'lmcp')}>
          {isCopied('lmcp') ? 'COPIED' : 'COPY'}
        </button>
      </div>
    </section>

    <!-- Block 2: other devices on the LAN -->
    <section class="block">
      <h3 class="sect">Other devices on this network</h3>
      <p class="primer">
        Use these from your phone, tablet, or another laptop on the same
        Wi-Fi. macOS resolves <code>satchel.local</code> natively; Windows 10
        and newer resolve it via the built-in DNS client; Linux desktops
        need <code>nss-mdns</code> or <code>avahi-daemon</code>. If
        <code>.local</code> does not resolve, use the LAN IP row below.
      </p>

      {#if insecureNow && currentHost && !currentHost.startsWith('localhost') && !currentHost.startsWith('127.0.0.1')}
        <p class="warn">
          Heads up: you are currently viewing SATCHEL at <code>{currentHost}</code>,
          which is not a secure-context origin. WebGPU (the local-LLM Chat
          tab) only works at <code>localhost</code> or <code>127.0.0.1</code>.
          Switch the URL in your browser to the WEB UI row at the top to use
          Chat.
        </p>
      {/if}

      {#if hostWeb}
        <div class="row" data-tone="teal">
          <span class="lbl">WEB UI</span>
          <span class="val">{hostWeb}</span>
          <button class="copy" type="button" onclick={() => copy(hostWeb, 'hwebui')}>
            {isCopied('hwebui') ? 'COPIED' : 'COPY'}
          </button>
        </div>
        <div class="row" data-tone="teal">
          <span class="lbl">MCP</span>
          <span class="val">{hostMcp}</span>
          <button class="copy" type="button" onclick={() => copy(hostMcp ?? '', 'hmcp')}>
            {isCopied('hmcp') ? 'COPIED' : 'COPY'}
          </button>
        </div>
      {:else}
        <p class="hint">
          mDNS is off, so <code>satchel.local</code> is not being broadcast.
          Toggle it on below to give other devices a stable hostname.
        </p>
      {/if}

      {#if lanWeb}
        <div class="row">
          <span class="lbl">LAN WEB</span>
          <span class="val">{lanWeb}</span>
          <button class="copy" type="button" onclick={() => copy(lanWeb, 'lanwebui')}>
            {isCopied('lanwebui') ? 'COPIED' : 'COPY'}
          </button>
        </div>
        <div class="row">
          <span class="lbl">LAN MCP</span>
          <span class="val">{lanMcp}</span>
          <button class="copy" type="button" onclick={() => copy(lanMcp ?? '', 'lanmcp')}>
            {isCopied('lanmcp') ? 'COPIED' : 'COPY'}
          </button>
        </div>
      {/if}

      <div class="mdns-toggle">
        <label class="switch">
          <input type="checkbox"
            checked={mdns?.enabled ?? info.mdns.enabled}
            disabled={mdnsBusy}
            onchange={(e) => toggleMdns((e.target as HTMLInputElement).checked)} />
          <span>Broadcast satchel.local on this network</span>
        </label>
        <p class="hint">
          When on, SATCHEL advertises itself over multicast DNS so other
          devices on the same network can reach it by hostname. When off,
          only the LAN IP works (and only on this network). The setting is
          persisted at <code>&lt;vault&gt;/mdns.toml</code>.
        </p>
      </div>
    </section>
  {/if}
{:else if activeTab === 'clients'}
  <p class="primer">
    Pick a client. Claude Desktop, Claude Code, and Cursor talk to SATCHEL
    over stdio (no HTTP server needed). The binary path is auto-filled from
    the running server, so the snippet is ready to paste as-is.
  </p>

  <div class="client-tabs">
    {#each Object.entries(snippets) as [key, cfg] (key)}
      <button class="client-tab" type="button"
        class:active={activeClient === key}
        onclick={() => (activeClient = key as Client)}>
        {cfg.title}
      </button>
    {/each}
  </div>

  <p class="instructions">{activeSnippet.instructions}</p>

  {#if activeSnippet.code}
    <div class="snippet">
      <button class="copy" type="button"
        onclick={() => copy(activeSnippet.code, 'snippet')}>
        {isCopied('snippet') ? 'COPIED' : 'COPY'}
      </button>
      <pre>{activeSnippet.code}</pre>
    </div>
  {/if}

  <p class="instructions sm">{activeSnippet.note}</p>
{/if}

<style>
  .tabs {
    display: flex;
    gap: 0;
    margin-bottom: 18px;
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }
  .tab {
    padding: 12px 22px;
    background: transparent;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-family: inherit;
    font-size: 11px;
    letter-spacing: 2px;
    text-transform: uppercase;
    font-weight: 700;
    margin-bottom: -1px;
    transition: 120ms ease;
  }
  .tab:hover {
    color: var(--text-bright);
  }
  .tab.active {
    color: var(--amber);
    border-bottom-color: var(--amber);
  }

  .primer {
    margin: 0 0 18px;
    color: var(--text-dim);
    font-size: 12px;
    line-height: 1.7;
  }
  .primer code {
    color: var(--teal);
    background: var(--teal-soft);
    padding: 1px 6px;
    font-size: 11px;
  }
  .hint {
    margin: 8px 0 0;
    color: var(--text-dim);
    font-size: 11px;
    line-height: 1.6;
  }
  .hint code {
    color: var(--teal);
    background: var(--teal-soft);
    padding: 1px 5px;
    font-size: 10px;
  }
  .warn {
    margin: 0 0 14px;
    padding: 10px 12px;
    background: var(--amber-soft);
    border: 1px solid var(--amber-line);
    color: var(--text);
    font-size: 11px;
    line-height: 1.6;
  }
  .warn code {
    color: var(--amber);
    background: transparent;
    padding: 0;
    font-weight: 700;
  }
  .err {
    color: var(--danger);
    font-size: 12px;
  }
  .loading {
    color: var(--text-dim);
    font-size: 12px;
  }

  /* ───────── address blocks on the Local Addresses tab ───────── */

  .block {
    margin-bottom: 28px;
  }
  .sect {
    margin: 0 0 8px;
    font-size: 10px;
    letter-spacing: 3px;
    text-transform: uppercase;
    color: var(--amber);
    font-weight: 700;
  }

  .row {
    display: grid;
    grid-template-columns: 100px 1fr auto;
    gap: 12px;
    align-items: center;
    padding: 10px 14px;
    background: var(--bg-deep);
    border: 1px solid var(--border);
    margin-bottom: 6px;
  }
  .row[data-tone='primary'] {
    border-color: var(--amber);
    background: var(--amber-soft);
  }
  .row[data-tone='teal'] {
    border-color: var(--teal);
    background: var(--teal-soft);
  }
  .lbl {
    font-size: 9px;
    letter-spacing: 2px;
    text-transform: uppercase;
    color: var(--text-dim);
    font-weight: 700;
  }
  .row[data-tone='primary'] .lbl { color: var(--amber); }
  .row[data-tone='teal'] .lbl { color: var(--teal); }
  .val {
    font-family: inherit;
    font-size: 12px;
    color: var(--text-bright);
    word-break: break-all;
    user-select: all;
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
    font-weight: 700;
    color: var(--text-bright);
    cursor: pointer;
  }
  .switch input {
    accent-color: var(--amber);
  }

  /* ───────── client-setup tab ───────── */

  .client-tabs {
    display: flex;
    gap: 0;
    margin-bottom: 14px;
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }
  .client-tab {
    padding: 8px 14px;
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
  .client-tab:hover {
    color: var(--text-bright);
  }
  .client-tab.active {
    color: var(--amber);
    border-bottom-color: var(--amber);
  }

  .instructions {
    margin-bottom: 12px;
    color: var(--text-dim);
    font-size: 12px;
    line-height: 1.7;
  }
  .instructions.sm {
    font-size: 11px;
    margin-top: 12px;
  }

  .snippet {
    background: var(--bg-deep);
    border: 1px solid var(--border);
    padding: 16px 18px;
    position: relative;
  }
  .snippet pre {
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
  .row .copy {
    padding: 4px 10px;
  }
  .snippet .copy {
    position: absolute;
    top: 10px;
    right: 10px;
  }
  .copy:hover {
    color: var(--amber);
    border-color: var(--amber-line);
  }
</style>
