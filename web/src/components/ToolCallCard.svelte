<script lang="ts">
  import type { ToolCallResult } from '../lib/types';

  interface Props {
    call: ToolCallResult;
  }
  let { call }: Props = $props();

  let argsPreview = $derived.by(() => {
    try {
      const j = JSON.stringify(call.args);
      return j.length > 240 ? j.slice(0, 237) + '...' : j;
    } catch {
      return '{}';
    }
  });

  let resultPreview = $derived.by(() => {
    if (!call.result) return null;
    return call.result.length > 800 ? call.result.slice(0, 797) + '...' : call.result;
  });

  let footer = $derived.by(() => {
    if (call.pending) return 'CALLING...';
    if (call.error) return 'ERROR';
    return 'OK';
  });
</script>

<div class="tool-call" class:err={call.error}>
  <div class="line">
    <span class="name">→ {call.name}</span>
    <span class="footer">{footer}</span>
  </div>
  <div class="args">{argsPreview}</div>
  {#if call.error}
    <div class="error">{call.error}</div>
  {:else if resultPreview}
    <pre class="result">{resultPreview}</pre>
  {/if}
</div>

<style>
  .tool-call {
    margin: 12px 0;
    padding: 10px 14px;
    background: var(--bg-deep);
    border: 1px solid var(--border);
    border-left: 2px solid var(--teal);
    font-size: 11px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .tool-call.err {
    border-left-color: var(--danger);
  }
  .line {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
  }
  .name {
    color: var(--teal);
    font-weight: 700;
    letter-spacing: 1px;
  }
  .footer {
    color: var(--text-dim);
    font-size: 10px;
    letter-spacing: 1.5px;
  }
  .args {
    color: var(--text-dim);
    font-size: 10px;
    overflow-wrap: anywhere;
  }
  .result {
    color: var(--text);
    font-size: 11px;
    line-height: 1.55;
    white-space: pre-wrap;
    background: var(--surface);
    border: 1px solid var(--border);
    padding: 8px 10px;
    max-height: 240px;
    overflow-y: auto;
    font-family: inherit;
  }
  .error {
    color: var(--danger);
    font-size: 11px;
  }
</style>
