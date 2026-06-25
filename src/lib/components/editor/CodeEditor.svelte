<script lang="ts">
	import {
		EditorView,
		keymap,
		lineNumbers,
		highlightActiveLineGutter,
		highlightSpecialChars,
		drawSelection,
		highlightActiveLine,
		crosshairCursor,
		dropCursor,
		ViewPlugin,
		Decoration,
		type DecorationSet,
		ViewUpdate
	} from '@codemirror/view';
	import { EditorState, Compartment, EditorSelection } from '@codemirror/state';
	import {
		defaultKeymap,
		history,
		historyKeymap,
		indentMore,
		indentLess
	} from '@codemirror/commands';
	import {
		syntaxHighlighting,
		defaultHighlightStyle,
		indentOnInput,
		indentUnit,
		bracketMatching,
		foldGutter,
		foldKeymap
	} from '@codemirror/language';
	import { oneDark } from '@codemirror/theme-one-dark';
	import {
		closeBrackets,
		closeBracketsKeymap,
		autocompletion,
		completionKeymap
	} from '@codemirror/autocomplete';
	import { searchKeymap, highlightSelectionMatches } from '@codemirror/search';
	import { lintKeymap } from '@codemirror/lint';
	import { LanguageDescription } from '@codemirror/language';
	import { getSettings } from '$lib/state/settings.svelte';

	interface Props {
		content: string;
		language: LanguageDescription | undefined;
		tabSize: number;
		insertSpaces: boolean;
		onchange: (content: string) => void;
		onsave: () => void;
	}

	let { content, language, tabSize, insertSpaces, onchange, onsave }: Props = $props();

	let editorView: EditorView | undefined;
	let currentLanguageName: string | undefined = undefined;
	const languageCompartment = new Compartment();
	const indentCompartment = new Compartment();

	const settings = getSettings();
	const editorFontSize = settings.fontSize ?? 14;
	const editorFontFamily = settings.fontFamily;
	const WHITESPACE_HIGHLIGHT_COLOR = "#FFFFFF";
	const WHITESPACE_HIGHLIGHT_OPACITY = "10%";
	const appleDarkTheme = EditorView.theme(
		{
			'&': {
				backgroundColor: '#1c1c1e',
				height: '100%'
			},
			'.cm-scroller': {
				overflow: 'auto',
				fontFamily: `${editorFontFamily ? `"${editorFontFamily}", ` : ""}'JetBrains Mono', 'SF Mono', 'Cascadia Code', monospace`,
				fontSize: `${editorFontSize}px`,
				lineHeight: '1.6'
			},
			'.cm-gutters': {
				backgroundColor: '#0a0a0a',
				color: '#86868b',
				border: 'none',
				paddingLeft: '.5em',
				paddingRight: '.25em'
			},
			'.cm-activeLineGutter': {
				backgroundColor: 'rgba(255, 255, 255, 0.03)'
			},
			'&.cm-focused .cm-selectionBackground, .cm-selectionBackground': {
				backgroundColor: 'rgba(10, 132, 255, 0.25) !important'
			},
			'.cm-activeLine': {
				backgroundColor: 'rgba(255, 255, 255, 0.03)'
			},
			'.cm-cursor': {
				borderLeftColor: '#0a84ff'
			},
			'.cm-highlightSpace': {
				backgroundImage: `url("data:image/svg+xml,${encodeURIComponent(`<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 10 10'><circle cx='5' cy='5' r='4' fill='${WHITESPACE_HIGHLIGHT_COLOR}'/></svg>`)}")`,
				backgroundPosition: 'center',
				backgroundSize: '.25em',
				backgroundRepeat: 'no-repeat',
				opacity: WHITESPACE_HIGHLIGHT_OPACITY,
			},
			'.cm-highlightTab': {
				background: "none",
				position: 'relative',
			},
			'.cm-highlightTab::before': {
				content: '""',
				backgroundImage: [
					`url(data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 100"><path d="M 5 50 L 15 50" stroke="${WHITESPACE_HIGHLIGHT_COLOR}" stroke-width="8" stroke-linecap="round"/></svg>`)})`,
					`url(data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 25 100"><path d="M -10 50 L 20 50 M 5 35 L 20 50 L 5 65" fill="none" stroke="${WHITESPACE_HIGHLIGHT_COLOR}" stroke-width="8" stroke-linecap="round" stroke-linejoin="round"/></svg>`)})`,
					`url(data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 100" preserveAspectRatio="none"><path d="M 0 50 L 10 50" stroke="${WHITESPACE_HIGHLIGHT_COLOR}" stroke-width="8"/></svg>`)})`,
				].join(","),
				backgroundPosition: [
					"left center",
					"right center",
					"0.1em center",
				].join(","),
				backgroundSize: [
					"auto 100%",
					"auto 100%",
					"calc(100% - 0.2em) 100%",
				].join(","),
				backgroundRepeat: [
					"no-repeat",
					"no-repeat",
					"no-repeat",
				].join(","),
				opacity: WHITESPACE_HIGHLIGHT_OPACITY,
				position: "absolute",
				top: "0",
				left: "0.1em",
				width: "calc(100% - 0.2em)",
				height: "100%",
				pointerEvents: "none",
				maxWidth: ".7em",
			},
		},
		{ dark: true }
	);

	function loadLanguage(desc: LanguageDescription | undefined): void {
		if (!editorView) return;
		if (!desc) {
			editorView.dispatch({ effects: languageCompartment.reconfigure([]) });
			return;
		}
		desc.load().then(support => {
			editorView?.dispatch({ effects: languageCompartment.reconfigure(support) });
		});
	}

	function cjkRectangularSelection() {
		return EditorView.mouseSelectionStyle.of((view, event) => {
			const isRectSelect = (event.altKey && event.shiftKey && event.button === 0) || event.button === 1;
			if (!isRectSelect) return null;

			let startX = event.clientX;
			let startY = event.clientY;

			return {
				update(update) {},
				get(event, _extend, multiple) {
					let left = Math.min(startX, event.clientX);
					let right = Math.max(startX, event.clientX);

					let startPos = view.posAtCoords({ x: startX, y: startY }, false);
					let curPos = view.posAtCoords(
						{ x: event.clientX, y: event.clientY },
						false,
					);
					let startLine = view.state.doc.lineAt(startPos);
					let curLine = view.state.doc.lineAt(curPos);
					let minLine = Math.min(startLine.number, curLine.number);
					let maxLine = Math.max(startLine.number, curLine.number);

					let ranges = [];
					for (let i = minLine; i <= maxLine; i++) {
						let line = view.state.doc.line(i);
						let block = view.lineBlockAt(line.from);
						let midY = view.documentTop + block.top + block.height / 2;

						let from = view.posAtCoords({ x: left, y: midY }, false);
						let to = view.posAtCoords({ x: right, y: midY }, false);

						if (from >= 0 && to >= 0) {
							ranges.push(EditorSelection.range(from, to));
						}
					}

					if (!ranges.length) return view.state.selection;
					if (multiple)
						return EditorSelection.create(
							ranges.concat(view.state.selection.ranges),
						);
					return EditorSelection.create(ranges);
				},
			};
		});
	}

	const visibleWhitespace = ViewPlugin.fromClass(class {
		decorations: DecorationSet;
		constructor(view: EditorView) {
			this.decorations = this.compute(view);
		}
		update(update: ViewUpdate) {
			if (update.selectionSet || update.docChanged || update.viewportChanged) {
				this.decorations = this.compute(update.view);
			}
		}
		compute(view: EditorView) {
			const sel = view.state.selection.main;
			if (sel.empty) return Decoration.none;

			const vp = view.viewport;
			const from = Math.max(sel.from, vp.from);
			const to = Math.min(sel.to, vp.to);
			if (from >= to) return Decoration.none;

			const text = view.state.sliceDoc(from, to);
			const marks: any[] = [];
			for (let i = 0, pos = from; i < text.length; i++, pos++) {
				if (text[i] === ' ') marks.push(Decoration.mark({ class: 'cm-highlightSpace' }).range(pos, pos + 1));
				else if (text[i] === '\t') marks.push(Decoration.mark({ class: 'cm-highlightTab' }).range(pos, pos + 1));
			}
			return Decoration.set(marks, true);
		}
	}, {
		decorations: v => v.decorations,
	});

	function isFullWidthCharacter(charCode: number): boolean {
		return (
			(charCode >= 0x2E80 && charCode <= 0xD7AF)
			|| (charCode >= 0xF900 && charCode <= 0xFAFF)
			|| (charCode >= 0xFF01 && charCode <= 0xFF5E)
			|| (charCode >= 0xFFE0 && charCode <= 0xFFE6)
		);
	}
	function visualColumn(line: string, pos: number, tabSize: number): number {
		let col = 0;
		for (let i = 0; i < pos; i++) {
			const code = line.charCodeAt(i);
			if (code === 9) {
				col += tabSize - (col % tabSize);
			} else if (isFullWidthCharacter(code)) {
				col += 2;
			} else {
				col++;
			}
		}
		return col;
	}
	function spacesToNextTabStop(line: string, pos: number, tabSize: number): string {
		const col = visualColumn(line, pos, tabSize);
		const needed = tabSize - (col % tabSize);
		return ' '.repeat(needed || tabSize);
	}

	function mountEditor(node: HTMLDivElement): { destroy: () => void } {
		const state = EditorState.create({
			doc: content,
			extensions: [
				lineNumbers(),
				highlightActiveLineGutter(),
				highlightSpecialChars(),
				history(),
				foldGutter(),
				drawSelection(),
				dropCursor(),
				indentOnInput(),
				syntaxHighlighting(defaultHighlightStyle, { fallback: true }),
				bracketMatching(),
				closeBrackets(),
				autocompletion(),
				EditorState.allowMultipleSelections.of(true),
				cjkRectangularSelection(),
				// crosshairCursor(),
				highlightActiveLine(),
				highlightSelectionMatches(),
				appleDarkTheme,
				oneDark,
				languageCompartment.of([]),
				indentCompartment.of([
					EditorState.tabSize.of(tabSize),
					indentUnit.of(insertSpaces ? " ".repeat(tabSize) : "\t"),
				]),
				visibleWhitespace,
				keymap.of([
					...defaultKeymap,
					...searchKeymap,
					...historyKeymap,
					...foldKeymap,
					...completionKeymap,
					...closeBracketsKeymap,
					...lintKeymap,
					{
						key: 'Tab',
						run: ({ state, dispatch }) => {
							if (state.selection.ranges.some(r => !r.empty))
								return indentMore({ state, dispatch });

							const tabSize = state.facet(EditorState.tabSize);
							const line = state.doc.lineAt(state.selection.main.head);
							const pos = state.selection.main.head - line.from;
							const spaces = spacesToNextTabStop(line.text, pos, tabSize);
							dispatch(state.update(state.replaceSelection(spaces), {
								scrollIntoView: true,
								userEvent: 'input',
							}));
							return true;
						},
						shift: indentLess,
					},
					{
						key: 'Ctrl-s',
						mac: 'Cmd-s',
						run: () => {
							onsave();
							return true;
						}
					}
				]),
				EditorView.updateListener.of((update) => {
					if (update.docChanged) {
						onchange(update.state.doc.toString());
					}
				}),
				EditorView.clickAddsSelectionRange.of((e) => e.altKey),
			]
		});

		editorView = new EditorView({ state, parent: node });
		currentLanguageName = language?.name;
		loadLanguage(language);

		return {
			destroy() {
				editorView?.destroy();
				editorView = undefined;
			}
		};
	}

	// React to language prop changes only
	$effect(() => {
		if (!editorView || language?.name === currentLanguageName) return;
		currentLanguageName = language?.name;
		loadLanguage(language);
	});

	// React to indent-related props change
	$effect(() => {
		if (!editorView) return;
		editorView.dispatch({
			effects: indentCompartment.reconfigure([
				EditorState.tabSize.of(tabSize),
				indentUnit.of(insertSpaces ? ' '.repeat(tabSize) : '\t'),
			])
		});
	});
</script>

<div use:mountEditor class="code-editor-wrapper"></div>

<style>
	.code-editor-wrapper {
		width: 100%;
		height: 100%;
		overflow: hidden;
	}
</style>
