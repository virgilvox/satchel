<script lang="ts">
  interface Props {
    placeholder?: string;
    disabled?: boolean;
    sendLabel?: string;
    onSend: (text: string) => void;
    onCancel?: () => void;
    busy?: boolean;
  }
  let {
    placeholder = 'message... (enter to send, shift+enter newline)',
    disabled = false,
    sendLabel = 'SEND',
    onSend,
    onCancel,
    busy = false,
  }: Props = $props();

  let value = $state('');
  let textarea: HTMLTextAreaElement;

  function autosize() {
    if (!textarea) return;
    textarea.style.height = 'auto';
    textarea.style.height = Math.min(textarea.scrollHeight, 200) + 'px';
  }

  function send() {
    const text = value.trim();
    if (!text || disabled) return;
    onSend(text);
    value = '';
    autosize();
  }
</script>

<div class="composer">
  <textarea
    bind:this={textarea}
    bind:value
    class="input"
    rows="1"
    {placeholder}
    {disabled}
    oninput={autosize}
    onkeydown={(e) => {
      if (e.key === 'Enter' && !e.shiftKey) {
        e.preventDefault();
        send();
      }
    }}
  ></textarea>
  {#if busy && onCancel}
    <button class="btn btn-secondary" type="button" onclick={onCancel}>STOP</button>
  {:else}
    <button class="btn btn-primary" type="button" {disabled} onclick={send}>
      {sendLabel} <kbd>⌘↵</kbd>
    </button>
  {/if}
</div>

<style>
  .composer {
    position: sticky;
    bottom: 0;
    background: var(--bg);
    border-top: 1px solid var(--border);
    padding: 14px 0;
    margin-top: 22px;
    display: flex;
    gap: 12px;
    align-items: flex-end;
  }
  textarea.input {
    flex: 1;
    min-height: 48px;
    max-height: 200px;
    resize: none;
    overflow-y: auto;
  }
</style>
