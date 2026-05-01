<script lang="ts">
  import ViewHead from '../components/ViewHead.svelte';
  import { api } from '../lib/api';
  import type { FileTypeStat, SourceRow } from '../lib/types';

  let q = $state('');
  let type = $state('');
  let sort = $state('name');
  let sources = $state<SourceRow[]>([]);
  let total = $state(0);
  let loading = $state(true);
  let types = $state<FileTypeStat[]>([]);
  let debounce: number | undefined;

  const PAGE = 50;

  async function load() {
    loading = true;
    try {
      const r = await api.sources({ q, filter_type: type, sort_by: sort, limit: PAGE, offset: 0 });
      sources = r.sources ?? [];
      total = r.total ?? 0;
    } finally {
      loading = false;
    }
  }

  async function loadMore() {
    const r = await api.sources({ q, filter_type: type, sort_by: sort, limit: PAGE, offset: sources.length });
    sources = [...sources, ...(r.sources ?? [])];
    total = r.total ?? total;
  }

  async function loadTypes() {
    const r = await api.types();
    types = r.types ?? [];
  }

  function onQ(e: Event) {
    q = (e.target as HTMLInputElement).value;
    if (debounce) clearTimeout(debounce);
    debounce = window.setTimeout(load, 250);
  }

  $effect(() => {
    load();
    loadTypes();
  });

  let canMore = $derived(sources.length < total);
  let summary = $derived(
    sources.length === 0
      ? ''
      : `SHOWING ${sources.length} OF ${total} SOURCE PATH${total === 1 ? '' : 'S'}` +
        (q ? ` · MATCHING "${q.toUpperCase()}"` : '')
  );

  function fmtNum(n: number | undefined) {
    return (n ?? 0).toLocaleString();
  }
</script>

<ViewHead num="04" title={`DOCUMENTS <span class="slash">/</span> SOURCE INDEX`}
  desc="Every ingested source path with chunk and record counts. Filter, sort, drill in." />

<div class="row">
  <input type="text" class="input grow" placeholder="filter by path substring (case-sensitive)"
    value={q} oninput={onQ} />
  <select class="select fixed" bind:value={type} onchange={load}>
    <option value="">all types</option>
    {#each types as t (t.file_type)}
      <option value={t.file_type}>{t.file_type} ({t.source_count})</option>
    {/each}
  </select>
  <select class="select fixed" bind:value={sort} onchange={load}>
    <option value="name">sort: name</option>
    <option value="date">sort: newest first</option>
    <option value="chunks">sort: most chunks</option>
    <option value="records">sort: most records</option>
  </select>
</div>

<div class="summary">{summary}</div>

{#if loading && sources.length === 0}
  <div class="loading"><span class="glyph">::</span>loading documents...</div>
{:else if sources.length === 0}
  <div class="empty"><span class="glyph">::</span>no documents match. use the ingest tab to add files.</div>
{:else}
  <table class="doc-table">
    <thead><tr>
      <th>Type</th><th>Source</th><th>Records</th><th>Chunks</th><th>Ingested</th>
    </tr></thead>
    <tbody>
      {#each sources as s (s.path)}
        <tr>
          <td><span class="badge {s.file_type}">{s.file_type}</span></td>
          <td class="path">{s.path}</td>
          <td>{s.record_count > 1 ? `${fmtNum(s.record_count)} records` : '1 record'}</td>
          <td><span class="num">{fmtNum(s.chunk_count)}</span></td>
          <td>{s.ingested_at ?? '—'}</td>
        </tr>
      {/each}
    </tbody>
  </table>
  {#if canMore}
    <div class="more"><span class="info">{sources.length} / {total}</span>
      <button class="btn btn-secondary btn-sm" onclick={loadMore}>LOAD MORE</button>
    </div>
  {/if}
{/if}

<style>
  .row {
    display: flex; gap: 10px; flex-wrap: wrap; margin-bottom: 14px;
  }
  .grow { flex: 1; min-width: 220px; }
  .fixed { flex: 0 0 200px; }
  .summary {
    font-size: 10px; color: var(--text-dim); letter-spacing: 1.5px;
    margin-bottom: 10px; min-height: 1em;
  }
  .doc-table { width: 100%; border-collapse: collapse; }
  .doc-table th {
    text-align: left; padding: 10px 14px;
    font-size: 9px; color: var(--text-dim); text-transform: uppercase;
    letter-spacing: 2px; font-weight: 700;
    border-bottom: 1px solid var(--border); background: var(--surface);
  }
  .doc-table td {
    padding: 10px 14px; border-bottom: 1px solid var(--border);
    font-size: 12px; vertical-align: middle;
  }
  .doc-table tr:hover td { background: var(--surface); }
  .doc-table .path { color: var(--text); word-break: break-all; font-size: 11px; }
  .doc-table .num { color: var(--amber); font-weight: 700; letter-spacing: 1px; }

  .badge {
    display: inline-block;
    padding: 2px 7px; font-size: 9px; font-weight: 700;
    letter-spacing: 1.5px; text-transform: uppercase;
    background: var(--surface-2); color: var(--text-dim);
    border: 1px solid var(--border);
  }
  .badge.md   { color: var(--teal); border-color: var(--teal); background: var(--teal-soft); }
  .badge.pdf  { color: var(--danger); border-color: var(--danger); background: var(--danger-soft); }
  .badge.json { color: var(--amber); border-color: var(--amber-line); background: var(--amber-soft); }
  .badge.html { color: var(--teal); border-color: var(--teal); background: var(--teal-soft); }
  .badge.csv  { color: var(--teal); border-color: var(--teal); background: var(--teal-soft); }

  .more {
    display: flex; justify-content: space-between; align-items: center;
    padding: 14px 0; gap: 12px;
  }
  .info { font-size: 10px; color: var(--text-dim); letter-spacing: 1.5px; }
</style>
