<script module lang="ts">
	export interface Props {
		toolCallId: string;
		/** Whether the card's detail section is expanded. */
		expanded: boolean;
	}
</script>

<script lang="ts">
	import { Terminal } from '@xterm/xterm';
	import { FitAddon } from '@xterm/addon-fit';
	import '@xterm/xterm/css/xterm.css';
	import { agentResizeTerminal, onTerminalOutput } from '$lib/state/agent.svelte';
	import { getSettings } from '$lib/state/settings.svelte';
	import { getTerminalTheme } from '$lib/data/terminal-themes';
	import { decodeBase64 } from './utils';

	let { toolCallId, expanded }: Props = $props();

	let containerEl: HTMLDivElement | undefined = $state();
	let term: Terminal | undefined;
	let fit: FitAddon | undefined;
	let unsubscribe: (() => void) | undefined;
	let resizeObserver: ResizeObserver | undefined;

	function fitAndNotify(): void {
		const t = term;
		const f = fit;
		const el = containerEl;
		if (!t || !f || !el || !t.element) return;
		if (el.clientWidth === 0 || el.clientHeight === 0) return;
		try {
			f.fit();
			agentResizeTerminal(toolCallId, t.cols, t.rows).catch(() => {});
		} catch {
			/* fit on a hidden container */
		}
	}

	$effect(() => {
		if (!containerEl) return;
		const el = containerEl;
		const s = getSettings();
		const t = new Terminal({
			fontFamily: s.fontFamily || 'monospace',
			fontSize: Math.max(10, (s.fontSize ?? 14) - 1),
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

		unsubscribe = onTerminalOutput(toolCallId, (dataB64) => {
			t.write(decodeBase64(dataB64));
		});

		let resizeTimer: ReturnType<typeof setTimeout> | undefined;
		resizeObserver = new ResizeObserver(() => {
			if (resizeTimer) clearTimeout(resizeTimer);
			resizeTimer = setTimeout(() => {
				if (expanded) fitAndNotify();
			}, 50);
		});
		resizeObserver.observe(el);

		requestAnimationFrame(() => fitAndNotify());

		return () => {
			resizeObserver?.disconnect();
			if (resizeTimer) clearTimeout(resizeTimer);
			unsubscribe?.();
			// Leaving the layout: remote PTY returns to the default width
			// (design 01 §2.3 terminal section).
			agentResizeTerminal(toolCallId, 100, 24).catch(() => {});
			t.dispose();
			term = undefined;
			fit = undefined;
		};
	});

	// Collapse -> shrink the remote PTY to 100 cols; expand -> real width.
	$effect(() => {
		if (!term) return;
		if (expanded) {
			requestAnimationFrame(() => fitAndNotify());
		} else {
			agentResizeTerminal(toolCallId, 100, 24).catch(() => {});
		}
	});
</script>

<div class="tool-terminal" class:collapsed={!expanded}>
	<div bind:this={containerEl} class="tool-terminal-container"></div>
</div>

<style>
	.tool-terminal {
		height: 240px;
		background: var(--color-bg-primary);
		overflow: hidden;
	}

	.tool-terminal.collapsed {
		height: 0;
	}

	.tool-terminal-container {
		width: 100%;
		height: 100%;
	}

	.tool-terminal-container :global(.xterm) {
		height: 100%;
		padding: 4px 6px;
	}

	.tool-terminal-container :global(.xterm-viewport) {
		scrollbar-width: thin;
	}
</style>
