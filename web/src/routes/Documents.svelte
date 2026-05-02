<script lang="ts">
  import { onMount } from 'svelte';
  import ViewHead from '../components/ViewHead.svelte';
  import { api, type CollectionSummary } from '../lib/api';
  import type { DocumentRow, FileTypeStat, SourceRow } from '../lib/types';

  // ─── view mode ───
  // BY RECORD shows every `documents` row — what the user thinks of as
  // "ingested data" (each Slack message, each PDF, each conversation).
  // BY SOURCE groups by source_path so an archive collapses into one row
  // per file. The default is RECORD because that's the question users
  // actually want answered ("what's in my vault, and how do I move
  // pieces of it into a collection").
  type ViewMode = 'record' | 'source';
  let viewMode = $state<ViewMode>('record');

  let q = $state('');
  let type = $state('');
  let sort = $state('name');
  let collectionId: number | '' = $state('');
  let sources = $state<SourceRow[]>([]);
  let documents = $state<DocumentRow[]>([]);
  let total = $state(0);
  // Whole-vault count, independent of collection / search filters. Each
  // mode tracks its own — sources counts source_paths, records counts
  // documents — so the ALL-tab number is honest about which axis you're
  // looking at.
  let vaultTotalSources = $state(0);
  let vaultTotalRecords = $state(0);
  let loading = $state(true);
  let types = $state<FileTypeStat[]>([]);
  let collections = $state<CollectionSummary[]>([]);
  let debounce: number | undefined;

  // ─── multi-select ───
  // Source mode keys selection by source_path; record mode by document.id.
  // They're disjoint sets so switching modes doesn't lose context within
  // a mode but bulk-move always operates on the active mode's selection.
  let selectedSources = $state<Set<string>>(new Set());
  let selectedDocs = $state<Set<string>>(new Set());
  let collectionDialogOpen = $state(false);
  let collectionDialogMode: 'add' | 'remove' = $state('add');
  let bulkBusy = $state(false);
  let bulkError: string | undefined = $state(undefined);
  let bulkTargetId: number | '' = $state('');

  // ─── new-collection inline form ───
  let newCollectionName = $state('');
  let creatingCollection = $state(false);

  const PAGE = 50;

  function commonParams(offset: number) {
    return {
      q,
      filter_type: type,
      sort_by: sort,
      limit: PAGE,
      offset,
      ...(collectionId !== '' ? { collection_id: collectionId as number } : {}),
    };
  }

  async function load() {
    loading = true;
    try {
      if (viewMode === 'source') {
        const r = await api.sources(commonParams(0));
        sources = r.sources ?? [];
        total = r.total ?? 0;
      } else {
        const r = await api.documents(commonParams(0));
        documents = r.documents ?? [];
        total = r.total ?? 0;
      }
      // Refresh the unfiltered vault totals so the ALL tab is honest about
      // what's in the vault regardless of what the user is currently
      // scoped to. One extra cheap call per mode when filters are active.
      const filterActive = collectionId !== '' || q || type;
      if (!filterActive) {
        if (viewMode === 'source') vaultTotalSources = total;
        else vaultTotalRecords = total;
      } else {
        // Only fetch the mode that's currently visible; the other refreshes
        // when the user toggles into it.
        if (viewMode === 'source') {
          const u = await api.sources({ limit: 1, offset: 0 });
          vaultTotalSources = u.total ?? vaultTotalSources;
        } else {
          const u = await api.documents({ limit: 1, offset: 0 });
          vaultTotalRecords = u.total ?? vaultTotalRecords;
        }
      }
    } finally {
      loading = false;
    }
  }

  async function loadMore() {
    if (viewMode === 'source') {
      const r = await api.sources(commonParams(sources.length));
      sources = [...sources, ...(r.sources ?? [])];
      total = r.total ?? total;
    } else {
      const r = await api.documents(commonParams(documents.length));
      documents = [...documents, ...(r.documents ?? [])];
      total = r.total ?? total;
    }
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

  function setMode(m: ViewMode) {
    if (m === viewMode) return;
    viewMode = m;
    // Selection axes are different — clear so the user doesn't carry a
    // stale path-set into a record-set view (and vice versa).
    selectedSources = new Set();
    selectedDocs = new Set();
    load();
  }

  onMount(() => {
    load();
    loadTypes();
    loadCollections();
  });

  let rowCount = $derived(viewMode === 'source' ? sources.length : documents.length);
  let canMore = $derived(rowCount < total);
  let activeCollection = $derived(
    collectionId === '' ? null : collections.find((c) => c.id === collectionId) ?? null,
  );
  let unit = $derived(viewMode === 'source' ? 'SOURCE PATH' : 'RECORD');
  let summary = $derived(
    rowCount === 0
      ? ''
      : `SHOWING ${rowCount} OF ${total} ${unit}${total === 1 ? '' : 'S'}` +
          (activeCollection ? ` · IN "${activeCollection.name.toUpperCase()}"` : '') +
          (q ? ` · MATCHING "${q.toUpperCase()}"` : '')
  );
  let allTabCount = $derived(
    viewMode === 'source' ? vaultTotalSources : vaultTotalRecords,
  );
  let selectedCount = $derived(
    viewMode === 'source' ? selectedSources.size : selectedDocs.size,
  );
  let allSelected = $derived(rowCount > 0 && selectedCount === rowCount);

  function fmtNum(n: number | undefined) {
    return (n ?? 0).toLocaleString();
  }

  function toggleSourceRow(path: string) {
    const next = new Set(selectedSources);
    if (next.has(path)) next.delete(path);
    else next.add(path);
    selectedSources = next;
  }
  function toggleDocRow(id: string) {
    const next = new Set(selectedDocs);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    selectedDocs = next;
  }
  function toggleAll() {
    if (viewMode === 'source') {
      selectedSources =
        selectedSources.size === sources.length
          ? new Set()
          : new Set(sources.map((s) => s.path));
    } else {
      selectedDocs =
        selectedDocs.size === documents.length
          ? new Set()
          : new Set(documents.map((d) => d.id));
    }
  }
  function clearSelection() {
    selectedSources = new Set();
    selectedDocs = new Set();
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
    if (selectedCount === 0) return;
    bulkError = undefined;
    bulkTargetId = activeCollection ? activeCollection.id : '';
    collectionDialogMode = mode;
    collectionDialogOpen = true;
  }

  async function applyMove() {
    if (bulkBusy || bulkTargetId === '' || selectedCount === 0) return;
    bulkBusy = true;
    bulkError = undefined;
    try {
      const id = bulkTargetId as number;
      const r =
        viewMode === 'source'
          ? collectionDialogMode === 'add'
            ? await api.collectionAssign(id, Array.from(selectedSources))
            : await api.collectionUnassign(id, Array.from(selectedSources))
          : collectionDialogMode === 'add'
            ? await api.collectionAssignDocs(id, Array.from(selectedDocs))
            : await api.collectionUnassignDocs(id, Array.from(selectedDocs));
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

  function collectionNamesFor(ids: number[]): string {
    if (!ids.length) return '';
    return ids
      .map((id) => collections.find((c) => c.id === id)?.name ?? `#${id}`)
      .join(', ');
  }
</script>

<ViewHead num="04" title={`DOCUMENTS <span class="slash">/</span> VAULT INDEX`}
  desc="Every ingested record in the vault. Toggle BY RECORD to see individual messages / files / conversations, or BY SOURCE to collapse archives by their source path. Multi-select rows and move them between collections." />

<!-- Collection picker + management strip -->
<div class="collections-bar">
  <div class="collections-tabs" role="tablist">
    <button class="ctab" type="button"
      class:active={collectionId === ''}
      onclick={() => { collectionId = ''; load(); }}>
      ALL <span class="count">{allTabCount}</span>
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
  <input type="text" class="input grow"
    placeholder={viewMode === 'record'
      ? 'filter by title or path substring…'
      : 'filter by path substring…'}
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
    {#if viewMode === 'source'}
      <option value="records">sort: most records</option>
    {/if}
  </select>
  <div class="mode-toggle" role="radiogroup" aria-label="View mode">
    <button class="mtog" type="button" role="radio"
      aria-checked={viewMode === 'record'}
      class:active={viewMode === 'record'}
      onclick={() => setMode('record')}
      title="One row per ingested record (each Slack message, each PDF, each conversation)">BY RECORD</button>
    <button class="mtog" type="button" role="radio"
      aria-checked={viewMode === 'source'}
      class:active={viewMode === 'source'}
      onclick={() => setMode('source')}
      title="Group records by source_path (archives collapse to one row)">BY SOURCE</button>
  </div>
</div>

<div class="summary">
  <span>{summary}</span>
  {#if selectedCount > 0}
    <span class="bulk-actions">
      {selectedCount} selected ·
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

{#if loading && rowCount === 0}
  <div class="loading"><span class="glyph">::</span>loading documents...</div>
{:else if rowCount === 0}
  <div class="empty">
    <span class="glyph">::</span>
    {activeCollection
      ? `no ${viewMode === 'record' ? 'records' : 'sources'} in "${activeCollection.name}". Switch to ALL above and select rows to move them in.`
      : 'nothing in the vault matches. Use the Ingest tab to add files, or clear the filters above.'}
  </div>
{:else if viewMode === 'source'}
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
        <tr class:selected={selectedSources.has(s.path)}>
          <td class="check">
            <input type="checkbox"
              checked={selectedSources.has(s.path)}
              onchange={() => toggleSourceRow(s.path)}
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
{:else}
  <table class="doc-table">
    <thead><tr>
      <th class="check">
        <input type="checkbox" checked={allSelected} onchange={toggleAll}
          aria-label="Select all visible records" />
      </th>
      <th>Type</th><th>Title / Source</th><th>Chunks</th><th>Collections</th><th>Ingested</th>
    </tr></thead>
    <tbody>
      {#each documents as d (d.id)}
        <tr class:selected={selectedDocs.has(d.id)}>
          <td class="check">
            <input type="checkbox"
              checked={selectedDocs.has(d.id)}
              onchange={() => toggleDocRow(d.id)}
              aria-label={`Select ${d.title ?? d.id}`} />
          </td>
          <td><span class="badge {d.file_type}">{d.file_type}</span></td>
          <td class="path">
            {#if d.title}<span class="title">{d.title}</span>{/if}
            <span class="sub">{d.source_path}</span>
          </td>
          <td><span class="num">{fmtNum(d.chunk_count)}</span></td>
          <td class="cols">
            {#if d.collection_ids.length === 0}
              <span class="dim">—</span>
            {:else}
              {collectionNamesFor(d.collection_ids)}
            {/if}
          </td>
          <td>{d.ingested_at ?? '—'}</td>
        </tr>
      {/each}
    </tbody>
  </table>
  {#if canMore}
    <div class="more"><span class="info">{documents.length} / {total}</span>
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
          {#if viewMode === 'record'}
            {collectionDialogMode === 'add'
              ? `MOVE ${selectedCount} RECORD${selectedCount === 1 ? '' : 'S'}`
              : `REMOVE ${selectedCount} RECORD${selectedCount === 1 ? '' : 'S'}`}
          {:else}
            {collectionDialogMode === 'add'
              ? `MOVE ${selectedCount} SOURCE PATH${selectedCount === 1 ? '' : 'S'}`
              : `REMOVE ${selectedCount} SOURCE PATH${selectedCount === 1 ? '' : 'S'}`}
          {/if}
        </span>
        <button class="modal-close" type="button"
          onclick={() => (collectionDialogOpen = false)}>×</button>
      </div>
      <div class="modal-body">
        <p class="desc">
          {#if viewMode === 'record'}
            {collectionDialogMode === 'add'
              ? 'Pick the destination collection. Each selected record is added individually; existing memberships are unaffected (a record can be in multiple collections).'
              : 'Pick the collection to remove these records from. The records themselves are not deleted.'}
          {:else}
            {collectionDialogMode === 'add'
              ? 'Pick the destination collection. The selected source paths will be added — every record at each path joins the collection.'
              : 'Pick the collection to remove these source paths from. The documents themselves are not deleted.'}
          {/if}
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
  .doc-table .path .title {
    display: block; color: var(--text-bright); font-weight: 700;
    font-size: 12px; letter-spacing: 0.3px; margin-bottom: 2px;
  }
  .doc-table .path .sub {
    display: block; font-size: 10px; color: var(--text-dim);
    word-break: break-all;
  }
  .doc-table .num { color: var(--amber); font-weight: 700; letter-spacing: 1px; }
  .doc-table .cols {
    color: var(--teal); font-size: 11px; letter-spacing: 0.3px;
  }
  .doc-table .cols .dim { color: var(--text-dim); }

  /* mode toggle inside the filter row */
  .mode-toggle {
    display: inline-flex; gap: 0;
    border: 1px solid var(--border);
  }
  .mtog {
    background: var(--surface);
    border: 0;
    color: var(--text-dim);
    cursor: pointer;
    padding: 8px 14px;
    font-family: inherit;
    font-size: 10px;
    letter-spacing: 1.5px;
    text-transform: uppercase;
    font-weight: 700;
    transition: 120ms ease;
  }
  .mtog + .mtog { border-left: 1px solid var(--border); }
  .mtog:hover { color: var(--text-bright); }
  .mtog.active {
    color: var(--amber);
    background: var(--amber-soft);
  }
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
