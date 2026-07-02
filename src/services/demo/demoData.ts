/**
 * Demo Mode dataset (Audible-shaped, LibriVox-sourced)
 *
 * Provides a fake Audible account and a curated set of free public-domain
 * LibriVox audiobooks, shaped exactly like the `Book` objects the rest of the
 * Audible UI consumes. Demo books are tagged `source: 'audible'` so the UI shows
 * zero LibriVox branding — the whole point of demo mode is to replicate the real
 * Audible experience without an Amazon login.
 *
 * Metadata below was fetched once from the public LibriVox API and baked in, so
 * browsing/filtering/sorting work fully offline. Only cover images and the actual
 * audio download require a network connection.
 *
 * Nothing here is ever written to SQLite — demo books live in-memory and are
 * served by the demo-aware bridge facade (see ./bridge.ts and ./demoMode.ts).
 */

import type { Account, Book, Locale } from '../../../modules/expo-rust-bridge';

export const DEMO_ACCOUNT_ID = 'demo:us';
/** asin prefix used to mark every demo book (e.g. `demo_56`). */
export const DEMO_ASIN_PREFIX = 'demo_';

const DEMO_LOCALE: Locale = {
  country_code: 'us',
  name: 'Demo Library',
  domain: 'audible.com',
  with_username: true,
};

/**
 * Fake Audible account. Tokens are placeholders and the expiry is far in the
 * future so the token-refresh paths are never triggered. It is never persisted.
 */
export const DEMO_ACCOUNT: Account = {
  account_id: DEMO_ACCOUNT_ID,
  account_name: 'Demo Library (US)',
  library_scan: true,
  decrypt_key: '',
  locale: DEMO_LOCALE,
  identity: {
    access_token: {
      token: 'demo-access-token',
      expires_at: '2099-01-01T00:00:00.000Z',
    },
    refresh_token: 'demo-refresh-token',
    device_private_key: '',
    adp_token: '',
    cookies: {},
    device_serial_number: 'DEMO0000000000000000000000000000',
    device_type: 'DEMO',
    device_name: 'LibriSync Demo',
    amazon_account_id: 'demo-user',
    store_authentication_cookie: '',
    locale: DEMO_LOCALE,
    customer_info: {
      account_pool: 'DEMO',
      user_id: 'demo-user',
      home_region: 'NA',
      name: 'Demo Library',
    },
  },
};

/** Extended Book carrying demo-only fields used for filtering and downloading. */
export interface DemoBook extends Book {
  /** Genre used by the category filter (real schema stores this in joined tables). */
  category: string;
  /** Real LibriVox project id, used at download time to resolve chapter MP3s. */
  librivoxId: string;
  /** Whole-book zip fallback URL. */
  zipUrl: string;
}

interface RawDemoBook {
  librivoxId: string;
  title: string;
  authors: string[];
  narrators: string[];
  durationSeconds: number;
  coverSlug: string;
  category: string;
  description: string;
  zipUrl: string;
  seriesName?: string;
  seriesSequence?: number;
}

// Verified LibriVox metadata (ids, durations, cover slugs, zip URLs are real).
const RAW_DEMO_BOOKS: RawDemoBook[] = [
  { librivoxId: '56', title: 'Secret Garden', authors: ['Frances Hodgson Burnett'], narrators: ['LibriVox Volunteers'], durationSeconds: 32905, coverSlug: 'the-secret-garden-by-frances-hodgson-burnett', category: "Children's", description: 'Mary Lennox is a spoiled, middle-class, self-centered child who has been recently orphaned. She is accepted into the quiet and remote country house of an uncle, who has almost completely withdrawn into himself after the death of his wife.', zipUrl: 'https://archive.org/compress/secret_garden_librivox/formats=64KBPS MP3&file=/secret_garden_librivox.zip' },
  { librivoxId: '59', title: 'Adventures of Huckleberry Finn', authors: ['Mark Twain'], narrators: ['LibriVox Volunteers'], durationSeconds: 38527, coverSlug: 'the-adventures-of-huckleberry-finn-by-mark-twain', category: 'Adventure', description: 'Adventures of Huckleberry Finn (1884) by Mark Twain is one of the truly great American novels, beloved by children, adults, and literary critics alike.', zipUrl: 'https://archive.org/compress/huck_finn_librivox/formats=64KBPS MP3&file=/huck_finn_librivox.zip' },
  { librivoxId: '64', title: 'Heart of Darkness', authors: ['Joseph Conrad'], narrators: ['LibriVox Volunteers'], durationSeconds: 15012, coverSlug: 'heart-of-darkness-by-joseph-conrad', category: 'Classics', description: 'Set in a time of oppressive colonisation, Heart of Darkness famously explores the rituals of civilisation and barbarism.', zipUrl: 'https://archive.org/compress/heart_of_darkness/formats=64KBPS MP3&file=/heart_of_darkness.zip', seriesName: 'Joseph Conrad Collection', seriesSequence: 1 },
  { librivoxId: '65', title: 'Odyssey', authors: ['Homer'], narrators: ['LibriVox Volunteers'], durationSeconds: 40705, coverSlug: 'the-odyssey-by-homer', category: 'Classics', description: 'The Odyssey is one of the two major ancient Greek epic poems attributed to the poet Homer, concerning the events that befall the Greek hero Odysseus on his journey home.', zipUrl: 'https://archive.org/compress/odyssey_butler_librivox/formats=64KBPS MP3&file=/odyssey_butler_librivox.zip' },
  { librivoxId: '71', title: 'Canterville Ghost', authors: ['Oscar Wilde'], narrators: ['LibriVox Volunteers'], durationSeconds: 4988, coverSlug: 'the-canterville-ghost-by-oscar-wilde', category: 'Horror', description: 'The American Minister and his family have bought the English stately home Canterville Chase, complete with the ghost of Sir Simon de Canterville.', zipUrl: 'https://archive.org/compress/canterville_ghost_librivox/formats=64KBPS MP3&file=/canterville_ghost_librivox.zip', seriesName: 'Oscar Wilde Collection', seriesSequence: 1 },
  { librivoxId: '74', title: 'Mother Goose in Prose', authors: ['L. Frank Baum'], narrators: ['LibriVox Volunteers'], durationSeconds: 17184, coverSlug: 'mother-goose-in-prose-by-l-frank-baum', category: "Children's", description: 'The songs attributed to Mother Goose are what we remember from our childhood. Some of these nursery rhymes are complete tales in themselves.', zipUrl: 'https://archive.org/compress/mother_goose_prose_librivox/formats=64KBPS MP3&file=/mother_goose_prose_librivox.zip' },
  { librivoxId: '75', title: "Uncle Tom's Cabin", authors: ['Harriet Beecher Stowe'], narrators: ['LibriVox Volunteers'], durationSeconds: 65193, coverSlug: 'uncle-toms-cabin-by-harriet-beecher-stowe', category: 'Classics', description: "Uncle Tom's Cabin; or, Life Among the Lowly is a novel by American author Harriet Beecher Stowe which treats slavery as a central theme.", zipUrl: 'https://archive.org/compress/uncle_toms_cabin_librivox/formats=64KBPS MP3&file=/uncle_toms_cabin_librivox.zip' },
  { librivoxId: '81', title: 'Dream Psychology', authors: ['Sigmund Freud'], narrators: ['LibriVox Volunteers'], durationSeconds: 21842, coverSlug: 'dream-psychology-by-sigmund-freud', category: 'Non-Fiction', description: "Freud's interpretation of dreams, offered to the world in a book as circumstantial as a legal record to be pondered over by scientists.", zipUrl: 'https://archive.org/compress/dream_psychology_librivox/formats=64KBPS MP3&file=/dream_psychology_librivox.zip' },
  { librivoxId: '86', title: 'Emma', authors: ['Jane Austen'], narrators: ['LibriVox Volunteers'], durationSeconds: 64950, coverSlug: 'emma-by-jane-austen-solo', category: 'Romance', description: "Jane Austen's sparkling comedy of manners. Emma blithely manipulates and misunderstands her friends and family until she finally grows up.", zipUrl: 'https://archive.org/compress/emma_solo_librivox/formats=64KBPS MP3&file=/emma_solo_librivox.zip' },
  { librivoxId: '90', title: 'Importance of Being Earnest', authors: ['Oscar Wilde'], narrators: ['LibriVox Volunteers'], durationSeconds: 8280, coverSlug: 'the-importance-of-being-earnest-by-oscar-wilde', category: 'Comedy', description: 'A classic comedy of manners in which two flippant young men pretend that their names are "Ernest" to impress their beloveds.', zipUrl: 'https://archive.org/compress/being_earnest_librivox/formats=64KBPS MP3&file=/being_earnest_librivox.zip', seriesName: 'Oscar Wilde Collection', seriesSequence: 2 },
  { librivoxId: '94', title: 'Wind in the Willows', authors: ['Kenneth Grahame'], narrators: ['LibriVox Volunteers'], durationSeconds: 23407, coverSlug: 'the-wind-in-the-willows-by-kenneth-grahame-2', category: "Children's", description: 'This much-loved story follows a group of animal friends — Mole, Rat, and Toad — in the English countryside as they pursue adventure.', zipUrl: 'https://archive.org/compress/wind_in_the_willows_collab_librivox/formats=64KBPS MP3&file=/wind_in_the_willows_collab_librivox.zip' },
  { librivoxId: '97', title: 'Lord Jim', authors: ['Joseph Conrad'], narrators: ['LibriVox Volunteers'], durationSeconds: 51917, coverSlug: 'lord-jim-by-joseph-conrad', category: 'Adventure', description: 'A classic of early literary modernism, Lord Jim tells the story of a young man who loses his honor in a display of cowardice at sea.', zipUrl: 'https://archive.org/compress/lord_jim_librivox/formats=64KBPS MP3&file=/lord_jim_librivox.zip', seriesName: 'Joseph Conrad Collection', seriesSequence: 2 },
  { librivoxId: '100', title: 'Narrative of Arthur Gordon Pym of Nantucket', authors: ['Edgar Allan Poe'], narrators: ['LibriVox Volunteers'], durationSeconds: 22932, coverSlug: 'narrative-of-arthur-gordon-pym', category: 'Adventure', description: "Edgar Allan Poe's only complete novel relates the tale of young Arthur Gordon Pym who stows away aboard a whaling ship called Grampus.", zipUrl: 'https://archive.org/compress/narrative_gordon_pym_librivox/formats=64KBPS MP3&file=/narrative_gordon_pym_librivox.zip' },
  { librivoxId: '104', title: 'Merry Adventures of Robin Hood', authors: ['Howard Pyle'], narrators: ['LibriVox Volunteers'], durationSeconds: 39649, coverSlug: 'the-merry-adventures-of-robin-hood-by-howard-pyle', category: 'Adventure', description: 'Robin Hood is the archetypal English folk hero — a courteous, swashbuckling outlaw famous for robbing the rich to feed the poor.', zipUrl: 'https://archive.org/compress/merry_adventures_robin_hood_librivox/formats=64KBPS MP3&file=/merry_adventures_robin_hood_librivox.zip' },
  { librivoxId: '119', title: 'Art of War', authors: ['Sun Tzu'], narrators: ['LibriVox Volunteers'], durationSeconds: 4334, coverSlug: 'the-art-of-war-by-sun-tzu', category: 'Non-Fiction', description: 'The Art of War is a Chinese military treatise written during the 6th century BC by Sun Tzu, long praised as the definitive work on military strategy.', zipUrl: 'https://archive.org/compress/art_of_war_librivox/formats=64KBPS MP3&file=/art_of_war_librivox.zip' },
  { librivoxId: '67', title: 'Divine Comedy', authors: ['Dante Alighieri'], narrators: ['LibriVox Volunteers'], durationSeconds: 44605, coverSlug: 'the-divine-comedy-by-dante-alighieri', category: 'Poetry', description: 'Written by Dante Alighieri between 1308 and his death in 1321, widely considered the central epic poem of Italian literature.', zipUrl: 'https://archive.org/compress/divine_comedy_librivox/formats=64KBPS MP3&file=/divine_comedy_librivox.zip' },
  { librivoxId: '55', title: 'This Side of Paradise', authors: ['F. Scott Fitzgerald'], narrators: ['LibriVox Volunteers'], durationSeconds: 32184, coverSlug: 'this-side-of-paradise-by-f-scott-fitzgerald', category: 'Classics', description: 'The debut novel of F. Scott Fitzgerald, published in 1920, examines the lives and morality of post-World War I youth.', zipUrl: 'https://archive.org/compress/this_side_paradise_librivox/formats=64KBPS MP3&file=/this_side_paradise_librivox.zip' },
  { librivoxId: '47', title: 'Count of Monte Cristo', authors: ['Alexandre Dumas'], narrators: ['LibriVox Volunteers'], durationSeconds: 178995, coverSlug: 'the-count-of-monte-cristo-by-alexandre-dumas', category: 'Adventure', description: "An adventure novel by Alexandre Dumas, often considered along with The Three Musketeers as Dumas's most popular work.", zipUrl: 'https://archive.org/compress/count_monte_cristo_0711_librivox/formats=64KBPS MP3&file=/count_monte_cristo_0711_librivox.zip' },
];

function coverUrlFromSlug(slug: string): string {
  return `https://archive.org/services/img/${slug}`;
}

/** Deterministic ISO timestamp so date-based sorting is stable across launches. */
function demoTimestamp(index: number): string {
  const day = String((index % 28) + 1).padStart(2, '0');
  const month = String((index % 12) + 1).padStart(2, '0');
  return `2024-${month}-${day}T12:00:00.000Z`;
}

export const DEMO_BOOKS: DemoBook[] = RAW_DEMO_BOOKS.map((raw, index) => {
  const timestamp = demoTimestamp(index);
  return {
    id: Number(raw.librivoxId),
    audible_product_id: `${DEMO_ASIN_PREFIX}${raw.librivoxId}`,
    title: raw.title,
    authors: raw.authors,
    narrators: raw.narrators,
    series_name: raw.seriesName,
    series_sequence: raw.seriesSequence,
    description: raw.description,
    publisher: 'LibriVox',
    release_date: timestamp,
    duration_seconds: raw.durationSeconds,
    language: 'english',
    cover_url: coverUrlFromSlug(raw.coverSlug),
    created_at: timestamp,
    updated_at: timestamp,
    source: 'audible',
    account: DEMO_ACCOUNT_ID,
    is_downloadable: true,
    // demo-only fields
    category: raw.category,
    librivoxId: raw.librivoxId,
    zipUrl: raw.zipUrl,
  };
});

export const DEMO_SERIES: string[] = Array.from(
  new Set(DEMO_BOOKS.map(b => b.series_name).filter((s): s is string => !!s))
).sort();

export const DEMO_CATEGORIES: string[] = Array.from(
  new Set(DEMO_BOOKS.map(b => b.category))
).sort();
