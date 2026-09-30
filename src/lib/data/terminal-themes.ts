/**
 * Xterm.js terminal theme definitions.
 * Themes are auto-discovered from ./themes/*.ts via Vite's import.meta.glob.
 * Add a new theme by dropping a .ts file in the themes/ directory.
 *
 * Each theme file must export a named `theme` of type TerminalThemeDef,
 * e.g.: `export const theme: TerminalThemeDef = { name: 'My Theme', theme: {...} };`
 */

import type { ITheme } from '@xterm/xterm';
import type { TerminalThemeDef } from './terminal-themes-type';
import { theme } from './themes/default';

export { DEFAULT_FOREGROUND, DEFAULT_BACKGROUND, DEFAULT_CURSOR, DEFAULT_CURSOR_ACCENT, TABBY_DEFAULT_SELECTION as DEFAULT_SELECTION } from './theme-constants';

/** ANSI color labels in display order */
export const ANSI_COLORS = [
	{ key: 'black', label: 'Black' },
	{ key: 'red', label: 'Red' },
	{ key: 'green', label: 'Green' },
	{ key: 'yellow', label: 'Yellow' },
	{ key: 'blue', label: 'Blue' },
	{ key: 'magenta', label: 'Magenta' },
	{ key: 'cyan', label: 'Cyan' },
	{ key: 'white', label: 'White' },
	{ key: 'brightBlack', label: 'Bright Black' },
	{ key: 'brightRed', label: 'Bright Red' },
	{ key: 'brightGreen', label: 'Bright Green' },
	{ key: 'brightYellow', label: 'Bright Yellow' },
	{ key: 'brightBlue', label: 'Bright Blue' },
	{ key: 'brightMagenta', label: 'Bright Magenta' },
	{ key: 'brightCyan', label: 'Bright Cyan' },
	{ key: 'brightWhite', label: 'Bright White' },
] as const;

const DEFAULT_THEME: TerminalThemeDef = theme;

// Auto-discover all theme files in the themes/ directory at build time.
// Each file must export: `export const theme: TerminalThemeDef = {...};`
const themeModules = import.meta.glob<{ theme: TerminalThemeDef }>(
	'./themes/*.ts',
	{ eager: true }
);

/** All available terminal themes, sorted alphabetically by filename. */
export const TERMINAL_THEMES: TerminalThemeDef[] = Object.entries(themeModules)
	.map(([, mod]) => mod.theme)
	.sort((a, b) => (a.index ?? 0) - (b.index ?? 0) || a.name.localeCompare(b.name));

/** Look up a terminal theme by name. Returns the Default theme if not found. */
export function getTerminalTheme(name: string): ITheme {
	const found = TERMINAL_THEMES.find((t) => t.name === name);
	return (found ?? DEFAULT_THEME).theme;
}
