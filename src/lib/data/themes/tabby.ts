import type { TerminalThemeDef } from '../terminal-themes';
import { DEFAULT_BACKGROUND, DEFAULT_CURSOR, DEFAULT_SELECTION } from '../theme-constants';

export const theme: TerminalThemeDef = {
	name: 'Tabby',
	theme: {
		background: '#171717',
		foreground: '#cacaca',
		cursor: DEFAULT_CURSOR,
		cursorAccent: DEFAULT_BACKGROUND,
		selectionBackground: DEFAULT_SELECTION,
		selectionForeground: undefined,
		black: '#000000',
		red: '#ff615a',
		green: '#b1e969',
		yellow: '#ebd99c',
		blue: '#5da9f6',
		magenta: '#e86aff',
		cyan: '#82fff7',
		white: '#dedacf',
		brightBlack: '#313131',
		brightRed: '#f58c80',
		brightGreen: '#ddf88f',
		brightYellow: '#eee5b2',
		brightBlue: '#a5c7ff',
		brightMagenta: '#ddaaff',
		brightCyan: '#b7fff9',
		brightWhite: '#ffffff',
	},
};
