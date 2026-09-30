import type { TerminalThemeDef } from '../terminal-themes-type';
import { DEFAULT_CURSOR, DEFAULT_LIGHT_SELECTION } from '../theme-constants';

export const theme: TerminalThemeDef = {
	name: 'Default Light',
	theme: {
		background: '#ffffff',
		foreground: '#1d1d1f',
		cursor: DEFAULT_CURSOR,
		cursorAccent: '#ffffff',
		selectionBackground: DEFAULT_LIGHT_SELECTION,
		selectionForeground: '#1d1d1f',
		black: '#e5e5ea',
		red: '#c73a2e',
		green: '#248a3d',
		yellow: '#b27a00',
		blue: '#0a64d1',
		magenta: '#a040c4',
		cyan: '#007580',
		white: '#1d1d1f',
		brightBlack: '#8e8e93',
		brightRed: '#e04e3f',
		brightGreen: '#2ea043',
		brightYellow: '#d9a300',
		brightBlue: '#409cff',
		brightMagenta: '#c16bde',
		brightCyan: '#0096a0',
		brightWhite: '#3a3a3c',
	},
};
