// Client-side rendering of the output folder template, for the live preview in
// Settings → Media Management. Mirrors the rules in server/DESIGN.md ("Output layout");
// the server is authoritative, this only has to look right.

export type TemplateBook = {
  id: string;
  provider: string;
  title: string;
  subtitle: string | null;
  authors: string[];
  narrators: string[];
  series: string | null;
  seriesSequence: string | null;
  releaseDate: string | null;
};

export const TEMPLATE_TOKENS: { token: string; description: string }[] = [
  { token: '{author}', description: 'First author' },
  { token: '{authors}', description: 'All authors' },
  { token: '{title}', description: 'Title' },
  { token: '{subtitle}', description: 'Subtitle' },
  { token: '{series}', description: 'Series name' },
  { token: '{seriesSequence}', description: 'Position in series' },
  { token: '{year}', description: 'Release year' },
  { token: '{narrator}', description: 'First narrator' },
  { token: '{asin}', description: 'Store product id' },
  { token: '{provider}', description: 'Store (audible, librofm, chirp)' },
];

const KNOWN = new Set(TEMPLATE_TOKENS.map((t) => t.token.slice(1, -1)));
const FORBIDDEN = /[/\\:*?"<>|]/g;
const MAX_COMPONENT_BYTES = 120;

function tokenValues(book: TemplateBook): Record<string, string> {
  return {
    author: book.authors[0] ?? 'Unknown Author',
    authors: book.authors.length ? book.authors.join(', ') : 'Unknown Author',
    title: book.title,
    subtitle: book.subtitle ?? '',
    series: book.series ?? '',
    seriesSequence: book.seriesSequence ?? '',
    year: book.releaseDate ? book.releaseDate.slice(0, 4) : '',
    narrator: book.narrators[0] ?? '',
    asin: book.id,
    provider: book.provider,
  };
}

function truncateBytes(s: string, max: number): string {
  const enc = new TextEncoder();
  if (enc.encode(s).length <= max) return s;
  let out = '';
  for (const ch of s) {
    if (enc.encode(out + ch).length > max) break;
    out += ch;
  }
  return out;
}

function sanitizeComponent(s: string): string {
  const cleaned = s.replace(FORBIDDEN, '').replace(/\s+/g, ' ').trim();
  return truncateBytes(cleaned, MAX_COMPONENT_BYTES).replace(/[. ]+$/, '');
}

export type TemplateResult = {
  path: string;
  unknownTokens: string[];
};

/** Renders a template like "{author}/{series}/{seriesSequence} - {title}" for one book. */
export function renderFolderTemplate(template: string, book: TemplateBook): TemplateResult {
  const values = tokenValues(book);
  const unknownTokens: string[] = [];
  const components = template.split('/').map((part) => {
    const filled = part.replace(/\{([^}]*)\}/g, (match, name: string) => {
      if (!KNOWN.has(name)) {
        unknownTokens.push(match);
        return match;
      }
      // Token values may themselves contain '/', which must not create a folder.
      return (values[name] ?? '').replace(FORBIDDEN, '');
    });
    return sanitizeComponent(filled.replace(/^[\s-]+|[\s-]+$/g, ''));
  });
  return {
    path: components.filter(Boolean).join('/'),
    unknownTokens: [...new Set(unknownTokens)],
  };
}

/** The file name(s) the pipeline writes inside the folder. */
export function outputFileExample(book: TemplateBook, format: 'm4b' | 'mp3'): string {
  if (format === 'm4b') return `${sanitizeComponent(book.title)}.m4b`;
  return '01 - Chapter One.mp3, 02 - Chapter Two.mp3, …';
}

/**
 * Example books for when the library is empty, so the preview still says something.
 * These are illustrative only and are labelled as examples in the UI.
 */
export const EXAMPLE_SERIES_BOOK: TemplateBook = {
  id: 'B000000000',
  provider: 'audible',
  title: 'The Example Voyage',
  subtitle: null,
  authors: ['Jane Author'],
  narrators: ['Sam Narrator'],
  series: 'The Example Saga',
  seriesSequence: '1',
  releaseDate: '2021-05-04',
};

export const EXAMPLE_STANDALONE_BOOK: TemplateBook = {
  ...EXAMPLE_SERIES_BOOK,
  title: 'A Standalone Story',
  series: null,
  seriesSequence: null,
};
