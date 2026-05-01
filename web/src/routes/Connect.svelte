<script lang="ts">
  import ViewHead from '../components/ViewHead.svelte';
  import { ORIGIN } from '../lib/api';

  type Client = 'claude-desktop' | 'claude-code' | 'cursor' | 'browser';
  let active: Client = $state('claude-desktop');
  let copied = $state(false);

  const configs: Record<Client, { title: string; instructions: string; code: string; note?: string }> = {
    'claude-desktop': {
      title: 'Claude Desktop',
      instructions: 'Add this to your Claude Desktop config (Settings → Developer → Edit Config):',
      code: JSON.stringify({ mcpServers: { satchel: { command: '/path/to/satchel', args: ['serve'] } } }, null, 2),
      note: 'Replace /path/to/satchel with the actual path to the binary. Run "satchel config" in your terminal to get the snippet with the path filled in.',
    },
    'claude-code': {
      title: 'Claude Code',
      instructions: 'Run this command in your terminal:',
      code: 'claude mcp add satchel -- /path/to/satchel serve',
      note: 'Replace /path/to/satchel with the actual binary path.',
    },
    'cursor': {
      title: 'Cursor',
      instructions: 'Add this to your Cursor MCP configuration:',
      code: JSON.stringify({ mcpServers: { satchel: { command: '/path/to/satchel', args: ['serve'] } } }, null, 2),
      note: 'Replace /path/to/satchel with the actual binary path.',
    },
    'browser': {
      title: 'claude.ai (web)',
      instructions: 'Claude.ai web does not natively support local MCP. Use a tunnel or fall back to Claude Desktop:',
      code:
        '# Option 1 — Use Claude Desktop (recommended for local MCP).\n' +
        '#   Claude Desktop talks directly to this binary over stdio.\n\n' +
        '# Option 2 — Expose this server over HTTPS via a tunnel:\n' +
        `cloudflared tunnel --url ${ORIGIN}\n` +
        '# or\n' +
        'ngrok http 7428\n\n' +
        '# Then add the tunnel URL as a Custom Connector in claude.ai\n' +
        '# Settings → Connectors, pointing at https://<tunnel>/mcp.',
      note: 'For ChatGPT, the same tunnel URL can back a Custom GPT action targeting /api/search.',
    },
  };

  async function copy() {
    try {
      await navigator.clipboard.writeText(configs[active].code);
      copied = true;
      setTimeout(() => (copied = false), 1500);
    } catch {}
  }
</script>

<ViewHead num="07" title={`CONNECT <span class="slash">/</span> AI CLIENTS`}
  desc="SATCHEL exposes the vault over MCP. Pick a client for setup details." />

<div class="tabs">
  {#each Object.entries(configs) as [key, cfg] (key)}
    <button class="tab" class:active={active === key} onclick={() => (active = key as Client)}>
      {cfg.title}
    </button>
  {/each}
</div>

<p class="instructions">{configs[active].instructions}</p>
<div class="block">
  <button class="copy" onclick={copy}>{copied ? 'COPIED' : 'COPY'}</button>
  <pre>{configs[active].code}</pre>
</div>
{#if configs[active].note}
  <p class="instructions sm">{configs[active].note}</p>
{/if}

<style>
  .tabs {
    display: flex; gap: 0; margin-bottom: 18px;
    border-bottom: 1px solid var(--border); flex-wrap: wrap;
  }
  .tab {
    padding: 10px 18px; background: transparent;
    border: none; border-bottom: 2px solid transparent;
    color: var(--text-dim); cursor: pointer;
    font-family: inherit; font-size: 10px; letter-spacing: 2px;
    text-transform: uppercase; font-weight: 700;
    margin-bottom: -1px;
  }
  .tab:hover { color: var(--text-bright); }
  .tab.active { color: var(--amber); border-bottom-color: var(--amber); }

  .instructions {
    margin-bottom: 16px; color: var(--text-dim);
    font-size: 12px; line-height: 1.7;
  }
  .instructions.sm { font-size: 11px; margin-top: 14px; }

  .block {
    background: var(--bg-deep); border: 1px solid var(--border);
    padding: 16px 18px; position: relative;
  }
  .block pre {
    font-family: inherit; font-size: 12px; line-height: 1.6;
    overflow-x: auto; white-space: pre-wrap; color: var(--text);
  }
  .copy {
    position: absolute; top: 10px; right: 10px;
    padding: 5px 10px; font-size: 9px; letter-spacing: 1.5px;
    font-weight: 700; text-transform: uppercase;
    background: var(--surface); border: 1px solid var(--border);
    color: var(--text-dim); cursor: pointer; font-family: inherit;
  }
  .copy:hover { color: var(--amber); border-color: var(--amber-line); }
</style>
