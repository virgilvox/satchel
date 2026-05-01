<script lang="ts">
  interface Props {
    tone?: 'amber' | 'teal' | 'danger' | 'dim';
    pulse?: boolean;
  }
  let { tone = 'dim', pulse = false }: Props = $props();
</script>

<span class="dot {tone}" class:pulse aria-hidden="true"></span>

<style>
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    display: inline-block;
    flex-shrink: 0;
  }
  .amber {
    background: var(--amber);
    box-shadow: 0 0 8px rgba(245, 185, 65, 0.4);
  }
  .teal {
    background: var(--teal);
    box-shadow: 0 0 8px rgba(94, 234, 212, 0.4);
  }
  .danger {
    background: var(--danger);
  }
  .dim {
    background: var(--text-dim);
  }
  :global(:root[data-mode='light']) .amber,
  :global(:root[data-mode='light']) .teal {
    box-shadow: none;
  }
  .pulse {
    animation: dotPulse 1.4s ease-in-out infinite;
  }
  @keyframes dotPulse {
    0%, 100% { opacity: 0.4; }
    50%      { opacity: 1; }
  }
</style>
