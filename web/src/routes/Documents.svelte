<script lang="ts">
  import { onMount } from 'svelte';
  import ViewHead from '../components/ViewHead.svelte';
  import { api, type CollectionSummary } from '../lib/api';
  import type { FileTypeStat, SourceRow } from '../lib/types';

  let q = $state('');
  let type = $state('');
  let sort = $state('name');
  let collectionId: number | '' = $state('');
  let sources = $state<SourceRow[]>([]);
  let total = $state(0);
  // Whole-vault count, independent of collection / search filters. The ALL
  // tab displays this so it stays accurate when the user is filtered into a
  // collection. Refreshed alongside the filtered list.
  let vaultTotal = $state(0);
  let loading = $state(true);
  let types = $state<FileTypeStat[]>([]);
  let collections = $state<CollectionSummary[]>([]);
  let debounce: number | undefined;

  // ─── multi-select for bulk move-to-collection ───
  let selected = $state<Set<string>>(new Set());
  let collectionDialogOpen = $state(false);
  let collectionDialogMode: 'add' | 'remove' = $state('add');
  let bulkBusy = $state(false);
  let bulkError: string | undefined = $state(undefined);
  let bulkTargetId: number | '' = $state('');

  // ─── new-collection inline form ───
  let newCollectionName = $state('');
  let creatingCollection = $state(false);

  const PAGE = 50;

  async function load() {
    loading = true;
    try {
      const r = await api.sources({
        q,
        filter_type: type,
        sort_by: sort,
        limit: PAGE,
        offset: 0,
        ...(collectionId !== '' ? { collection_id: collectionId as number } : {}),
      });
      sources = r.sources ?? [];
      total = r.total ?? 0;
      // If we're already viewing the unfiltered vault, the same response gives
      // us the vault total for free. Otherwise refresh it separately so the
      // ALL tab's count is right even while the user is scoped to a collection.
      if (collectionId === '' && !q && !type) {
        vaultTotal = total;
      } else {
        const u = await api.sources({ limit: 1, offset: 0 });
        vaultTotal = u.total ?? vaultTotal;
      }
    } finally {
      loading = false;
    }
  }

  async function loadMore() {
    const r = await api.sources({
      q,
      filter_type: type,
      sort_by: sort,
      limit: PAGE,
      offset: sources.length,
      ...(collectionId !== '' ? { collection_id: collectionId as number } : {}),
    });
    sources = [...sources, ...(r.sources ?? [])];
    total = r.total ?? total;
  }

  async function loadTypes() {
    const r = await api.types();
    types = r.types ?? [];
  }

  async function loadCollections() {
    const r = await api.collectionsList();
    if (!r.error) collections = r.collections ?? [];
  }

  function onQ(e: Event) {
    q = (e.target as HTMLInputElement).value;
    if (debounce) clearTimeout(debounce);
    debounce = window.setTimeout(load, 250);
  }

  onMount(() => {
    load();
    loadTypes();
    loadCollections();
  });

  let canMore = $derived(sources.length < total);
  let activeCollection = $derived(
    collectionId === '' ? null : collections.find((c) => c.id === collectionId) ?? null,
  );
  let summary = $derived(
    sources.length === 0
      ? ''
      : `SHOWING ${sources.length} OF ${total} SOURCE PATH${total === 1 ? '' : 'S'}` +
          (activeCollection ? ` · IN "${activeCollection.name.toUpperCase()}"` : '') +
          (q ? ` · MATCHING "${q.toUpperCase()}"` : '')
  );

  function fmtNum(n: number | undefined) {
    return (n ?? 0).toLocaleString();
  }

  function toggleRow(path: string) {
    const next = new Set(selected);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    selected = next;
  }
  function toggleAll() {
    if (selected.size === sources.length) {
      selected = new Set();
    } else {
      selected = new Set(sources.map((s) => s.path));
    }
  }
  function clearSelection() {
    selected = new Set();
  }

  async function createCollection() {
    if (creatingCollection || !newCollectionName.trim()) return;
    creatingCollection = true;
    try {
      const r = await api.collectionsCreate(newCollectionName.trim());
      if (r.error) {
        bulkError = r.error;
      } else {
        newCollectionName = '';
        await loadCollections();
      }
    } finally {
      creatingCollection = false;
    }
  }

  async function deleteCollection(id: number, name: string) {
    if (!confirm(`Delete collection "${name}"? Documents are not deleted; only the collection is removed.`)) return;
    const r = await api.collectionsDelete(id);
    if (r.error) {
      bulkError = r.error;
      return;
    }
    if (collectionId === id) collectionId = '';
    await loadCollections();
    await load();
  }

  function openMoveDialog(mode: 'add' | 'remove') {
    if (selected.size === 0) return;
    bulkError = undefined;
    bulkTargetId = activeCollection ? activeCollection.id : '';
    collectionDialogMode = mode;
    collectionDialogOpen = true;
  }

  async function applyMove() {
    if (bulkBusy || bulkTargetId === '' || selected.size === 0) return;
    bulkBusy = true;
    bulkError = undefined;
    try {
      const ids = Array.from(selected);
      const id = bulkTargetId as number;
      const r =
        collectionDialogMode === 'add'
          ? await api.collectionAssign(id, ids)
          : await api.collectionUnassign(id, ids);
      if (r.error) {
        bulkError = r.error;
      } else {
        collectionDialogOpen = false;
        clearSelection();
        await loadCollections();
        await load();
      }
    } finally {
      bulkBusy = false;
    }
  }

  let allSelected = $derived(sources.length > 0 && selected.size === sources.length);
</script>

<ViewHead num="04" title={`DOCUMENTS <span class="slash">/</span> SOURCE INDEX`}
  desc="Every ingested source path with chunk and record counts. Group sources into collections, filter by collection, drill in." />

<!-- Collection picker + management strip -->
<div class="collections-bar">
  <div class="collections-tabs" role="tablist">
    <button class="ctab" type="button"
      class:active={collectionId === ''}
      onclick={() => { collectionId = ''; load(); }}>
      ALL <span class="count">{vaultTotal}</span>
    </button>
    {#each collections as c (c.id)}
      <button class="ctab" type="button"
        class:active={collectionId === c.id}
        onclick={() => { collectionId = c.id; load(); }}>
        {c.name}
        <span class="count">{c.document_count}</span>
        <span class="x" role="button" aria-label="Delete collection"
          onclick={(e) => { e.stopPropagation(); deleteCollection(c.id, c.name); }}
          onkeydown={(e) => e.key === 'Enter' && deleteCollection(c.id, c.name)}
          tabindex="0">×</span>
      </button>
    {/each}
  </div>
  <div class="new-collection">
    <input type="text" class="input new-input" placeholder="new collection name…"
      bind:value={newCollectionName}
      onkeydown={(e) => e.key === 'Enter' && createCollection()} />
    <button class="btn btn-secondary btn-sm" type="button"
      onclick={createCollection}
      disabled={creatingCollection || !newCollectionName.trim()}>+ NEW</button>
  </div>
</div>

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

<div class="summary">
  <span>{summary}</span>
  {#if selected.size > 0}
    <span class="bulk-actions">
      {selected.size} selected ·
      <button class="link" type="button" onclick={() => openMoveDialog('add')}>move to collection</button>
      {#if activeCollection}
        <span class="sep">·</span>
        <button class="link" type="button" onclick={() => openMoveDialog('remove')}>remove from "{activeCollection.name}"</button>
      {/if}
      <span class="sep">·</span>
      <button class="link dim" type="button" onclick={clearSelection}>clear</button>
    </span>
  {/if}
</div>

{#if loading && sources.length === 0}
  <div class="loading"><span class="glyph">::</span>loading documents...</div>
{:else if sources.length === 0}
  <div class="empty">
    <span class="glyph">::</span>
    {activeCollection
      ? `no documents in "${activeCollection.name}". Switch to ALL above and select rows to move them in.`
      : 'no documents match. use the ingest tab to add files.'}
  </div>
{:else}
  <table class="doc-table">
    <thead><tr>
      <th class="check">
        <input type="checkbox" checked={allSelected} onchange={toggleAll}
          aria-label="Select all visible rows" />
      </th>
      <th>Type</th><th>Source</th><th>Records</th><th>Chunks</th><th>Ingested</th>
    </tr></thead>
    <tbody>
      {#each sources as s (s.path)}
        <tr class:selected={selected.has(s.path)}>
          <td class="check">
            <input type="checkbox"
              checked={selected.has(s.path)}
              onchange={() => toggleRow(s.path)}
              aria-label={`Select ${s.path}`} />
          </td>
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

{#if collectionDialogOpen}
  <div class="modal-backdrop" role="dialog"
    onclick={(e) => e.target === e.currentTarget && (collectionDialogOpen = false)}
    onkeydown={(e) => e.key === 'Escape' && (collectionDialogOpen = false)}
    tabindex="-1">
    <div class="modal">
      <div class="modal-head">
        <span class="modal-title">
          {collectionDialogMode === 'add'
            ? `MOVE ${selected.size} SOURCE PATH${selected.size === 1 ? '' : 'S'}`
            : `REMOVE ${selected.size} SOURCE PATH${selected.size === 1 ? '' : 'S'}`}
        </span>
        <button class="modal-close" type="button"
          onclick={() => (collectionDialogOpen = false)}>×</button>
      </div>
      <div class="modal-body">
        <p class="desc">
          {collectionDialogMode === 'add'
            ? 'Pick the destination collection. The selected source paths will be added; existing memberships are unaffected (a path can be in multiple collections).'
            : 'Pick the collection to remove these source paths from. The documents themselves are not deleted.'}
        </p>
        <select class="select" bind:value={bulkTargetId}>
          <option value="">choose collection…</option>
          {#each collections as c (c.id)}
            <option value={c.id}>{c.name} · {c.document_count} docs</option>
          {/each}
        </select>
        {#if bulkError}<p class="err">{bulkError}</p>{/if}
      </div>
      <div class="modal-foot">
        <button class="btn btn-secondary btn-sm" type="button"
          onclick={() => (collectionDialogOpen = false)}>CANCEL</button>
        <button class="btn btn-primary btn-sm" type="button"
          onclick={applyMove}
          disabled={bulkBusy || bulkTargetId === ''}>
          {bulkBusy ? 'WORKING…' : (collectionDialogMode === 'add' ? 'ADD' : 'REMOVE')}
        </button>
      </div>
    </div>
  </div>
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
    display: flex; justify-content: space-between; align-items: center;
    flex-wrap: wrap; gap: 12px;
  }
  .bulk-actions { color: var(--amber); font-weight: 700; }
  .bulk-actions .sep { color: var(--text-dim); margin: 0 4px; font-weight: 400; }
  .link {
    background: transparent; border: 0; padding: 0;
    font: inherit; color: var(--teal);
    cursor: pointer; text-decoration: underline; letter-spacing: inherit;
  }
  .link:hover { color: var(--teal-deep); }
  .link.dim { color: var(--text-dim); }

  /* ───── collection bar ───── */
  .collections-bar {
    display: flex; gap: 12px; flex-wrap: wrap;
    align-items: center; margin-bottom: 14px;
    padding-bottom: 10px; border-bottom: 1px dashed var(--border);
  }
  .collections-tabs {
    display: flex; gap: 6px; flex-wrap: wrap; flex: 1; min-width: 0;
  }
  .ctab {
    background: var(--surface);
    border: 1px solid var(--border);
    color: var(--text-dim);
    cursor: pointer;
    padding: 6px 12px;
    font-family: inherit;
    font-size: 11px;
    letter-spacing: 1.5px;
    text-transform: uppercase;
    font-weight: 700;
    display: inline-flex; align-items: center; gap: 6px;
    transition: 120ms ease;
  }
  .ctab:hover { color: var(--text-bright); border-color: var(--border-strong); }
  .ctab.active {
    color: var(--amber);
    border-color: var(--amber);
    background: var(--amber-soft);
  }
  .ctab .count {
    font-size: 9px; color: var(--text-dim); font-weight: 500;
    letter-spacing: 1px;
  }
  .ctab.active .count { color: var(--amber); }
  .ctab .x {
    color: var(--text-dim);
    margin-left: 4px;
    padding: 0 4px;
    font-weight: 400;
    font-size: 14px;
    line-height: 1;
  }
  .ctab .x:hover { color: var(--danger); }
  .new-collection {
    display: flex; gap: 8px; align-items: center;
  }
  .new-input {
    width: 200px;
    font-size: 11px;
    padding: 6px 10px;
  }

  .doc-table { width: 100%; border-collapse: collapse; }
  .doc-table th {
    text-align: left; padding: 10px 14px;
    font-size: 9px; color: var(--text-dim); text-transform: uppercase;
    letter-spacing: 2px; font-weight: 700;
    border-bottom: 1px solid var(--border); background: var(--surface);
  }
  .doc-table th.check, .doc-table td.check {
    width: 28px; padding-right: 6px; padding-left: 14px;
  }
  .doc-table td {
    padding: 10px 14px; border-bottom: 1px solid var(--border);
    font-size: 12px; vertical-align: middle;
  }
  .doc-table tr:hover td { background: var(--surface); }
  .doc-table tr.selected td { background: var(--amber-soft); }
  .doc-table .path { color: var(--text); word-break: break-all; font-size: 11px; }
  .doc-table .num { color: var(--amber); font-weight: 700; letter-spacing: 1px; }
  .doc-table input[type='checkbox'] {
    accent-color: var(--amber);
    width: 14px; height: 14px;
    cursor: pointer;
  }

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

  /* ───── bulk-move modal (lightweight, uses the existing modal CSS classes) ───── */
  .modal-backdrop {
    position: fixed; inset: 0; background: rgba(0, 0, 0, 0.65);
    z-index: 100; display: flex; align-items: center; justify-content: center;
    padding: 24px;
  }
  .modal {
    background: var(--surface); border: 1px solid var(--border);
    width: min(540px, 100%); max-height: 80vh;
    display: flex; flex-direction: column;
    box-shadow: var(--shadow-frame);
  }
  .modal-head {
    display: flex; justify-content: space-between; align-items: center;
    padding: 14px 18px; border-bottom: 1px solid var(--border);
  }
  .modal-title {
    font-size: 11px; font-weight: 700; letter-spacing: 2.5px;
    text-transform: uppercase; color: var(--amber);
  }
  .modal-close {
    background: transparent; border: 0; color: var(--text-dim);
    font-size: 22px; cursor: pointer; line-height: 1;
  }
  .modal-close:hover { color: var(--text-bright); }
  .modal-body {
    padding: 14px 18px;
    display: flex; flex-direction: column; gap: 10px;
  }
  .modal-foot {
    display: flex; justify-content: flex-end; gap: 10px;
    padding: 12px 18px; border-top: 1px solid var(--border);
  }
  .desc {
    font-size: 12px; color: var(--text-dim); line-height: 1.6; margin: 0;
  }
  .err { color: var(--danger); font-size: 11px; margin: 0; }
</style>
