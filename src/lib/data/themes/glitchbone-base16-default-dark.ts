import type { TerminalThemeDef } from '../terminal-themes-type';
import { DEFAULT_DARK_SELECTION } from '../theme-constants';

export const theme: TerminalThemeDef = {
	name: 'Glitchbone Base16 Default Dark',
	index: 1,
	theme: {
		background: "#181818",
		foreground: "#D8D8D8",
		cursor: "#D8D8D8",
		cursorAccent: "#D8D8D8",
		selectionBackground: DEFAULT_DARK_SELECTION,
		selectionForeground: "#D8D8D8",
		black: "#181818",
		red: "#AB4642",
		green: "#A1B56C",
		yellow: "#F7CA88",
		blue: "#7CAFC2",
		magenta: "#BA8BAF",
		cyan: "#86C1B9",
		white: "#D8D8D8",
		brightBlack: "#585858",
		brightRed: "#AB4642",
		brightGreen: "#A1B56C",
		brightYellow: "#F7CA88",
		brightBlue: "#7CAFC2",
		brightMagenta: "#BA8BAF",
		brightCyan: "#86C1B9",
		brightWhite: "#F8F8F8",
	},
};
