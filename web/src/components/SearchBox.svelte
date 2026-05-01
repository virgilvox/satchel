<script lang="ts">
  interface Props {
    value: string;
    placeholder?: string;
    onsubmit?: (value: string) => void;
    oninput?: (value: string) => void;
  }
  let { value = $bindable(''), placeholder = 'search...', onsubmit, oninput }: Props = $props();
</script>

<div class="search-box">
  <span class="icon" aria-hidden="true">
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
      <circle cx="11" cy="11" r="8" />
      <path d="m21 21-4.35-4.35" />
    </svg>
  </span>
  <input
    type="text"
    class="input"
    {placeholder}
    bind:value
    oninput={(e) => oninput?.((e.target as HTMLInputElement).value)}
    onkeydown={(e) => {
      if (e.key === 'Enter') onsubmit?.(value);
    }}
  />
</div>

<style>
  .search-box { position: relative; }
  .search-box :global(.input) { padding-left: 40px; }
  .icon {
    position: absolute;
    left: 14px;
    top: 50%;
    transform: translateY(-50%);
    color: var(--text-dim);
    line-height: 0;
    pointer-events: none;
  }
  .icon svg { width: 14px; height: 14px; }
</style>
