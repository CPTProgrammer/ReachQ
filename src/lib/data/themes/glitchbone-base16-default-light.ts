import type { TerminalThemeDef } from '../terminal-themes-type';
import { DEFAULT_LIGHT_SELECTION } from '../theme-constants';

export const theme: TerminalThemeDef = {
	name: 'Glitchbone Base16 Default Light',
	index: 1,
	theme: {
		background: "#F8F8F8",
		foreground: "#383838",
		cursor: "#383838",
		cursorAccent: "#383838",
		selectionBackground: DEFAULT_LIGHT_SELECTION,
		selectionForeground: "#383838",
		black: "#F8F8F8",
		red: "#AB4642",
		green: "#A1B56C",
		yellow: "#F7CA88",
		blue: "#7CAFC2",
		magenta: "#BA8BAF",
		cyan: "#86C1B9",
		white: "#383838",
		brightBlack: "#B8B8B8",
		brightRed: "#AB4642",
		brightGreen: "#A1B56C",
		brightYellow: "#F7CA88",
		brightBlue: "#7CAFC2",
		brightMagenta: "#BA8BAF",
		brightCyan: "#86C1B9",
		brightWhite: "#181818",
	},
};
