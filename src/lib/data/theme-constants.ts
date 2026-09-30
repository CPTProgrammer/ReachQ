/**
 * Shared terminal theme constants.
 * Imported by both terminal-themes.ts and individual theme files in ./themes/.
 * Kept separate to avoid circular dependency with import.meta.glob.
 */

export const DEFAULT_FOREGROUND = '#ffffff';
export const DEFAULT_BACKGROUND = '#000000';
export const DEFAULT_CURSOR = '#0a84ff';
export const DEFAULT_CURSOR_ACCENT = DEFAULT_BACKGROUND;

export const DEFAULT_DARK_SELECTION = 'rgba(10, 132, 255, 0.3)';
export const DEFAULT_LIGHT_SELECTION = 'rgba(10, 132, 255, 0.2)';

export const TABBY_DEFAULT_SELECTION = '#88888888';
