/**
 * Diagnostics you can get out of a user's phone.
 *
 * Audible's library response carries no total (verified against captured responses:
 * `items` and `response_groups`, nothing else), so a sync that stops early looks
 * identical to one that finished — the counts agree with themselves. The page trail
 * recorded here is the only evidence of where a sync stopped and why, which makes it
 * the difference between "some books are missing" and an actionable bug report.
 *
 * Reports are held in memory and mirrored to a file so they survive a crash or restart,
 * and are rendered as plain text for the clipboard.
 */
import { File, Paths } from 'expo-file-system';
import Constants from 'expo-constants';
import { Platform } from 'react-native';
import type { SyncStats } from '../../modules/expo-rust-bridge';

const REPORT_FILE_NAME = 'last-sync-report.txt';
/** Keep the clipboard payload sane: the head and tail of a long error list is enough. */
const MAX_ERRORS_IN_REPORT = 40;

let lastReport: string | null = null;

function appVersion(): string {
  // A local build without APP_VERSION set reports app.config.js's "0.0.4" fallback, so
  // say plainly when a report came from a dev build rather than a released one.
  const version = Constants.expoConfig?.version ?? 'unknown';
  const build = __DEV__ ? ', dev build' : '';
  return `LibriSync ${version} (${Platform.OS} ${Platform.Version}${build})`;
}

function reportFile(): File {
  return new File(Paths.document, REPORT_FILE_NAME);
}

/**
 * Render a finished sync as copyable text. `label` names what was synced (an account
 * or provider), so a multi-account report stays readable.
 */
export function formatSyncReport(label: string, stats: SyncStats): string {
  const lines: string[] = [];

  lines.push('=== LibriSync sync report ===');
  lines.push(appVersion());
  lines.push(`When: ${new Date().toISOString()}`);
  lines.push(`Account: ${label}`);
  lines.push('');
  lines.push(`Items fetched:  ${stats.total_items}`);
  lines.push(`Books added:    ${stats.books_added}`);
  lines.push(`Books updated:  ${stats.books_updated}`);
  lines.push(`Unreadable:     ${stats.items_failed}`);
  lines.push(`Marked absent:  ${stats.books_absent}`);

  const pages = stats.pages ?? [];
  if (pages.length > 0) {
    const skipped = pages.filter((p) => p.error);
    lines.push(`Pages fetched:  ${pages.length}${skipped.length ? ` (${skipped.length} skipped)` : ''}`);
    lines.push('');
    lines.push('Page  Items  Added  Updated  Failed  Tries  Time');
    for (const page of pages) {
      const row =
        String(page.page).padEnd(6) +
        String(page.items).padEnd(7) +
        String(page.added).padEnd(7) +
        String(page.updated).padEnd(9) +
        String(page.failedItems).padEnd(8) +
        String(page.attempts).padEnd(7) +
        `${Math.round(page.durationMs)}ms`;
      lines.push(page.error ? `${row}  SKIPPED: ${page.error}` : row);
    }

    // The shape of the page trail is the diagnosis: a short page followed by the end of
    // pagination is what a truncated library looks like from the inside.
    const last = pages[pages.length - 1];
    const full = pages.filter((p) => !p.error && p.items > 0);
    const pageSize = full.length > 0 ? Math.max(...full.map((p) => p.items)) : 0;
    if (last && !last.error && last.items > 0 && pageSize > 0 && last.items === pageSize) {
      lines.push('');
      lines.push(
        'NOTE: the last page was full, so pagination may have stopped early rather than ' +
        'at the end of the library.'
      );
    }
  }

  if (stats.errors.length > 0) {
    lines.push('');
    lines.push(`Errors (${stats.errors.length}):`);
    const shown = stats.errors.slice(0, MAX_ERRORS_IN_REPORT);
    shown.forEach((error, index) => lines.push(`${index + 1}. ${error}`));
    if (stats.errors.length > shown.length) {
      lines.push(`... and ${stats.errors.length - shown.length} more`);
    }
  }

  return lines.join('\n');
}

/** True when the sync lost something — the case worth showing the user. */
export function syncHadProblems(stats: SyncStats): boolean {
  return stats.items_failed > 0 || stats.errors.length > 0
    || (stats.pages ?? []).some((page) => !!page.error);
}

/** Remember a report for later retrieval from Settings, and across a restart. */
export function storeSyncReport(report: string): void {
  lastReport = report;
  try {
    const file = reportFile();
    if (file.exists) file.delete();
    file.create();
    file.write(report, { encoding: 'utf8' });
  } catch (error) {
    // Persistence is a convenience; the in-memory copy still serves this session.
    console.warn('[diagnostics] Failed to persist sync report:', error);
  }
}

/** The most recent report, from this session or a previous one. */
export function getLastSyncReport(): string | null {
  if (lastReport !== null) return lastReport;
  try {
    const file = reportFile();
    if (file.exists) {
      lastReport = file.textSync();
      return lastReport;
    }
  } catch (error) {
    console.warn('[diagnostics] Failed to read stored sync report:', error);
  }
  return null;
}

/**
 * Render a crash as copyable text, with the last sync report appended when there is
 * one — a crash during or after a sync is usually about what the sync just hit.
 */
export function formatCrashReport(error: Error, componentStack?: string): string {
  const lines: string[] = [];

  lines.push('=== LibriSync crash report ===');
  lines.push(appVersion());
  lines.push(`When: ${new Date().toISOString()}`);
  lines.push('');
  lines.push(`Error: ${error?.name ?? 'Error'}: ${error?.message ?? String(error)}`);

  if (error?.stack) {
    lines.push('');
    lines.push('Stack:');
    lines.push(error.stack);
  }

  if (componentStack) {
    lines.push('');
    lines.push('Component stack:');
    lines.push(componentStack.trim());
  }

  const sync = getLastSyncReport();
  if (sync) {
    lines.push('');
    lines.push('--- last sync report ---');
    lines.push(sync);
  }

  return lines.join('\n');
}
