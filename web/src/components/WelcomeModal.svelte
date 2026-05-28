<script lang="ts">
  import Mark from './Mark.svelte';
  import { router } from '../lib/stores.svelte';
  import type { Tab } from '../lib/types';

  // First-launch welcome. Three numbered steps, plain language, one
  // dismiss action. Once dismissed the flag persists in localStorage
  // under `satchel-welcome-dismissed=1` so the modal does not nag
  // on subsequent loads. Setting the key from elsewhere (or clearing
  // it) re-enables the modal next reload, which is useful for users
  // who want a refresher.

  interface Props {
    onDismiss: () => void;
  }
  let { onDismiss }: Props = $props();

  function go(tab: Tab) {
    router.set(tab);
    onDismiss();
  }

  function onBackdropClick(e: MouseEvent) {
    if (e.target === e.currentTarget) onDismiss();
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') onDismiss();
  }

  $effect(() => {
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });
</script>

<div class="backdrop" role="dialog" aria-modal="true" aria-labelledby="welcome-title"
  tabindex="-1"
  onclick={onBackdropClick}
  onkeydown={(e) => e.key === 'Enter' && onDismiss()}>
  <div class="modal">
    <header class="head">
      <div class="brand">
        <span class="mark"><Mark size={28} strong /></span>
        <div class="brand-text">
          <h2 id="welcome-title">Welcome to SATCHEL</h2>
          <p class="lede">
            A portable knowledge vault that runs entirely on this machine.
            Three steps to get going.
          </p>
        </div>
      </div>
      <button class="x" type="button" aria-label="Dismiss" onclick={onDismiss}>×</button>
    </header>

    <ol class="steps">
      <li class="step">
        <span class="num">01</span>
        <div class="body">
          <h3>Load some files</h3>
          <p>
            Open the Ingest tab and drop a folder of notes, PDFs,
            markdown, or chat exports. SATCHEL chunks, embeds, and
            indexes everything locally. Nothing leaves this machine.
          </p>
          <button class="cta" type="button" onclick={() => go('ingest')}>
            Open Ingest
          </button>
        </div>
      </li>

      <li class="step">
        <span class="num">02</span>
        <div class="body">
          <h3>Hook up an AI client</h3>
          <p>
            On the Connect tab, copy the one-line snippet for Claude
            Desktop, Claude Code, or Cursor and paste it into that
            tool's config. The built-in Chat tab also works with your
            Anthropic API key or a small local model in the browser.
          </p>
          <button class="cta" type="button" onclick={() => go('connect')}>
            Open Connect
          </button>
        </div>
      </li>

      <li class="step">
        <span class="num">03</span>
        <div class="body">
          <h3>Ask your vault</h3>
          <p>
            Try Search for plain hybrid retrieval, Ask for a
            conversational version of the same, or Chat for a real
            agent that calls the search tool on your behalf.
          </p>
          <button class="cta" type="button" onclick={() => go('ask')}>
            Open Ask
          </button>
        </div>
      </li>
    </ol>

    <footer class="foot">
      <button class="dismiss" type="button" onclick={onDismiss}>
        Got it, dismiss
      </button>
    </footer>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.7);
    backdrop-filter: blur(4px);
    -webkit-backdrop-filter: blur(4px);
    z-index: 200;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
  }

  .modal {
    background: var(--bg);
    border: 1px solid var(--border);
    box-shadow: var(--shadow-frame);
    width: min(640px, 100%);
    max-height: 90vh;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
  }

  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
    padding: 24px 28px 18px;
    border-bottom: 1px solid var(--border);
  }
  .brand {
    display: flex;
    align-items: flex-start;
    gap: 14px;
  }
  .mark {
    display: inline-flex;
    align-items: center;
    color: var(--amber);
    flex-shrink: 0;
    margin-top: 2px;
  }
  .brand-text {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  h2 {
    margin: 0;
    font-size: 16px;
    letter-spacing: 4px;
    text-transform: uppercase;
    font-weight: 700;
    color: var(--amber);
  }
  .lede {
    margin: 0;
    font-size: 12px;
    color: var(--text-dim);
    line-height: 1.6;
    max-width: 460px;
  }
  .x {
    background: transparent;
    border: 0;
    color: var(--text-dim);
    font-size: 22px;
    line-height: 1;
    cursor: pointer;
    padding: 0;
    flex-shrink: 0;
  }
  .x:hover { color: var(--text-bright); }

  .steps {
    list-style: none;
    margin: 0;
    padding: 22px 28px;
    display: flex;
    flex-direction: column;
    gap: 18px;
  }
  .step {
    display: grid;
    grid-template-columns: 44px 1fr;
    gap: 16px;
    align-items: flex-start;
  }
  .num {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    border: 1px solid var(--amber);
    background: var(--amber-soft);
    color: var(--amber);
    font-size: 11px;
    letter-spacing: 1.5px;
    font-weight: 700;
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .body h3 {
    margin: 0;
    font-size: 13px;
    letter-spacing: 2px;
    text-transform: uppercase;
    font-weight: 700;
    color: var(--text-bright);
  }
  .body p {
    margin: 0;
    font-size: 12px;
    line-height: 1.65;
    color: var(--text-dim);
  }
  .cta {
    align-self: flex-start;
    margin-top: 6px;
    background: transparent;
    border: 1px solid var(--border);
    color: var(--text);
    padding: 6px 12px;
    font-family: inherit;
    font-size: 10px;
    letter-spacing: 1.5px;
    text-transform: uppercase;
    font-weight: 700;
    cursor: pointer;
    transition: 120ms ease;
  }
  .cta:hover {
    color: var(--amber);
    border-color: var(--amber);
    background: var(--amber-soft);
  }

  .foot {
    padding: 16px 28px 22px;
    border-top: 1px solid var(--border);
    display: flex;
    justify-content: flex-end;
  }
  .dismiss {
    background: var(--amber);
    border: 1px solid var(--amber);
    color: var(--bg);
    padding: 8px 18px;
    font-family: inherit;
    font-size: 11px;
    letter-spacing: 2px;
    text-transform: uppercase;
    font-weight: 700;
    cursor: pointer;
    transition: 120ms ease;
  }
  .dismiss:hover {
    background: transparent;
    color: var(--amber);
  }

  @media (max-width: 540px) {
    .head { padding: 18px 18px 14px; }
    .steps { padding: 16px 18px; gap: 14px; }
    .foot { padding: 14px 18px 18px; }
    h2 { font-size: 14px; }
  }
</style>
