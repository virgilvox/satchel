<script lang="ts">
  import ViewHead from '../components/ViewHead.svelte';
  import Mark from '../components/Mark.svelte';
  import Composer from '../components/Composer.svelte';
  import MessageBubble from '../components/MessageBubble.svelte';
  import { api } from '../lib/api';
  import type { ChatMessage, ToolCallResult } from '../lib/types';

  let transcript = $state<ChatMessage[]>([]);
  let busy = $state(false);
  const ASK_LIMIT = 5;

  const examples = [
    'recent decisions about hiring',
    'how does the ingest pipeline detect Slack exports',
    'notes on bylaws and quorum rules',
  ];

  let stream: HTMLDivElement;

  async function ask(query: string) {
    if (busy) return;
    busy = true;
    transcript = [...transcript, { id: crypto.randomUUID(), role: 'user', content: query }];

    const callId = crypto.randomUUID();
    const pendingCall: ToolCallResult = {
      id: callId, name: 'search_knowledge',
      args: { query, limit: ASK_LIMIT }, pending: true,
    };
    const aId = crypto.randomUUID();
    transcript = [...transcript, {
      id: aId, role: 'assistant', content: '', toolCalls: [pendingCall], streaming: true,
    }];

    setTimeout(() => stream?.scrollTo({ top: stream.scrollHeight, behavior: 'smooth' }));

    try {
      const r = await api.search(query, ASK_LIMIT, 0);
      const updated: ToolCallResult = {
        ...pendingCall, pending: false,
        result: r.error
          ? undefined
          : `${r.results?.length ?? 0} of ${r.total ?? 0} matches`,
        error: r.error,
      };
      const summary = r.error
        ? `Search failed.`
        : (r.results?.length ?? 0) === 0
          ? `Nothing in the vault matched. Try a different phrasing or ingest more sources.`
          : `Top ${r.results?.length} match${r.results?.length === 1 ? '' : 'es'} for "${query}":`;

      transcript = transcript.map((m) =>
        m.id !== aId
          ? m
          : {
              ...m,
              streaming: false,
              toolCalls: [updated],
              content: summary,
              retrieval: r.error ? undefined : r.results,
            }
      );
    } catch (e) {
      const err = (e as Error).message;
      transcript = transcript.map((m) =>
        m.id !== aId
          ? m
          : { ...m, streaming: false, role: 'error' as const, content: err }
      );
    } finally {
      busy = false;
    }
  }
</script>

<ViewHead num="02" title={`ASK <span class="slash">/</span> CONVERSATIONAL SEARCH`}
  desc="Phrase a question; the vault answers with the most relevant passages. Pure retrieval — no LLM." />

<div class="stream" bind:this={stream}>
  {#if transcript.length === 0}
    <div class="empty-state">
      <Mark size={80} strong />
      <h3>ASK YOUR VAULT</h3>
      <p>Type a question or phrase below. SATCHEL runs hybrid retrieval (embedding + FTS) and shows the top passages with source attribution. For tool-using assistants, point them at the MCP endpoint.</p>
      <div class="examples">
        {#each examples as q (q)}
          <button class="example" type="button" onclick={() => ask(q)}>{q}</button>
        {/each}
      </div>
    </div>
  {:else}
    {#each transcript as m (m.id)}
      <MessageBubble message={m} />
    {/each}
  {/if}
</div>

<Composer
  onSend={ask}
  {busy}
  placeholder="ask the vault... (enter to send, shift+enter newline)"
/>

<style>
  .stream {
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    max-height: calc(100vh - 280px);
  }
  .empty-state {
    text-align: center;
    padding: 60px 24px;
    color: var(--text-dim);
    border: 1px dashed var(--border);
    margin-bottom: 18px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 18px;
  }
  .empty-state h3 {
    font-size: 14px; letter-spacing: 4px;
    text-transform: uppercase; color: var(--amber); font-weight: 700;
  }
  .empty-state p {
    font-size: 12px; line-height: 1.7;
    max-width: 440px;
  }
  .examples {
    display: flex; flex-direction: column; gap: 8px;
    margin-top: 22px; max-width: 480px;
  }
  .example {
    background: var(--surface);
    border: 1px solid var(--border);
    padding: 10px 14px;
    font-size: 11px;
    color: var(--text);
    cursor: pointer;
    text-align: left;
    letter-spacing: 0.3px;
    font-family: inherit;
    transition: 120ms ease;
  }
  .example::before {
    content: '*~'; color: var(--amber); margin-right: 10px; font-weight: 700;
  }
  .example:hover { border-color: var(--amber-line); color: var(--text-bright); }
</style>
