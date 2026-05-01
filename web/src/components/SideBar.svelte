<script lang="ts">
  import Dot from './Dot.svelte';
  import { router, status } from '../lib/stores.svelte';
  import type { Tab } from '../lib/types';

  interface Props {
    activeJobs?: number;
  }
  let { activeJobs = 0 }: Props = $props();

  type NavItem = { tab: Tab; label: string; glyph: string; group: string };
  const items: NavItem[] = [
    { group: 'VAULT',   tab: 'dashboard', label: 'Dashboard', glyph: '::' },
    { group: 'VAULT',   tab: 'ask',       label: 'Ask',       glyph: '*~' },
    { group: 'VAULT',   tab: 'chat',      label: 'Chat',      glyph: '**' },
    { group: 'VAULT',   tab: 'search',    label: 'Search',    glyph: '::' },
    { group: 'VAULT',   tab: 'documents', label: 'Documents', glyph: '::' },
    { group: 'DATA',    tab: 'ingest',    label: 'Ingest',    glyph: '⌘+' },
    { group: 'DATA',    tab: 'manage',    label: 'Manage',    glyph: '::' },
    { group: 'CLIENTS', tab: 'connect',   label: 'Connect',   glyph: '::' },
  ];

  // Group by section while preserving order.
  const groups = $derived.by(() => {
    const out: { name: string; items: NavItem[] }[] = [];
    for (const item of items) {
      const last = out[out.length - 1];
      if (!last || last.name !== item.group) out.push({ name: item.group, items: [item] });
      else last.items.push(item);
    }
    return out;
  });
</script>

<aside class="sidebar">
  {#each groups as g (g.name)}
    <div class="nav-label">{g.name}</div>
    {#each g.items as item (item.tab)}
      <button
        class="nav-item"
        class:active={router.tab === item.tab}
        type="button"
        onclick={() => router.set(item.tab)}
      >
        <span class="glyph">{item.glyph}</span>
        <span class="label">{item.label}</span>
        {#if item.tab === 'ingest' && activeJobs > 0}
          <span class="badge">{activeJobs}</span>
        {/if}
      </button>
    {/each}
  {/each}

  <div class="foot">
    <div class="row">
      <Dot tone={status.data?.embedding_available ? 'teal' : 'dim'} />
      <span><b>{status.data?.version ? 'v' + status.data.version : 'v—'}</b></span>
    </div>
    <div class="row sub">{status.data?.embedding_model ?? '—'}</div>
  </div>
</aside>

<style>
  .sidebar {
    grid-area: sidebar;
    background: var(--bg);
    border-right: 1px solid var(--border);
    padding: 20px 0;
    display: flex;
    flex-direction: column;
    position: sticky;
    top: 49px;
    height: calc(100vh - 49px);
    overflow-y: auto;
  }
  .nav-label {
    font-size: 10px;
    letter-spacing: 2.5px;
    color: var(--text-dim);
    padding: 0 24px;
    margin: 12px 0 8px;
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .nav-label::before { content: '■'; color: var(--amber); font-size: 9px; }
  .nav-label::after  { content: ''; flex: 1; border-bottom: 1px solid var(--border); }

  .nav-item {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 10px 24px;
    color: var(--text-dim);
    text-decoration: none;
    font-size: 11px;
    letter-spacing: 2.5px;
    font-weight: 700;
    text-transform: uppercase;
    border: none;
    background: transparent;
    border-left: 2px solid transparent;
    transition: color 120ms ease, background 120ms ease, border-color 120ms ease;
    cursor: pointer;
    width: 100%;
    font-family: inherit;
    text-align: left;
  }
  .nav-item:hover { color: var(--text-bright); background: var(--surface); }
  .nav-item.active {
    color: var(--amber);
    background: var(--amber-soft);
    border-left-color: var(--amber);
  }
  .glyph { color: var(--text-dim); font-weight: 400; width: 14px; flex-shrink: 0; }
  .nav-item.active .glyph { color: var(--amber); }
  .badge {
    margin-left: auto;
    font-size: 9px;
    padding: 1px 6px;
    background: var(--surface-2);
    color: var(--text-dim);
    letter-spacing: 1px;
  }
  .nav-item.active .badge { color: var(--amber); background: var(--amber-soft); }

  .foot {
    margin-top: auto;
    padding: 16px 24px;
    border-top: 1px solid var(--border);
    font-size: 10px;
    letter-spacing: 1.5px;
    color: var(--text-dim);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .row { display: flex; align-items: center; gap: 8px; }
  .row b { color: var(--text); font-weight: 700; }
  .row.sub { font-size: 9px; }

  @media (max-width: 880px) {
    .sidebar { padding: 16px 0; }
    .nav-label { display: none; }
    .nav-item {
      padding: 12px 0;
      justify-content: center;
      border-left-width: 2px;
      gap: 0;
    }
    .glyph { width: auto; font-size: 14px; }
    .label, .badge { display: none; }
    .foot { display: none; }
  }
</style>
