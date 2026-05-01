<script lang="ts">
  import Modal from './Modal.svelte';
  import { chatSettings, type ContextSize, type SlidingSize } from '../lib/stores.svelte';

  interface Props {
    open: boolean;
    onClose: () => void;
    /** True when an engine is currently loaded. Context/sliding-window
     *  changes need an UNLOAD + LOAD round-trip to take effect. We grey
     *  those rows out when the engine is hot. */
    engineLoaded: boolean;
  }
  let { open, onClose, engineLoaded }: Props = $props();

  const ctxOptions: ContextSize[] = ['auto', 4096, 8192, 16384, 32768];
  const slidingOptions: SlidingSize[] = ['off', 1024, 2048, 4096, 8192];

  function setCtx(v: string) {
    if (v === 'auto') return chatSettings.setContextWindowSize('auto');
    chatSettings.setContextWindowSize(Number(v) as ContextSize);
  }
  function setSliding(v: string) {
    if (v === 'off') return chatSettings.setSlidingWindowSize('off');
    chatSettings.setSlidingWindowSize(Number(v) as SlidingSize);
  }

  function fmt(v: number, digits = 2): string {
    return v.toFixed(digits).replace(/\.?0+$/, '');
  }
</script>

<Modal {open} title="CHAT SETTINGS" {onClose}>
  <div class="body">
    <!-- ============ Generation ============ -->
    <div class="section-label">GENERATION</div>

    <div class="row">
      <div class="row-head">
        <span class="name">temperature</span>
        <span class="val">{fmt(chatSettings.temperature, 2)}</span>
      </div>
      <input type="range" min="0" max="2" step="0.05"
        value={chatSettings.temperature}
        oninput={(e) => chatSettings.setTemperature(Number((e.target as HTMLInputElement).value))}
      />
      <p class="desc">Sampling temperature. Higher = more creative; lower = more deterministic. 0.6 is a good default for tool-using agents.</p>
    </div>

    <div class="row">
      <div class="row-head">
        <span class="name">max_tokens</span>
        <span class="val">{chatSettings.maxTokens}</span>
      </div>
      <input type="range" min="256" max="4096" step="128"
        value={chatSettings.maxTokens}
        oninput={(e) => chatSettings.setMaxTokens(Number((e.target as HTMLInputElement).value))}
      />
      <p class="desc">Maximum tokens the model can emit per turn.</p>
    </div>

    <div class="row">
      <div class="row-head">
        <span class="name">max_rounds</span>
        <span class="val">{chatSettings.maxRounds}</span>
      </div>
      <input type="range" min="1" max="20" step="1"
        value={chatSettings.maxRounds}
        oninput={(e) => chatSettings.setMaxRounds(Number((e.target as HTMLInputElement).value))}
      />
      <p class="desc">Hard cap on tool-call rounds per user turn.</p>
    </div>

    <!-- ============ Agent loop ============ -->
    <div class="section-label">AGENT</div>

    <div class="row">
      <div class="row-head">
        <span class="name">min_tool_calls</span>
        <span class="val">{chatSettings.minToolCalls}</span>
      </div>
      <input type="range" min="0" max="5" step="1"
        value={chatSettings.minToolCalls}
        oninput={(e) => chatSettings.setMinToolCalls(Number((e.target as HTMLInputElement).value))}
      />
      <p class="desc">Persistence backstop. Model can't `respond_to_user` until it has called real tools at least this many times. 0 disables.</p>
    </div>

    <div class="row">
      <div class="row-head">
        <span class="name">weak_score_threshold</span>
        <span class="val">{fmt(chatSettings.weakScoreThreshold, 3)}</span>
      </div>
      <input type="range" min="0" max="0.5" step="0.01"
        value={chatSettings.weakScoreThreshold}
        oninput={(e) => chatSettings.setWeakScoreThreshold(Number((e.target as HTMLInputElement).value))}
      />
      <p class="desc">RRF scores below this are "noise" — the system prompt tells the model not to base its final answer on results scoring below this line.</p>
    </div>

    <!-- ============ Context (engine-load) ============ -->
    <div class="section-label">
      CONTEXT
      {#if engineLoaded}<span class="hint">· UNLOAD + RELOAD to apply</span>{/if}
    </div>

    <div class="row" class:disabled={engineLoaded}>
      <div class="row-head">
        <span class="name">context_window_size</span>
        <span class="val">{chatSettings.contextWindowSize}</span>
      </div>
      <select class="select"
        disabled={engineLoaded}
        value={String(chatSettings.contextWindowSize)}
        onchange={(e) => setCtx((e.target as HTMLSelectElement).value)}
      >
        {#each ctxOptions as opt (opt)}
          <option value={String(opt)}>{opt}</option>
        {/each}
      </select>
      <p class="desc">Override the model's compiled context window. Hermes 3 3B defaults to 4096; 8192 fits ~3× more tool turns. Going past the model's compile-time max throws an error at LOAD.</p>
    </div>

    <div class="row" class:disabled={engineLoaded}>
      <div class="row-head">
        <span class="name">sliding_window_size</span>
        <span class="val">{chatSettings.slidingWindowSize}</span>
      </div>
      <select class="select"
        disabled={engineLoaded}
        value={String(chatSettings.slidingWindowSize)}
        onchange={(e) => setSliding((e.target as HTMLSelectElement).value)}
      >
        {#each slidingOptions as opt (opt)}
          <option value={String(opt)}>{opt}</option>
        {/each}
      </select>
      <p class="desc">Last-resort fallback when the transcript outgrows context: keep only the last N tokens in attention, drop older. Less ideal for tool-calling (the model loses earlier observations) but prevents hard failures.</p>
    </div>

    <!-- ============ Persistence ============ -->
    <div class="section-label">PERSISTENCE</div>

    <label class="check">
      <input type="checkbox"
        checked={chatSettings.persistHistory}
        onchange={(e) => chatSettings.setPersistHistory((e.target as HTMLInputElement).checked)}
      />
      <span>
        <span class="name">persist_history</span>
        <p class="desc">Keep the chat transcript in localStorage so a refresh doesn't nuke it.</p>
      </span>
    </label>

    <label class="check">
      <input type="checkbox"
        checked={chatSettings.showSystemPrompt}
        onchange={(e) => chatSettings.setShowSystemPrompt((e.target as HTMLInputElement).checked)}
      />
      <span>
        <span class="name">show_system_prompt</span>
        <p class="desc">Reveal the system prompt being sent to the model (useful when debugging unexpected behavior).</p>
      </span>
    </label>
  </div>

  {#snippet footer()}
    <button class="btn btn-primary btn-sm" type="button" onclick={onClose}>DONE</button>
  {/snippet}
</Modal>

<style>
  .body {
    overflow-y: auto;
    padding: 18px;
    display: flex;
    flex-direction: column;
    gap: 18px;
  }
  .section-label {
    font-size: 10px;
    letter-spacing: 2.5px;
    color: var(--text-dim);
    margin: 14px 0 0;
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .section-label::before { content: '■'; color: var(--amber); font-size: 10px; }
  .section-label::after { content: ''; flex: 1; border-bottom: 1px solid var(--border); }
  .section-label .hint {
    font-size: 9px;
    letter-spacing: 1.5px;
    color: var(--amber);
    text-transform: none;
    flex: none;
  }

  .row { display: flex; flex-direction: column; gap: 6px; }
  .row.disabled { opacity: 0.5; }
  .row-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 11px;
    letter-spacing: 1.5px;
  }
  .name { color: var(--text-bright); font-weight: 700; }
  .val { color: var(--amber); font-weight: 700; }
  .desc {
    font-size: 11px;
    color: var(--text-dim);
    line-height: 1.55;
    margin: 0;
  }

  .check {
    display: flex;
    gap: 12px;
    align-items: flex-start;
    cursor: pointer;
    padding: 6px 0;
  }
  .check input {
    flex-shrink: 0;
    margin-top: 2px;
    accent-color: var(--amber);
    width: 14px;
    height: 14px;
  }
  .check span { display: flex; flex-direction: column; gap: 4px; }

  /* Style the range slider track + thumb to match the brand. */
  input[type='range'] {
    -webkit-appearance: none;
    appearance: none;
    width: 100%;
    height: 4px;
    background: var(--border);
    outline: none;
  }
  input[type='range']::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 14px;
    height: 14px;
    background: var(--amber);
    cursor: pointer;
  }
  input[type='range']::-moz-range-thumb {
    width: 14px;
    height: 14px;
    background: var(--amber);
    border: 0;
    cursor: pointer;
  }
</style>
