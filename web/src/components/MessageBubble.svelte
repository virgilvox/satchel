<script lang="ts">
  import { marked } from 'marked';
  import type { ChatMessage } from '../lib/types';
  import ToolCallCard from './ToolCallCard.svelte';
  import ReasoningBlock from './ReasoningBlock.svelte';
  import ResultRow from './ResultRow.svelte';

  interface Props {
    message: ChatMessage;
  }
  let { message }: Props = $props();

  // Markdown for assistant text. `breaks: true` so single newlines render
  // as line breaks (chat-friendly); `gfm: true` for code fences and tables.
  marked.setOptions({ breaks: true, gfm: true });

  let body = $derived(
    message.role === 'assistant' ? (marked.parse(message.content) as string) : null
  );
</script>

<div class="message {message.role}">
  <div class="label">
    {message.role.toUpperCase()}
    {#if message.streaming}<span class="caret">▌</span>{/if}
  </div>
  <div class="body">
    {#if message.reasoning}
      <ReasoningBlock text={message.reasoning} />
    {/if}
    {#if message.toolCalls}
      {#each message.toolCalls as call (call.id)}
        <ToolCallCard {call} />
      {/each}
    {/if}
    {#if message.role === 'assistant'}
      <div class="md">{@html body}</div>
    {:else if message.role === 'error'}
      {message.content}
    {:else}
      {message.content}
    {/if}
    {#if message.retrieval && message.retrieval.length > 0}
      <div class="retrieval">
        {#each message.retrieval as r, i (i)}
          <!-- No truncate — ResultRow has its own scroll-clip via
               max-height + overflow-y:auto, same as the Search tab.
               Hard-truncating to 600 chars was ignoring half of every
               long mbox email body. -->
          <ResultRow result={r} />
        {/each}
      </div>
    {/if}
  </div>
</div>

<style>
  .message {
    margin-bottom: 22px;
    padding-left: 18px;
    border-left: 3px solid var(--text-dim);
  }
  .message.assistant { border-left-color: var(--amber); }
  .message.error { border-left-color: var(--danger); }
  .message.tool { border-left-color: var(--teal); }
  .message.system { border-left-color: var(--border-strong); }
  .label {
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 2.5px;
    color: var(--text-dim);
    text-transform: uppercase;
    margin-bottom: 8px;
  }
  .message.assistant .label { color: var(--amber); }
  .message.error .label { color: var(--danger); }
  .message.tool .label { color: var(--teal); }
  .body {
    font-size: 13px;
    line-height: 1.7;
    color: var(--text);
    word-wrap: break-word;
  }
  .body :global(code) {
    color: var(--teal);
    background: var(--teal-soft);
    padding: 1px 6px;
    font-size: 12px;
  }
  .body :global(pre) {
    background: var(--bg-deep);
    border: 1px solid var(--border);
    padding: 10px 12px;
    overflow-x: auto;
    font-size: 11px;
    line-height: 1.55;
    margin: 10px 0;
  }
  .body :global(pre code) {
    background: transparent;
    padding: 0;
  }
  .body :global(p) { margin: 0 0 8px; }
  .body :global(p:last-child) { margin-bottom: 0; }
  .body :global(ul), .body :global(ol) {
    margin: 6px 0 10px 22px;
  }
  .caret {
    color: var(--amber);
    animation: blink 1s steps(2) infinite;
  }
  @keyframes blink { 50% { opacity: 0; } }
  .md :global(a) { color: var(--teal); }
  .retrieval { margin-top: 14px; }
</style>
