<script lang="ts">
  import { onMount } from 'svelte';
  import ViewHead from '../components/ViewHead.svelte';
  import Modal from '../components/Modal.svelte';
  import StatusLine from '../components/StatusLine.svelte';
  import { api } from '../lib/api';
  import type { BrowseEntry, IngestJob } from '../lib/types';

  let path = $state('');
  let status = $state('');
  let statusTone = $state<'dim' | 'teal' | 'danger'>('dim');

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
      const r = await api.ingest(p);
      if (r.error) { statusTone = 'danger'; status = 'FAILED · ' + r.error; }
      else {
        statusTone = 'teal'; status = 'JOB QUEUED · TRACK PROGRESS BELOW';
        path = '';
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
  desc="Detects Slack, ChatGPT, Claude.ai, Discord, WhatsApp and mbox archives automatically. Each path runs as its own job." />

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
        <span><b>{fmtNum(j.files_seen)}</b> seen</span>
        {#if j.archive_kind}
          <span class="archive">archive: {j.archive_kind}</span>
        {/if}
        {#if j.status === 'running' || j.status === 'pending'}
          <span>elapsed: <b>{fmtDuration(j.started_at)}</b></span>
        {:else}
          <span>took <b>{fmtDuration(j.started_at, j.finished_at)}</b> · finished {fmtTime(j.finished_at)}</span>
        {/if}
      </div>
      {#if j.current_file && (j.status === 'running' || j.status === 'pending')}
        <div class="current">→ {j.current_file}</div>
      {/if}
      {#if j.error}<div class="error">{j.error}</div>{/if}
      {#if j.status === 'running' || j.status === 'pending'}
        <div class="bar"><div class="fill"></div></div>
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
