// SVG markup for the punk/displaced notched-pin mark. Kept as raw strings
// so any component can drop it inline without importing yet another SVG
// component. The CSS variables defined in tokens.css carry the per-mode
// stroke + mid-line + bg-cut colors.

export const PUNK_FILTER_DEFS = `
<defs>
  <filter id="punk" x="-5%" y="-5%" width="110%" height="110%">
    <feTurbulence type="fractalNoise" baseFrequency="0.04" numOctaves="4" result="n"/>
    <feDisplacementMap in="SourceGraphic" in2="n" scale="2"/>
  </filter>
  <filter id="punk-strong" x="-5%" y="-5%" width="110%" height="110%">
    <feTurbulence type="fractalNoise" baseFrequency="0.04" numOctaves="4" result="n"/>
    <feDisplacementMap in="SourceGraphic" in2="n" scale="2.2"/>
  </filter>
</defs>`;

/** The notched-pin mark contents. Caller wraps in a sized <svg> + filter group. */
export const MARK_BODY = `
<rect x="56"  y="166" width="8" height="20" fill="var(--mark-stroke)"/>
<rect x="76"  y="166" width="8" height="20" fill="var(--mark-stroke)"/>
<rect x="96"  y="166" width="8" height="20" fill="var(--mark-stroke)"/>
<rect x="116" y="166" width="8" height="20" fill="var(--mark-stroke)"/>
<rect x="136" y="166" width="8" height="20" fill="var(--mark-stroke)"/>
<path d="M 40 20 L 136 20 L 160 44 L 160 166 L 40 166 Z" fill="none" stroke="var(--mark-stroke)" stroke-width="6" stroke-linejoin="miter"/>
<line x1="56" y1="58"  x2="120" y2="58"  stroke="#5eead4" stroke-width="5" stroke-linecap="round"/>
<line x1="56" y1="88"  x2="144" y2="88"  stroke="#5eead4" stroke-width="5" stroke-linecap="round"/>
<line x1="56" y1="118" x2="144" y2="118" stroke="var(--mark-mid-line)" stroke-width="5" stroke-linecap="round"/>
<line x1="56" y1="148" x2="144" y2="148" stroke="#5eead4" stroke-width="5" stroke-linecap="round"/>
<circle cx="100" cy="118" r="7" fill="var(--bg)" stroke="var(--mark-stroke)" stroke-width="3.5"/>
<circle cx="100" cy="118" r="2.8" fill="#5eead4"/>
<circle cx="132" cy="58" r="2.8" fill="var(--mark-stroke)"/>`;

/** Punk variant — wraps the body in the filter and adds CSS-variable colors. */
export function markSvg(size = 24): string {
  return `<svg width="${size}" height="${size}" viewBox="0 0 200 200" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
  ${PUNK_FILTER_DEFS}
  <g filter="url(#punk)">${MARK_BODY}</g>
</svg>`;
}
