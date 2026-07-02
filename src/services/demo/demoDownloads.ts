/**
 * In-memory demo download manager.
 *
 * Demo mode performs REAL downloads of free LibriVox chapter MP3s (no DRM), but
 * stays entirely in TypeScript — it does NOT touch the native Rust download
 * manager or SQLite. It exposes `DownloadTask`-shaped state so the existing
 * Audible library UI (progress %, pause/resume/cancel buttons) renders unchanged.
 *
 * Divergences from the native Audible pipeline (intentional, to stay TS-only):
 *  - Files are saved to the app sandbox (`Paths.document/demo/<asin>/`), not the
 *    SAF user directory.
 *  - There are no native rich notifications; in-screen progress is the feedback.
 *  - Progress is tracked at chapter granularity (no byte-level callback exists in
 *    expo-file-system's `downloadFileAsync`).
 *  - Only the first {@link DEMO_MAX_CHAPTERS} chapters are fetched so a demo never
 *    pulls multi-GB books; the task is still marked complete afterwards.
 */

import { Directory, File, Paths } from 'expo-file-system';
import type { Book, DownloadTask } from '../../../modules/expo-rust-bridge';
import { getBookSections, type LibriVoxSection } from '../librivox';
import { librivoxIdFromAsin } from './demoMode';

/** Cap chapters per book so a demo download stays fast and small. */
const DEMO_MAX_CHAPTERS = 5;

interface Control {
  paused: boolean;
  cancelled: boolean;
}

const tasks = new Map<string, DownloadTask>();
const controls = new Map<string, Control>();
const sectionsByAsin = new Map<string, LibriVoxSection[]>();

function nowIso(): string {
  return new Date().toISOString();
}

function demoDir(asin: string): Directory {
  return new Directory(Paths.document, 'demo', asin);
}

function makeTask(book: Book): DownloadTask {
  return {
    task_id: `demo-${book.audible_product_id}`,
    asin: book.audible_product_id,
    title: book.title,
    status: 'queued',
    bytes_downloaded: 0,
    total_bytes: 0,
    download_url: '',
    download_path: '',
    output_path: '',
    request_headers: {},
    retry_count: 0,
    created_at: nowIso(),
  };
}

/** Snapshot of all demo tasks, keyed by asin — merged into the library poll. */
export function getTasks(): Map<string, DownloadTask> {
  return new Map(tasks);
}

export function getTask(asin: string): DownloadTask | undefined {
  return tasks.get(asin);
}

/**
 * Start (or restart) a demo download. Resolves the real LibriVox chapter MP3s
 * and downloads up to {@link DEMO_MAX_CHAPTERS} of them into the app sandbox.
 */
export async function enqueue(book: Book): Promise<void> {
  const asin = book.audible_product_id;

  // Already completed or in flight — ignore duplicate taps.
  const existing = tasks.get(asin);
  if (existing && (existing.status === 'completed' || existing.status === 'downloading' || existing.status === 'queued')) {
    return;
  }

  const task = makeTask(book);
  tasks.set(asin, task);
  controls.set(asin, { paused: false, cancelled: false });

  try {
    const allSections = await getBookSections(librivoxIdFromAsin(asin));
    const sections = allSections
      .filter(s => !!s.listen_url)
      .slice(0, DEMO_MAX_CHAPTERS);

    if (sections.length === 0) {
      task.status = 'failed';
      task.error = 'No downloadable chapters found for this title.';
      return;
    }

    sectionsByAsin.set(asin, sections);
    task.total_bytes = sections.length;

    const dir = demoDir(asin);
    if (!dir.exists) dir.create({ intermediates: true });
    task.output_path = dir.uri;
    task.download_path = dir.uri;

    await runFrom(asin);
  } catch (error: any) {
    task.status = 'failed';
    task.error = error?.message || 'Demo download failed';
  }
}

/** Download remaining chapters from the current progress point. */
async function runFrom(asin: string): Promise<void> {
  const task = tasks.get(asin);
  const control = controls.get(asin);
  const sections = sectionsByAsin.get(asin);
  if (!task || !control || !sections) return;

  task.status = 'downloading';
  task.started_at = task.started_at || nowIso();

  const dir = demoDir(asin);

  for (let i = task.bytes_downloaded; i < sections.length; i++) {
    if (control.cancelled) {
      cleanup(asin);
      task.status = 'cancelled';
      return;
    }
    if (control.paused) {
      task.status = 'paused';
      return;
    }

    const section = sections[i];
    const fileName = section.file_name || `chapter-${i + 1}.mp3`;
    const target = new File(dir, fileName);
    try {
      await File.downloadFileAsync(section.listen_url, target, { idempotent: true });
    } catch (error: any) {
      task.status = 'failed';
      task.error = error?.message || `Failed to download chapter ${i + 1}`;
      return;
    }

    task.bytes_downloaded = i + 1;
  }

  task.status = 'completed';
  task.completed_at = nowIso();
  // `output_path` already points at the downloaded folder; the library UI treats a
  // 'completed' task as downloaded, so no Book.file_path mutation is needed.
}

export function pause(asin: string): void {
  const control = controls.get(asin);
  const task = tasks.get(asin);
  if (control && task && task.status === 'downloading') {
    control.paused = true;
  }
}

export async function resume(asin: string): Promise<void> {
  const control = controls.get(asin);
  const task = tasks.get(asin);
  if (control && task && task.status === 'paused') {
    control.paused = false;
    await runFrom(asin);
  }
}

export function cancel(asin: string): void {
  const control = controls.get(asin);
  const task = tasks.get(asin);
  if (!control || !task) return;
  control.cancelled = true;
  // If not actively looping (queued/paused), tear down immediately.
  if (task.status !== 'downloading') {
    cleanup(asin);
    task.status = 'cancelled';
  }
}

function cleanup(asin: string): void {
  try {
    const dir = demoDir(asin);
    if (dir.exists) dir.delete();
  } catch {
    // best effort
  }
}
