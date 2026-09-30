import type { ITheme } from "@xterm/xterm";

export interface TerminalThemeDef {
	name: string;
	theme: ITheme;
	index?: number;
}
