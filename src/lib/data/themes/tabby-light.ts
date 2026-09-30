import type { TerminalThemeDef } from '../terminal-themes-type';
import { TABBY_DEFAULT_SELECTION } from '../theme-constants';

export const theme: TerminalThemeDef = {
	name: 'Tabby Light',
	theme: {
		background: '#ffffff',
		foreground: '#4d4d4c',
		cursor: '#4d4d4c',
		cursorAccent: '#ffffff',
		selectionBackground: TABBY_DEFAULT_SELECTION,
		selectionForeground: undefined,
		black: '#000000',
		red: '#c82829',
		green: '#718c00',
		yellow: '#eab700',
		blue: '#4271ae',
		magenta: '#8959a8',
		cyan: '#3e999f',
		white: '#ffffff',
		brightBlack: '#000000',
		brightRed: '#c82829',
		brightGreen: '#718c00',
		brightYellow: '#eab700',
		brightBlue: '#4271ae',
		brightMagenta: '#8959a8',
		brightCyan: '#3e999f',
		brightWhite: '#ffffff',
	},
};
