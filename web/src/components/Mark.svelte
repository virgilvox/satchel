<script lang="ts">
  // The punk/displaced notched-pin SATCHEL mark. CSS variables (--mark-stroke,
  // --mark-mid-line, --bg) carry the per-mode colors so this single component
  // works in both themes without prop changes.
  interface Props {
    size?: number;
    /** Use the stronger displacement scale for hero-size rendering. */
    strong?: boolean;
  }
  let { size = 24, strong = false }: Props = $props();
  let filterId = $derived(strong ? 'punk-strong' : 'punk');
  let scale = $derived(strong ? 2.2 : 2);
  let uid = $derived(`mk-${Math.random().toString(36).slice(2, 8)}`);
</script>

<svg width={size} height={size} viewBox="0 0 200 200" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
  <defs>
    <filter id="{uid}" x="-5%" y="-5%" width="110%" height="110%">
      <feTurbulence type="fractalNoise" baseFrequency="0.04" numOctaves="4" result="n" />
      <feDisplacementMap in="SourceGraphic" in2="n" scale={scale} />
    </filter>
  </defs>
  <g filter="url(#{uid})">
    <rect x="56"  y="166" width="8" height="20" fill="var(--mark-stroke)" />
    <rect x="76"  y="166" width="8" height="20" fill="var(--mark-stroke)" />
    <rect x="96"  y="166" width="8" height="20" fill="var(--mark-stroke)" />
    <rect x="116" y="166" width="8" height="20" fill="var(--mark-stroke)" />
    <rect x="136" y="166" width="8" height="20" fill="var(--mark-stroke)" />
    <path d="M 40 20 L 136 20 L 160 44 L 160 166 L 40 166 Z" fill="none" stroke="var(--mark-stroke)" stroke-width="6" stroke-linejoin="miter" />
    <line x1="56" y1="58"  x2="120" y2="58"  stroke="#5eead4" stroke-width="5" stroke-linecap="round" />
    <line x1="56" y1="88"  x2="144" y2="88"  stroke="#5eead4" stroke-width="5" stroke-linecap="round" />
    <line x1="56" y1="118" x2="144" y2="118" stroke="var(--mark-mid-line)" stroke-width="5" stroke-linecap="round" />
    <line x1="56" y1="148" x2="144" y2="148" stroke="#5eead4" stroke-width="5" stroke-linecap="round" />
    <circle cx="100" cy="118" r="7" fill="var(--bg)" stroke="var(--mark-stroke)" stroke-width="3.5" />
    <circle cx="100" cy="118" r="2.8" fill="#5eead4" />
    <circle cx="132" cy="58" r="2.8" fill="var(--mark-stroke)" />
  </g>
</svg>
