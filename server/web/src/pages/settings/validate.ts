import type { Settings } from '../../api/types';

/** Field path ("audible.dailyLicenceCap") → message. */
export type FieldErrors = Partial<Record<string, string>>;

const HHMM = /^([01]\d|2[0-3]):[0-5]\d$/;

function intIn(v: number, min: number, max: number): boolean {
  return Number.isInteger(v) && v >= min && v <= max;
}

/** Client-side checks for the ranges API.md documents. The server validates again. */
export function validateSettings(s: Settings): FieldErrors {
  const e: FieldErrors = {};

  if (!s.general.instanceName.trim()) e['general.instanceName'] = 'Give this instance a name.';

  if (!s.media.folderTemplate.trim()) e['media.folderTemplate'] = 'The template cannot be empty.';
  else if (!s.media.folderTemplate.includes('{title}'))
    e['media.folderTemplate'] = 'Include {title}, or every book in a series lands in the same folder.';
  if (!s.media.noSeriesFolderTemplate.trim()) e['media.noSeriesFolderTemplate'] = 'The template cannot be empty.';
  else if (!s.media.noSeriesFolderTemplate.includes('{title}'))
    e['media.noSeriesFolderTemplate'] = 'Include {title}, or books by the same author share a folder.';

  if (!intIn(s.sync.intervalHours, 1, 168)) e['sync.intervalHours'] = 'A whole number of hours from 1 to 168.';

  const { windowStart, windowEnd } = s.backfill;
  if ((windowStart === null) !== (windowEnd === null)) e['backfill.windowEnd'] = 'Set both a start and an end time.';
  if (windowStart !== null && !HHMM.test(windowStart)) e['backfill.windowStart'] = 'Use HH:MM.';
  if (windowEnd !== null && !HHMM.test(windowEnd)) e['backfill.windowEnd'] = 'Use HH:MM.';
  if (windowStart !== null && windowStart === windowEnd) e['backfill.windowEnd'] = 'Start and end cannot be the same.';

  if (!intIn(s.audible.dailyLicenceCap, 1, 200)) e['audible.dailyLicenceCap'] = 'A whole number from 1 to 200.';
  if (!intIn(s.audible.spacingMinMinutes, 0, 1440)) e['audible.spacingMinMinutes'] = 'Minutes, 0 or more.';
  if (!intIn(s.audible.spacingMaxMinutes, 0, 1440)) e['audible.spacingMaxMinutes'] = 'Minutes, 0 or more.';
  else if (s.audible.spacingMaxMinutes < s.audible.spacingMinMinutes)
    e['audible.spacingMaxMinutes'] = 'Must be at least the minimum.';
  if (!intIn(s.audible.throttleCooldownHours, 1, 168))
    e['audible.throttleCooldownHours'] = 'A whole number of hours from 1 to 168.';

  if (!intIn(s.downloads.concurrency, 1, 4)) e['downloads.concurrency'] = 'From 1 to 4.';

  return e;
}

const KNOWN_FIELDS = [
  'general.instanceName',
  'general.logLevel',
  'media.folderTemplate',
  'media.noSeriesFolderTemplate',
  'media.format',
  'media.writeSidecars',
  'sync.intervalHours',
  'sync.autoDownloadNew',
  'backfill.enabled',
  'backfill.order',
  'backfill.windowStart',
  'backfill.windowEnd',
  'audible.dailyLicenceCap',
  'audible.spacingMinMinutes',
  'audible.spacingMaxMinutes',
  'audible.throttleCooldownHours',
  'downloads.concurrency',
];

/**
 * API.md says a `400 invalid_input` "names the field" without saying where. Accept an
 * explicit `field` property, else look for a known field path in the message.
 */
export function fieldFromServerError(field: string | null, message: string): string | null {
  if (field) return field;
  return KNOWN_FIELDS.find((f) => message.includes(f)) ?? KNOWN_FIELDS.find((f) => message.includes(f.split('.')[1] ?? f)) ?? null;
}
