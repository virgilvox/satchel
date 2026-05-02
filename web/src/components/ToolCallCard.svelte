<script lang="ts">
  import type { ToolCallResult } from '../lib/types';

  interface Props {
    call: ToolCallResult;
    /** Default open state. Live (pending) calls open by default;
     *  completed calls collapse to a single line. */
    defaultOpen?: boolean;
  }
  let { call, defaultOpen }: Props = $props();
  let expanded = $state(defaultOpen ?? call.pending);

  let argsLine = $derived.by(() => {
    try {
      const j = JSON.stringify(call.args);
      return j.length > 80 ? j.slice(0, 77) + '…' : j;
    } catch {
      return '{}';
    }
  });

  let argsBlock = $derived.by(() => {
    try {
      return JSON.stringify(call.args, null, 2);
    } catch {
      return '{}';
    }
  });

  let resultPreview = $derived.by(() => {
    if (!call.result) return null;
    return call.result.length > 1200 ? call.result.slice(0, 1197) + '…' : call.result;
  });

  let footer = $derived.by(() => {
    if (call.pending) return 'CALLING';
    if (call.error) return 'ERROR';
    return 'OK';
  });

  let footerTone = $derived.by(() => {
    if (call.pending) return 'pending';
    if (call.error) return 'err';
    return 'ok';
  });

  function toggle() {
    expanded = !expanded;
  }
</script>

<div class="tool-call" class:err={call.error} class:expanded>
  <button class="row" type="button" onclick={toggle} aria-expanded={expanded}>
    <span class="caret" aria-hidden="true">{expanded ? '▾' : '▸'}</span>
    <span class="name">→ {call.name}</span>
    <span class="args-inline">{argsLine}</span>
    <span class="footer {footerTone}">{footer}</span>
  </button>
  {#if expanded}
    <div class="body">
      <div class="label">arguments</div>
      <pre class="args-block">{argsBlock}</pre>
      {#if call.error}
        <div class="label">error</div>
        <div class="error">{call.error}</div>
      {:else if resultPreview}
        <div class="label">result</div>
        <pre class="result">{resultPreview}</pre>
      {/if}
    </div>
  {/if}
</div>

<style>
  .tool-call {
    margin: 8px 0;
    background: var(--bg-deep);
    border: 1px solid var(--border);
    border-left: 2px solid var(--teal);
    font-size: 11px;
  }
  .tool-call.err { border-left-color: var(--danger); }

  .row {
    display: grid;
    grid-template-columns: auto auto 1fr auto;
    align-items: center;
    gap: 10px;
    padding: 6px 12px;
    background: transparent;
    border: 0;
    width: 100%;
    cursor: pointer;
    color: inherit;
    font-family: inherit;
    font-size: inherit;
    text-align: left;
    transition: background 120ms ease;
  }
  .row:hover { background: var(--surface); }
  .caret { color: var(--text-dim); width: 10px; flex-shrink: 0; }
  .name {
    color: var(--teal);
    font-weight: 700;
    letter-spacing: 0.5px;
    flex-shrink: 0;
  }
  .args-inline {
    color: var(--text-dim);
    font-size: 10px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .footer {
    font-size: 9px;
    letter-spacing: 1.5px;
    font-weight: 700;
    flex-shrink: 0;
    padding: 1px 7px;
    border: 1px solid var(--border);
  }
  .footer.ok { color: var(--teal); border-color: var(--teal); }
  .footer.pending {
    color: var(--amber);
    border-color: var(--amber-line);
    animation: pulse 1.4s ease-in-out infinite;
  }
  .footer.err { color: var(--danger); border-color: var(--danger); }
  @keyframes pulse { 0%, 100% { opacity: 0.6; } 50% { opacity: 1; } }

  .body {
    padding: 8px 12px 10px;
    border-top: 1px dashed var(--border);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .label {
    font-size: 9px;
    letter-spacing: 1.5px;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .args-block, .result {
    background: var(--surface);
    border: 1px solid var(--border);
    padding: 8px 10px;
    font-family: inherit;
    font-size: 11px;
    line-height: 1.55;
    color: var(--text);
    margin: 0;
    white-space: pre-wrap;
    overflow-x: auto;
    max-height: 280px;
    overflow-y: auto;
  }
  .args-block { color: var(--text-dim); }
  .error { color: var(--danger); font-size: 11px; }
</style>
