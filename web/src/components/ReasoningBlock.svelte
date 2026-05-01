<script lang="ts">
  interface Props {
    text: string;
    /** Default to collapsed; users opt in to read the full chain. */
    open?: boolean;
  }
  let { text, open = false }: Props = $props();
</script>

<details class="reasoning" {open}>
  <summary>
    <span class="glyph">∴</span>
    <span>REASONING</span>
    <span class="meta">{text.length} chars</span>
  </summary>
  <div class="body">{text}</div>
</details>

<style>
  .reasoning {
    margin: 10px 0;
    border: 1px dashed var(--border-strong);
    background: var(--surface);
  }
  summary {
    cursor: pointer;
    list-style: none;
    padding: 8px 12px;
    font-size: 10px;
    letter-spacing: 2px;
    text-transform: uppercase;
    color: var(--text-dim);
    display: flex;
    align-items: center;
    gap: 10px;
    user-select: none;
  }
  summary::-webkit-details-marker { display: none; }
  summary::before {
    content: '▸';
    color: var(--text-dim);
    transition: transform 120ms ease;
    display: inline-block;
  }
  details[open] summary::before { transform: rotate(90deg); }
  .glyph { color: var(--amber); font-weight: 700; }
  .meta { margin-left: auto; font-size: 9px; }
  .body {
    padding: 12px 14px;
    border-top: 1px dashed var(--border);
    font-size: 11px;
    line-height: 1.65;
    white-space: pre-wrap;
    color: var(--text-dim);
    font-style: italic;
    max-height: 360px;
    overflow-y: auto;
  }
</style>
