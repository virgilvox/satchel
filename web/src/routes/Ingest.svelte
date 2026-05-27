<script lang="ts">
  import { onMount } from 'svelte';
  import ViewHead from '../components/ViewHead.svelte';
  import Modal from '../components/Modal.svelte';
  import StatusLine from '../components/StatusLine.svelte';
  import { api } from '../lib/api';
  import { collection } from '../lib/stores.svelte';
  import type { BrowseEntry, IngestJob } from '../lib/types';

  let path = $state('');
  let status = $state('');
  let statusTone = $state<'dim' | 'teal' | 'danger'>('dim');

  // Collection destination for the next ingest job. Three states:
  //   'global'  -> follow the TopBar scope (which may itself be ALL)
  //   ''        -> explicitly ingest with no collection assignment
  //   <number>  -> override; pin this job to a specific collection
  // 'global' is the default so a user who picked Work from the chip
  // does not have to re-pick it here every time they queue a job.
  let destMode = $state<'global' | '' | number>('global');
  let newCollectionDraft = $state('');
  let createBusy = $state(false);

  function resolveDestination(): { collection_id?: number; collection_name?: string } | null {
    const draft = newCollectionDraft.trim();
    if (draft) {
      // Server side resolves-or-creates, so we pass the name through
      // verbatim. Clearing the draft happens after the job is queued.
      return { collection_name: draft };
    }
    if (destMode === 'global') {
      return collection.activeId != null ? { collection_id: collection.activeId } : {};
    }
    if (destMode === '') return {};
    return { collection_id: destMode };
  }

  let destLabel = $derived.by(() => {
    const draft = newCollectionDraft.trim();
    if (draft) return `new: ${draft}`;
    if (destMode === 'global') {
      return collection.activeName
        ? `follows scope: ${collection.activeName}`
        : 'no collection (whole vault)';
    }
    if (destMode === '') return 'no collection (whole vault)';
    const c = collection.list.find((c) => c.id === destMode);
    return c ? c.name : `#${destMode}`;
  });

  // Jobs
  let jobs = $state<IngestJob[]>([]);
  let timer: number | undefined;
  // Tracks whether this view is still alive. Async work that resolves after
  // unmount must not schedule new intervals or write reactive state.
  let alive = true;

  async function refreshJobs() {
    if (!alive) return;
    try {
      const r = await api.jobs();
      if (!alive) return;
      jobs = r.jobs ?? [];
      const active = jobs.some((j) => j.status === 'running' || j.status === 'pending');
      if (active && !timer) timer = window.setInterval(refreshJobs, 1500);
      if (!active && timer) {
        clearInterval(timer);
        timer = undefined;
      }
    } catch {
      // Silent on transient network errors.
    }
  }

  async function go() {
    const p = path.trim();
    if (!p) {
      statusTone = 'danger'; status = 'ENTER A PATH FIRST'; return;
    }
    statusTone = 'dim'; status = 'QUEUING JOB FOR ' + p + '...';
    try {
      const dest = resolveDestination();
      const r = await api.ingest(p, dest ?? {});
      if (r.error) { statusTone = 'danger'; status = 'FAILED · ' + r.error; }
      else {
        const into = destLabel;
        statusTone = 'teal';
        status = `JOB QUEUED · INTO ${into.toUpperCase()} · TRACK PROGRESS BELOW`;
        path = '';
        newCollectionDraft = '';
        refreshJobs();
      }
    } catch (e) {
      statusTone = 'danger'; status = 'NETWORK ERROR · ' + (e as Error).message;
    }
  }

  // Browse modal
  let bOpen = $state(false);
  let bPath = $state('');
  let bEntries = $state<BrowseEntry[]>([]);
  let bError = $state<string | undefined>();

  async function openBrowse() { bOpen = true; await loadBrowse(''); }
  async function loadBrowse(p: string) {
    const r = await api.browse(p);
    if (r.error) { bError = r.error; bEntries = []; return; }
    bError = undefined;
    bPath = r.path;
    bEntries = r.entries ?? [];
  }
  async function browseUp() {
    const r = await api.browse(bPath);
    if (r.parent) loadBrowse(r.parent);
  }
  function pickEntry(e: BrowseEntry) {
    if (e.kind === 'dir') loadBrowse(e.path);
    else { path = e.path; bOpen = false; }
  }

  // Poll jobs while this view is mounted; clear the interval on unmount so
  // we don't leak a 1.5s tick after the user navigates to another tab.
  onMount(() => {
    refreshJobs();
    return () => {
      alive = false;
      if (timer) {
        clearInterval(timer);
        timer = undefined;
      }
    };
  });

  function fmtNum(n: number | undefined) { return (n ?? 0).toLocaleString(); }
  /** Records-per-second derived from records_added and elapsed wall-clock.
   *  Returns null while the job is too young (<1s) to have a meaningful
   *  rate — no division-by-zero, no jittery 100k/s flashes at the start. */
  function rate(j: IngestJob): string | null {
    if (!j.started_at) return null;
    const start = new Date(j.started_at).getTime();
    const end = j.finished_at ? new Date(j.finished_at).getTime() : Date.now();
    const sec = (end - start) / 1000;
    if (sec < 1) return null;
    const rps = (j.records_added ?? 0) / sec;
    if (rps < 1) return rps.toFixed(2) + '/s';
    if (rps < 100) return rps.toFixed(1) + '/s';
    return Math.round(rps).toLocaleString() + '/s';
  }
  /** 0..1 fraction for the determinate progress bar, or null if we can't
   *  compute one (archive ingest, single-file, or pre-walk hasn't reported
   *  the total yet). */
  function progressFrac(j: IngestJob): number | null {
    if (!j.files_total || j.files_total === 0) return null;
    return Math.min(1, (j.files_seen ?? 0) / j.files_total);
  }
  function fmtTime(iso?: string) {
    if (!iso) return '';
    const d = new Date(iso);
    const ms = Date.now() - d.getTime();
    if (ms < 60_000) return Math.round(ms / 1000) + 's ago';
    if (ms < 3600_000) return Math.round(ms / 60000) + 'm ago';
    return d.toLocaleTimeString();
  }
  function fmtDuration(start?: string, end?: string) {
    if (!start) return '';
    const s = new Date(start).getTime();
    const e = end ? new Date(end).getTime() : Date.now();
    const sec = Math.max(0, Math.round((e - s) / 1000));
    if (sec < 60) return sec + 's';
    const m = Math.floor(sec / 60);
    return m + 'm ' + (sec - m * 60) + 's';
  }
</script>

<ViewHead num="05" title={`INGEST <span class="slash">/</span> ABSORB INTO VAULT`}
  desc="Drop a folder or a single file. Format-aware parsers handle archives; standard documents flow through their own extractor. Each path runs as its own job." />

<p class="instructions">
  Paste a path or use <em>Browse…</em> to navigate.
  <strong>Single files:</strong>
  <code>md</code> ·
  <code>txt</code> ·
  <code>pdf</code> ·
  <code>docx</code> ·
  <code>html</code> ·
  <code>csv</code> ·
  <code>tsv</code> ·
  <code>json</code>.
  <strong>Format-aware archives</strong> (auto-detected, parsed structurally —
  no raw JSON blobs in your vault): <code>Slack</code> exports ·
  <code>ChatGPT</code> exports · <code>Claude.ai</code> exports ·
  <code>Discord</code> (DiscordChatExporter) ·
  <code>WhatsApp</code> chat exports · <code>.mbox</code> mail.
  Multiple paths can queue concurrently — each runs as its own job with
  live progress below.
</p>

<div class="dest">
  <div class="dest-head">
    <span class="dest-label">DESTINATION COLLECTION</span>
    <span class="dest-preview">{destLabel}</span>
  </div>
  <div class="dest-row">
    <label class="dest-choice">
      <input type="radio" name="dest" value="global"
        checked={destMode === 'global'}
        onchange={() => (destMode = 'global')} />
      <span>
        FOLLOW TOP-BAR SCOPE
        <span class="hint">
          {collection.activeName
            ? `currently ${collection.activeName}`
            : 'currently ALL, whole vault'}
        </span>
      </span>
    </label>
    <label class="dest-choice">
      <input type="radio" name="dest" value="none"
        checked={destMode === ''}
        onchange={() => (destMode = '')} />
      <span>NO COLLECTION <span class="hint">whole vault, no membership added</span></span>
    </label>
    {#if collection.list.length > 0}
      <label class="dest-choice">
        <input type="radio" name="dest" value="pick"
          checked={typeof destMode === 'number'}
          onchange={() => {
            if (typeof destMode !== 'number') {
              destMode = collection.list[0].id;
            }
          }} />
        <span>
          PIN TO
          <select class="select inline"
            disabled={typeof destMode !== 'number'}
            bind:value={destMode}>
            {#each collection.list as c (c.id)}
              <option value={c.id}>{c.name}</option>
            {/each}
          </select>
        </span>
      </label>
    {/if}
  </div>
  <div class="dest-new">
    <span class="dest-label">OR CREATE A NEW COLLECTION FOR THIS JOB</span>
    <input type="text" class="input"
      placeholder="e.g. work, journal, slack-2026"
      bind:value={newCollectionDraft}
      disabled={createBusy} />
    <p class="hint">
      If filled, the job ignores the choices above and assigns every
      ingested document to a collection with this name (created if it
      does not yet exist).
    </p>
  </div>
</div>

<div class="row">
  <input type="text" class="input grow" placeholder="/Users/you/Documents/slack-export"
    bind:value={path}
    onkeydown={(e) => { if (e.key === 'Enter') go(); }} />
  <button class="btn btn-secondary btn-sm" onclick={openBrowse}>BROWSE…</button>
  <button class="btn btn-primary btn-sm" onclick={go}>INGEST <kbd>⌘↵</kbd></button>
</div>
<StatusLine text={status} tone={statusTone} />

<div class="section-label">JOBS</div>

{#if jobs.length === 0}
  <div class="empty"><span class="glyph">::</span>no ingest jobs yet</div>
{:else}
  {#each jobs as j (j.id)}
    <div class="job {j.status}">
      <div class="head">
        <div class="path">{j.path}</div>
        <span class="status {j.status}">{j.status}</span>
      </div>
      <div class="meta">
        <span><b>{fmtNum(j.records_added)}</b> added</span>
        <span><b>{fmtNum(j.records_skipped)}</b> skipped</span>
        {#if j.records_failed}
          <span><b>{fmtNum(j.records_failed)}</b> failed</span>
        {/if}
        {#if j.files_total}
          <span><b>{fmtNum(j.files_seen)}</b> / <b>{fmtNum(j.files_total)}</b> files</span>
        {:else}
          <span><b>{fmtNum(j.files_seen)}</b> seen</span>
        {/if}
        {#if j.archive_kind}
          <span class="archive">archive: {j.archive_kind}</span>
        {/if}
        {#if j.status === 'running' || j.status === 'pending'}
          <span>elapsed: <b>{fmtDuration(j.started_at)}</b></span>
          {#if rate(j)}<span>rate: <b>{rate(j)}</b></span>{/if}
        {:else}
          <span>took <b>{fmtDuration(j.started_at, j.finished_at)}</b> · finished {fmtTime(j.finished_at)}</span>
        {/if}
      </div>
      {#if j.current_file && (j.status === 'running' || j.status === 'pending')}
        <div class="current">→ {j.current_file}</div>
      {/if}
      {#if j.error}<div class="error">{j.error}</div>{/if}
      {#if j.status === 'running' || j.status === 'pending'}
        {@const frac = progressFrac(j)}
        {#if frac !== null}
          <div class="bar determinate">
            <div class="fill-det" style="width: {(frac * 100).toFixed(1)}%"></div>
          </div>
        {:else}
          <div class="bar"><div class="fill"></div></div>
        {/if}
      {/if}
    </div>
  {/each}
{/if}

<Modal open={bOpen} title="PICK A FOLDER" onClose={() => (bOpen = false)}>
  <div class="modal-path">{bPath || '~'}</div>
  <div class="modal-list">
    {#if bError}
      <div class="empty">{bError}</div>
    {:else if !bEntries.length}
      <div class="empty"><span class="glyph">::</span>empty folder</div>
    {:else}
      {#each bEntries as e (e.path)}
        <div class="entry"
          role="button" tabindex="0"
          onclick={() => pickEntry(e)}
          onkeydown={(ev) => ev.key === 'Enter' && pickEntry(e)}>
          <span class={e.kind === 'dir' ? 'glyph-d' : 'glyph-f'}>{e.kind === 'dir' ? '▸' : '·'}</span>
          <span>{e.name}</span>
        </div>
      {/each}
    {/if}
  </div>
  {#snippet footer()}
    <button class="btn btn-secondary btn-sm" onclick={browseUp}>↑ PARENT</button>
    <button class="btn btn-primary btn-sm" onclick={() => { path = bPath; bOpen = false; }}>USE THIS FOLDER</button>
  {/snippet}
</Modal>

<style>
  .instructions {
    color: var(--text-dim);
    font-size: 12px;
    line-height: 1.7;
    margin: 0 0 18px;
  }
  .instructions strong { color: var(--text-bright); font-weight: 700; }
  .instructions em { color: var(--amber); font-style: normal; }
  .instructions code {
    color: var(--teal);
    background: var(--teal-soft);
    padding: 1px 6px;
    font-size: 11px;
  }
  .dest {
    background: var(--surface);
    border: 1px solid var(--border);
    padding: 14px 16px;
    margin-bottom: 14px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .dest-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    gap: 12px;
    flex-wrap: wrap;
  }
  .dest-label {
    font-size: 9px;
    letter-spacing: 2px;
    text-transform: uppercase;
    color: var(--text-dim);
    font-weight: 700;
  }
  .dest-preview {
    color: var(--amber);
    font-size: 11px;
    letter-spacing: 1px;
    text-transform: uppercase;
    font-weight: 700;
  }
  .dest-row {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .dest-choice {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    font-size: 11px;
    color: var(--text);
    cursor: pointer;
    line-height: 1.5;
  }
  .dest-choice input[type='radio'] {
    margin-top: 3px;
    accent-color: var(--amber);
    cursor: pointer;
  }
  .dest-choice .hint {
    display: block;
    color: var(--text-dim);
    font-size: 10px;
    font-weight: 400;
    text-transform: none;
    letter-spacing: 0.3px;
    margin-top: 2px;
  }
  .dest-choice .select.inline {
    margin-left: 8px;
    padding: 2px 8px;
    font-size: 11px;
  }
  .dest-new {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-top: 10px;
    border-top: 1px dashed var(--border);
  }
  .dest-new .hint {
    color: var(--text-dim);
    font-size: 10px;
    margin: 0;
    line-height: 1.5;
  }

  .row { display: flex; gap: 10px; flex-wrap: wrap; margin-bottom: 14px; align-items: center; }
  .grow { flex: 1; min-width: 220px; }
  .job {
    background: var(--surface);
    border: 1px solid var(--border);
    padding: 14px 16px;
    margin-bottom: 10px;
    border-left: 2px solid var(--text-dim);
  }
  .job.running, .job.pending { border-left-color: var(--teal); }
  .job.completed { border-left-color: var(--amber); }
  .job.failed { border-left-color: var(--danger); }
  .head {
    display: flex; justify-content: space-between; gap: 14px;
    align-items: flex-start; margin-bottom: 8px;
  }
  .path { font-size: 12px; color: var(--text); word-break: break-all; flex: 1; }
  .status {
    font-size: 9px; font-weight: 700; letter-spacing: 2px;
    text-transform: uppercase;
    padding: 3px 8px; flex-shrink: 0;
    border: 1px solid var(--border);
  }
  .status.running, .status.pending {
    color: var(--teal); border-color: var(--teal); background: var(--teal-soft);
  }
  .status.completed {
    color: var(--amber); border-color: var(--amber-line); background: var(--amber-soft);
  }
  .status.failed {
    color: var(--danger); border-color: var(--danger); background: var(--danger-soft);
  }
  .meta {
    display: flex; flex-wrap: wrap; gap: 6px 16px;
    font-size: 10px; color: var(--text-dim); letter-spacing: 1px;
  }
  .meta b { color: var(--text-bright); font-weight: 700; }
  .archive { color: var(--amber); font-weight: 700; }
  .current {
    font-size: 10px; color: var(--text-dim);
    margin-top: 6px; overflow: hidden; text-overflow: ellipsis;
    white-space: nowrap;
  }
  .error { color: var(--danger); font-size: 11px; margin-top: 6px; }
  .bar {
    height: 2px; background: var(--border);
    margin-top: 8px; overflow: hidden; position: relative;
  }
  .fill {
    height: 100%; background: var(--amber); width: 30%;
    animation: pulse 1.4s ease-in-out infinite;
  }
  .bar.determinate { background: var(--border); }
  .fill-det {
    height: 100%; background: var(--teal);
    transition: width 300ms ease-out;
  }
  @keyframes pulse { 0% { transform: translateX(-100%); } 100% { transform: translateX(400%); } }

  .modal-path {
    padding: 10px 18px;
    font-size: 11px;
    color: var(--teal);
    border-bottom: 1px solid var(--border);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .modal-list {
    flex: 1; overflow-y: auto; padding: 6px 0; max-height: 50vh;
  }
  .entry {
    padding: 8px 18px; cursor: pointer; font-size: 12px;
    display: flex; gap: 10px; align-items: center;
    color: var(--text);
  }
  .entry:hover { background: var(--surface-2); }
  .glyph-d { color: var(--amber); }
  .glyph-f { color: var(--text-dim); }
</style>
