import React from 'react';
import { Platform, Text, View } from 'react-native';
import { FontAwesome6, Ionicons, MaterialCommunityIcons } from '@expo/vector-icons';
import { useStyles } from '../hooks/useStyles';
import type { Theme } from '../hooks/useStyles';

/**
 * One icon per audiobook source, drawn at any size in any colour — the shape the
 * provider registry hands to every screen, so a provider's icon can come from
 * whichever font (or glyph) suits it.
 */
export interface ProviderIconProps {
  size: number;
  color: string;
}

export type ProviderIcon = React.ComponentType<ProviderIconProps>;

/** Audible's own mark, from Font Awesome's brand set. */
export function AudibleIcon({ size, color }: ProviderIconProps) {
  return <FontAwesome6 name="audible" brand size={size} color={color} />;
}

/** Libro.fm. An open book — LibriVox already has the closed one. */
export function LibroFmIcon({ size, color }: ProviderIconProps) {
  return <MaterialCommunityIcons name="book-open-variant" size={size} color={color} />;
}

export function LibriVoxIcon({ size, color }: ProviderIconProps) {
  return <Ionicons name="book-outline" size={size} color={color} />;
}

/**
 * Chirp: an italic serif "C", after its logo. No icon font has one, so it is a letter
 * in the platform's serif face, sized to sit in the same box as a glyph icon.
 */
export function ChirpIcon({ size, color }: ProviderIconProps) {
  const styles = useStyles(createChirpStyles);
  return (
    <View
      style={[styles.box, { width: size, height: size }]}
      // It's a picture of a letter: without this, rows read out as "C, Chirp".
      importantForAccessibility="no-hide-descendants"
      accessibilityElementsHidden
    >
      <Text
        style={[
          styles.letter,
          {
            color,
            fontSize: size * 1.2,
            // A centred line box doesn't quite centre an italic capital: measured on
            // Android's serif face it sat 4% of `size` right and 3% high.
            transform: [{ translateX: -size * 0.04 }, { translateY: size * 0.03 }],
          },
        ]}
        allowFontScaling={false}
      >
        C
      </Text>
    </View>
  );
}

const createChirpStyles = (_theme: Theme) => ({
  box: {
    alignItems: 'center' as const,
    justifyContent: 'center' as const,
    overflow: 'visible' as const,
  },
  letter: {
    fontFamily: Platform.select({ ios: 'Georgia', default: 'serif' }),
    fontStyle: 'italic' as const,
    fontWeight: '700' as const,
    includeFontPadding: false,
    // Out of flow, so the letter keeps its natural height. In flow, it was measured to
    // fit the icon square — shorter than the face's line box — and the text view
    // clipped the bottom of the C at the square's edge. The square still centres it.
    position: 'absolute' as const,
  },
});
