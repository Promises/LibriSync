// Formatting helpers. All locale-aware formatting goes through Intl with the browser's
// locale, so dates and numbers read the way the user expects.

const dateTimeFmt = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium', timeStyle: 'short' });
const dateFmt = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium' });
const timeFmt = new Intl.DateTimeFormat(undefined, { hour: '2-digit', minute: '2-digit' });
const relFmt = new Intl.RelativeTimeFormat(undefined, { numeric: 'auto' });
const numberFmt = new Intl.NumberFormat();

function toDate(value: string | Date): Date | null {
  const d = value instanceof Date ? value : new Date(value);
  return Number.isNaN(d.getTime()) ? null : d;
}

export function formatDateTime(value: string | null | undefined): string {
  if (!value) return '—';
  const d = toDate(value);
  return d ? dateTimeFmt.format(d) : value;
}

export function formatDate(value: string | null | undefined): string {
  if (!value) return '—';
  const d = toDate(value);
  return d ? dateFmt.format(d) : value;
}

/** Time of day if today, else date and time. */
export function formatWhen(value: string | null | undefined, now = Date.now()): string {
  if (!value) return '—';
  const d = toDate(value);
  if (!d) return value;
  const sameDay = new Date(now).toDateString() === d.toDateString();
  return sameDay ? timeFmt.format(d) : dateTimeFmt.format(d);
}

const REL_UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ['year', 365 * 24 * 3600],
  ['month', 30 * 24 * 3600],
  ['week', 7 * 24 * 3600],
  ['day', 24 * 3600],
  ['hour', 3600],
  ['minute', 60],
  ['second', 1],
];

/** "3 hours ago", "in 20 minutes". */
export function formatRelative(value: string | null | undefined, now = Date.now()): string {
  if (!value) return 'never';
  const d = toDate(value);
  if (!d) return value;
  const diffSec = (d.getTime() - now) / 1000;
  const abs = Math.abs(diffSec);
  if (abs < 45) return diffSec < 0 ? 'just now' : 'in a moment';
  for (const [unit, sec] of REL_UNITS) {
    if (abs >= sec || unit === 'second') return relFmt.format(Math.round(diffSec / sec), unit);
  }
  return relFmt.format(Math.round(diffSec), 'second');
}

/** "2d 4h", "3h 12m", "45m 10s", "8s". */
export function formatDuration(totalSeconds: number): string {
  const s = Math.max(0, Math.round(totalSeconds));
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (d > 0) return `${d}d ${h}h`;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${sec.toString().padStart(2, '0')}s`;
  return `${sec}s`;
}

/** Book length: "11 h 32 min". */
export function formatLength(minutes: number): string {
  if (!minutes || minutes <= 0) return '—';
  const h = Math.floor(minutes / 60);
  const m = Math.round(minutes % 60);
  if (h === 0) return `${m} min`;
  return m === 0 ? `${h} h` : `${h} h ${m} min`;
}

const BYTE_UNITS = ['B', 'KB', 'MB', 'GB', 'TB'];

export function formatBytes(bytes: number | null | undefined): string {
  if (bytes === null || bytes === undefined || !Number.isFinite(bytes)) return '—';
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < BYTE_UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v >= 100 || i === 0 ? Math.round(v) : v.toFixed(1)} ${BYTE_UNITS[i]}`;
}

export function formatSpeed(bytesPerSec: number | null | undefined): string {
  if (!bytesPerSec) return '—';
  return `${formatBytes(bytesPerSec)}/s`;
}

export function formatNumber(n: number): string {
  return numberFmt.format(n);
}

export function formatPercent(fraction: number | null | undefined): string {
  if (fraction === null || fraction === undefined) return '—';
  return `${Math.round(Math.min(1, Math.max(0, fraction)) * 100)}%`;
}

export function joinNames(names: string[], max = 3): string {
  if (names.length === 0) return 'Unknown';
  if (names.length <= max) return names.join(', ');
  return `${names.slice(0, max).join(', ')} +${names.length - max}`;
}

export function pluralize(n: number, one: string, many = `${one}s`): string {
  return `${formatNumber(n)} ${n === 1 ? one : many}`;
}

/**
 * Store descriptions arrive as HTML (Audible's publisher summary). Render them as plain
 * paragraphs: DOMParser builds an inert document (no script runs, no image loads), and
 * only its text is used.
 */
export function htmlToParagraphs(html: string): string[] {
  if (!html) return [];
  const marked = html
    .replace(/<\s*br\s*\/?>/gi, '\n')
    .replace(/<\/\s*(p|div|li|h[1-6])\s*>/gi, '\n\n');
  const doc = new DOMParser().parseFromString(marked, 'text/html');
  const text = doc.body.textContent ?? '';
  return text
    .split(/\n\s*\n/)
    .map((p) => p.replace(/\s+/g, ' ').trim())
    .filter(Boolean);
}
