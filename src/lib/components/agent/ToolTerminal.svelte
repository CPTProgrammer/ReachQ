<script module lang="ts">
	export interface Props {
		toolCallId: string;
		/** Whether the card's detail section is expanded. The xterm exists only
		 *  while this is true: created on expand, disposed on collapse. */
		expanded: boolean;
		/** Settled-call output restored from the persisted tool result when no
		 *  live output buffer exists (app restart, eviction). Plain text, no
		 *  ANSI; PTY-width line wrapping is baked in. */
		fallbackText?: string | null;
	}
</script>

<script lang="ts">
	import { untrack } from 'svelte';
	import { Terminal } from '@xterm/xterm';
	import { FitAddon } from '@xterm/addon-fit';
	import '@xterm/xterm/css/xterm.css';
	import { agentResizeTerminal, onTerminalOutput } from '$lib/state/agent.svelte';
	import { getSettings } from '$lib/state/settings.svelte';
	import { getTerminalTheme } from '$lib/data/terminal-themes';
	import { decodeBase64 } from '$lib/utils/agent';

	/** The card grows with its content between these row bounds; past the cap
	 *  the xterm viewport scrolls internally. */
	const MIN_ROWS = 5;
	const MAX_ROWS = 1000;

	let { toolCallId, expanded, fallbackText }: Props = $props();

	let containerEl: HTMLDivElement | undefined = $state();
	let term: Terminal | undefined;
	let fit: FitAddon | undefined;
	let unsubscribe: (() => void) | undefined;
	let unsubscribeFeed: (() => void) | undefined;
	let resizeObserver: ResizeObserver | undefined;
	let sizeRaf: number | undefined;
	let notifiedCols = 0;
	let notifiedRows = 0;
	// Captured once from the prop (which never changes for a mounted card)
	// instead of read on every call: notifyPty also runs from the effect
	// teardown below, and reading a prop during branch destruction re-enters
	// the parent getters mid-destroy, which makes Svelte re-execute an
	// ancestor's dirty derived against stale `old_values` — silently rewiring
	// its subscriptions away from the newly selected thread's runtime (the
	// chat then stayed empty until switching threads away and back).
	let ptyCallId = '';

	function notifyPty(cols: number, rows: number): void {
		if (cols === notifiedCols && rows === notifiedRows) return;
		notifiedCols = cols;
		notifiedRows = rows;
		agentResizeTerminal(ptyCallId, cols, rows).catch(() => {});
	}

	/** Size the grid to its content: columns from the card width, rows from
	 *  the buffer length clamped to [MIN_ROWS, MAX_ROWS]. The wrapper has no
	 *  fixed height — .xterm-screen is in normal flow and the renderer sets
	 *  its pixel height on every resize, so the card simply wraps the grid. */
	function updateSize(): void {
		const t = term;
		const f = fit;
		const el = containerEl;
		if (!t || !f || !el || !t.element) return;
		if (!expanded || el.clientWidth === 0) return;
		try {
			const proposed = f.proposeDimensions();
			if (!proposed) return;
			// Columns first so reflow settles before lines are counted.
			if (proposed.cols !== t.cols) {
				t.resize(proposed.cols, t.rows);
			}
			const rows = Math.min(MAX_ROWS, Math.max(MIN_ROWS, t.buffer.active.length));
			if (rows !== t.rows) {
				t.resize(t.cols, rows);
			}
			notifyPty(t.cols, t.rows);
		} catch {
			/* measure on a hidden container */
		}
	}

	function scheduleSizeUpdate(): void {
		if (sizeRaf !== undefined) return;
		sizeRaf = requestAnimationFrame(() => {
			sizeRaf = undefined;
			updateSize();
		});
	}

	$effect(() => {
		if (!containerEl) return;
		const el = containerEl;
		// Creation-time inputs are untracked: re-running this effect would
		// dispose the xterm and lose its content. `toolCallId` never changes
		// for a mounted card; settings changes are applied live below.
		const id = untrack(() => toolCallId);
		ptyCallId = id;
		const fb = untrack(() => fallbackText);
		const s = untrack(getSettings);
		const t = new Terminal({
			fontFamily: s.fontFamily || 'monospace',
			fontSize: Math.max(10, (s.fontSize ?? 14) - 1),
			cols: 100,
			rows: MIN_ROWS,
			cursorBlink: false,
			disableStdin: true,
			scrollback: 5000,
			allowProposedApi: true,
			theme: getTerminalTheme(s.terminalTheme)
		});
		const f = new FitAddon();
		t.loadAddon(f);
		t.open(el);
		term = t;
		fit = f;

		// Synchronous mount settle: columns first so the replay wraps at the
		// final width, then a synchronous replay, then rows grown to fit. Each
		// step is synchronous per xterm's source, so the first paint already
		// shows the final height — no frame-by-frame growth on (re)mount.
		try {
			const proposed = f.proposeDimensions();
			if (proposed && proposed.cols !== t.cols) t.resize(proposed.cols, t.rows);
		} catch {
			/* measure on a hidden container */
		}

		let replayed = 0;
		unsubscribe = onTerminalOutput(id, (dataB64) => {
			replayed++;
			t.write(decodeBase64(dataB64));
		});

		// Settled call with no buffered output to replay (app restart, buffer
		// eviction): restore the persisted text projection so the card is not
		// blank. The grid text is LF-joined; xterm needs CRLF to return the
		// carriage.
		if (replayed === 0 && fb) {
			t.write(fb.replace(/\r?\n/g, '\r\n'));
		}

		// Rows to fit the replayed content + the single PTY size notification.
		updateSize();

		// Live growth from here on: a line feed schedules an incremental resize.
		const feedDisposable = t.onLineFeed(() => scheduleSizeUpdate());
		unsubscribeFeed = () => feedDisposable.dispose();

		let resizeTimer: ReturnType<typeof setTimeout> | undefined;
		resizeObserver = new ResizeObserver(() => {
			if (resizeTimer) clearTimeout(resizeTimer);
			resizeTimer = setTimeout(() => scheduleSizeUpdate(), 50);
		});
		resizeObserver.observe(el);

		return () => {
			resizeObserver?.disconnect();
			if (resizeTimer) clearTimeout(resizeTimer);
			if (sizeRaf !== undefined) cancelAnimationFrame(sizeRaf);
			unsubscribe?.();
			unsubscribeFeed?.();
			// Leaving the layout: remote PTY returns to the default width
			// (design 01 §2.3 terminal section).
			notifyPty(100, 24);
			t.dispose();
			term = undefined;
			fit = undefined;
		};
	});

	// Apply font/theme setting changes live, without recreating the terminal
	// (same pattern as the main Terminal view: options + refit, no dispose).
	$effect(() => {
		const s = getSettings();
		const t = term;
		if (!t) return;
		const fontFamily = s.fontFamily || 'monospace';
		const fontSize = Math.max(10, (s.fontSize ?? 14) - 1);
		const theme = getTerminalTheme(s.terminalTheme);
		if (t.options.fontFamily !== fontFamily || t.options.fontSize !== fontSize) {
			t.options.fontFamily = fontFamily;
			t.options.fontSize = fontSize;
			t.clearTextureAtlas();
			// Char-size re-measure and the screen height update are synchronous;
			// settle in the same frame instead of a rAF later.
			updateSize();
		}
		if (t.options.theme !== theme) {
			t.options.theme = theme;
			t.clearTextureAtlas();
		}
	});

	// ── Collapse → shrink the remote PTY to 100 cols; expand → fit content ──
	// Handled by the creation effect: mounting the container (expand) fits the
	// content; teardown (collapse) notifies 100×24 before disposing.
</script>

<div class="tool-terminal">
	<!-- The container exists only while expanded: mounting it runs the
	     creation effect above (xterm + buffer replay), leaving disposes it.
	     Settled cards therefore cost nothing until the user expands them. -->
	{#if expanded}
		<div bind:this={containerEl} class="tool-terminal-container"></div>
	{/if}
</div>

<style>
	.tool-terminal {
		background: var(--color-bg-primary);
		overflow: hidden;
	}

	.tool-terminal-container {
		width: 100%;
	}

	.tool-terminal-container :global(.xterm) {
		padding: 4px 6px;
	}

	/* The viewport keeps xterm's default overflow-y: scroll, so the scrollbar
	   gutter is always present but only usable past the MAX_ROWS cap. */
	.tool-terminal-container :global(.xterm-viewport) {
		scrollbar-width: thin;
		scrollbar-color: rgba(255, 255, 255, 0.15) transparent;
	}

	.tool-terminal-container :global(.xterm-viewport::-webkit-scrollbar) {
		width: 6px;
	}

	.tool-terminal-container :global(.xterm-viewport::-webkit-scrollbar-track) {
		background: transparent;
	}

	.tool-terminal-container :global(.xterm-viewport::-webkit-scrollbar-thumb) {
		background-color: rgba(255, 255, 255, 0.15);
		border-radius: 3px;
	}

	.tool-terminal-container :global(.xterm-viewport::-webkit-scrollbar-thumb:hover) {
		background-color: rgba(255, 255, 255, 0.25);
	}
</style>
