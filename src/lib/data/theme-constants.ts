/**
 * Shared terminal theme constants.
 * Imported by both terminal-themes.ts and individual theme files in ./themes/.
 * Kept separate to avoid circular dependency with import.meta.glob.
 */

export const DEFAULT_FOREGROUND = '#ffffff';
export const DEFAULT_BACKGROUND = '#000000';
export const DEFAULT_CURSOR = '#ffffff';
export const DEFAULT_CURSOR_ACCENT = DEFAULT_BACKGROUND;
export const DEFAULT_SELECTION = 'rgba(255, 255, 255, 0.3)';
