import { BookOpen, Headphones } from 'lucide-react';
import type { ProviderId } from '../api/types';
import styles from './ProviderIcon.module.css';

export const PROVIDER_NAMES: Record<ProviderId, string> = {
  audible: 'Audible',
  librofm: 'Libro.fm',
  chirp: 'Chirp',
};

export function providerName(id: string): string {
  return (PROVIDER_NAMES as Record<string, string>)[id] ?? id;
}

/**
 * The store's mark in a tinted square. Generic glyphs, not brand logos: headphones for
 * Audible, an open book for Libro.fm (as in the app), and an italic serif "C" for Chirp.
 */
export function ProviderIcon({
  provider,
  size = 'md',
  withLabel = false,
  decorative = false,
}: {
  provider: ProviderId;
  size?: 'sm' | 'md' | 'lg';
  withLabel?: boolean;
  /** Next to text that already names the store: hidden from assistive tech. */
  decorative?: boolean;
}) {
  const px = size === 'sm' ? 12 : size === 'lg' ? 22 : 16;
  const glyph =
    provider === 'audible' ? (
      <Headphones size={px} aria-hidden />
    ) : provider === 'librofm' ? (
      <BookOpen size={px} aria-hidden />
    ) : (
      <span className={styles.letter} aria-hidden>
        C
      </span>
    );
  return (
    <span className={styles.wrap} aria-hidden={decorative || undefined}>
      <span className={`${styles.icon} ${styles[size]} ${styles[provider]}`} title={withLabel || decorative ? undefined : PROVIDER_NAMES[provider]}>
        {glyph}
        {!withLabel && !decorative && <span className="visually-hidden">{PROVIDER_NAMES[provider]}</span>}
      </span>
      {withLabel && <span className={styles.label}>{PROVIDER_NAMES[provider]}</span>}
    </span>
  );
}
