<script lang="ts">
  import ViewHead from '../components/ViewHead.svelte';
  import StatusLine from '../components/StatusLine.svelte';
  import { api } from '../lib/api';
  import type { FileTypeStat } from '../lib/types';

  let prefix = $state('');
  let prefixStatus = $state(''); let prefixTone = $state<'dim' | 'teal' | 'danger'>('dim');

  let type = $state('');
  let typeStatus = $state(''); let typeTone = $state<'dim' | 'teal' | 'danger'>('dim');

  let clearStatus = $state(''); let clearTone = $state<'dim' | 'teal' | 'danger'>('dim');

  let types = $state<FileTypeStat[]>([]);

  $effect(() => {
    api.types().then((r) => { types = r.types ?? []; });
  });

  async function deleteOp(
    body: { prefix?: string; file_type?: string },
    setS: (s: string) => void,
    setT: (t: 'dim' | 'teal' | 'danger') => void,
    dryRun: boolean
  ) {
    setT('dim'); setS('WORKING...');
    try {
      const r = await api.deleteSources({ ...body, dry_run: dryRun });
      if (r.error) { setT('danger'); setS('FAILED · ' + r.error); return null; }
      setT('teal');
      setS((dryRun ? 'WOULD DELETE' : 'DELETED') + ' ' + r.deleted_documents + ' DOCUMENTS · ' + r.deleted_chunks + ' CHUNKS');
      return r;
    } catch (e) {
      setT('danger'); setS('NETWORK ERROR · ' + (e as Error).message);
      return null;
    }
  }

  async function previewPrefix() {
    if (!prefix.trim()) return;
    await deleteOp({ prefix }, (s) => (prefixStatus = s), (t) => (prefixTone = t), true);
  }
  async function commitPrefix() {
    if (!prefix.trim()) return;
    const r = await deleteOp({ prefix }, (s) => (prefixStatus = s), (t) => (prefixTone = t), true);
    if (r && r.deleted_documents > 0) {
      if (confirm(`Permanently delete ${r.deleted_documents} documents matching "${prefix}"?`)) {
        await deleteOp({ prefix }, (s) => (prefixStatus = s), (t) => (prefixTone = t), false);
      }
    }
  }

  async function previewType() {
    if (!type) return;
    await deleteOp({ file_type: type }, (s) => (typeStatus = s), (t) => (typeTone = t), true);
  }
  async function commitType() {
    if (!type) return;
    const r = await deleteOp({ file_type: type }, (s) => (typeStatus = s), (t) => (typeTone = t), true);
    if (r && r.deleted_documents > 0) {
      if (confirm(`Permanently delete all ${r.deleted_documents} .${type} documents?`)) {
        await deleteOp({ file_type: type }, (s) => (typeStatus = s), (t) => (typeTone = t), false);
      }
    }
  }

  async function clearAll() {
    clearTone = 'dim'; clearStatus = 'COUNTING...';
    try {
      const dr = await api.clear({ dry_run: true });
      if (dr.error) { clearTone = 'danger'; clearStatus = 'FAILED · ' + dr.error; return; }
      if (dr.deleted_documents === 0) { clearTone = 'teal'; clearStatus = 'VAULT IS ALREADY EMPTY'; return; }
      const typed = prompt(`Type WIPE to permanently delete ${dr.deleted_documents} documents and ${dr.deleted_chunks} chunks:`);
      if (typed !== 'WIPE') { clearStatus = 'CANCELLED'; return; }
      const r = await api.clear({ confirm: true });
      if (r.error) { clearTone = 'danger'; clearStatus = 'FAILED · ' + r.error; }
      else {
        clearTone = 'teal';
        clearStatus = `VAULT CLEARED · ${r.deleted_documents} DOCUMENTS · ${r.deleted_chunks} CHUNKS`;
      }
    } catch (e) {
      clearTone = 'danger'; clearStatus = 'NETWORK ERROR · ' + (e as Error).message;
    }
  }
</script>

<ViewHead num="06" title={`MANAGE <span class="slash">/</span> PRUNE THE VAULT`}
  desc="Delete operations are immediate and not undoable. Always preview first." />

<div class="section-label">DELETE BY PATH PREFIX</div>
<div class="row">
  <input type="text" class="input grow" placeholder="HeatSync Labs Slack export" bind:value={prefix} />
  <button class="btn btn-secondary btn-sm" onclick={previewPrefix}>PREVIEW</button>
  <button class="btn btn-danger btn-sm" onclick={commitPrefix}>DELETE</button>
</div>
<StatusLine text={prefixStatus} tone={prefixTone} />

<div class="section-label">DELETE BY FILE TYPE</div>
<div class="row">
  <select class="select grow" bind:value={type}>
    <option value="">choose type…</option>
    {#each types as t (t.file_type)}
      <option value={t.file_type}>{t.file_type} ({t.source_count})</option>
    {/each}
  </select>
  <button class="btn btn-secondary btn-sm" onclick={previewType}>PREVIEW</button>
  <button class="btn btn-danger btn-sm" onclick={commitType}>DELETE</button>
</div>
<StatusLine text={typeStatus} tone={typeTone} />

<div class="section-label danger">DANGER ZONE</div>
<p class="instructions">Wipe every document and chunk in the active vault. Schema and FTS index remain.</p>
<button class="btn btn-danger" onclick={clearAll}>WIPE ENTIRE VAULT…</button>
<div style="margin-top:10px;"><StatusLine text={clearStatus} tone={clearTone} /></div>

<style>
  .row { display: flex; gap: 10px; flex-wrap: wrap; align-items: center; margin-bottom: 10px; }
  .grow { flex: 1; min-width: 220px; }
  .section-label.danger { color: var(--danger); }
  .section-label.danger::before { color: var(--danger); }
  .instructions {
    margin-bottom: 16px; color: var(--text-dim);
    font-size: 12px; line-height: 1.7;
  }
</style>
