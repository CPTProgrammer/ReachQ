/**
 * Xterm.js terminal theme definitions.
 * Themes are auto-discovered from ./themes/*.ts via Vite's import.meta.glob.
 * Add a new theme by dropping a .ts file in the themes/ directory.
 *
 * Each theme file must export a named `theme` of type TerminalThemeDef,
 * e.g.: `export const theme: TerminalThemeDef = { name: 'My Theme', theme: {...} };`
 */

export interface ITheme {
	/** The default foreground color */
	foreground?: string;
	/** The default background color */
	background?: string;
	/** The cursor color */
	cursor?: string;
	/** The accent color of the cursor (fg color for a block cursor) */
	cursorAccent?: string;
	/** The selection background color (can be transparent) */
	selectionBackground?: string;
	/** The selection foreground color */
	selectionForeground?: string;
	/**
	 * The selection background color when the terminal does not have focus (can
	 * be transparent)
	 */
	selectionInactiveBackground?: string;
	/** ANSI black (eg. `\x1b[30m`) */
	black?: string;
	/** ANSI red (eg. `\x1b[31m`) */
	red?: string;
	/** ANSI green (eg. `\x1b[32m`) */
	green?: string;
	/** ANSI yellow (eg. `\x1b[33m`) */
	yellow?: string;
	/** ANSI blue (eg. `\x1b[34m`) */
	blue?: string;
	/** ANSI magenta (eg. `\x1b[35m`) */
	magenta?: string;
	/** ANSI cyan (eg. `\x1b[36m`) */
	cyan?: string;
	/** ANSI white (eg. `\x1b[37m`) */
	white?: string;
	/** ANSI bright black (eg. `\x1b[1;30m`) */
	brightBlack?: string;
	/** ANSI bright red (eg. `\x1b[1;31m`) */
	brightRed?: string;
	/** ANSI bright green (eg. `\x1b[1;32m`) */
	brightGreen?: string;
	/** ANSI bright yellow (eg. `\x1b[1;33m`) */
	brightYellow?: string;
	/** ANSI bright blue (eg. `\x1b[1;34m`) */
	brightBlue?: string;
	/** ANSI bright magenta (eg. `\x1b[1;35m`) */
	brightMagenta?: string;
	/** ANSI bright cyan (eg. `\x1b[1;36m`) */
	brightCyan?: string;
	/** ANSI bright white (eg. `\x1b[1;37m`) */
	brightWhite?: string;
	/** ANSI extended colors (16-255) */
	extendedAnsi?: string[];
}

export interface TerminalThemeDef {
	name: string;
	theme: ITheme;
}

export { DEFAULT_FOREGROUND, DEFAULT_BACKGROUND, DEFAULT_CURSOR, DEFAULT_CURSOR_ACCENT, DEFAULT_SELECTION } from './theme-constants';

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

const DEFAULT_THEME: ITheme = {
	background: '#0a0a0a',
	foreground: '#f5f5f7',
	cursor: '#0a84ff',
	cursorAccent: '#0a0a0a',
	selectionBackground: 'rgba(10, 132, 255, 0.3)',
	selectionForeground: '#f5f5f7',
	black: '#1d1f21',
	red: '#ff453a',
	green: '#30d158',
	yellow: '#ffd60a',
	blue: '#0a84ff',
	magenta: '#bf5af2',
	cyan: '#64d2ff',
	white: '#f5f5f7',
	brightBlack: '#6e6e73',
	brightRed: '#ff6961',
	brightGreen: '#4ae06a',
	brightYellow: '#ffe566',
	brightBlue: '#409cff',
	brightMagenta: '#da8fff',
	brightCyan: '#8be8ff',
	brightWhite: '#ffffff',
};

// Auto-discover all theme files in the themes/ directory at build time.
// Each file must export: `export const theme: TerminalThemeDef = {...};`
const themeModules = import.meta.glob<{ theme: TerminalThemeDef }>(
	'./themes/*.ts',
	{ eager: true }
);

/** All available terminal themes, sorted alphabetically by filename. */
export const TERMINAL_THEMES: TerminalThemeDef[] = Object.entries(themeModules)
	.map(([, mod]) => mod.theme)
	.sort((a, b) => a.name.localeCompare(b.name));

/** Look up a terminal theme by name. Returns the Default theme if not found. */
export function getTerminalTheme(name: string): ITheme {
	const found = TERMINAL_THEMES.find((t) => t.name === name);
	return found?.theme ?? DEFAULT_THEME;
}
