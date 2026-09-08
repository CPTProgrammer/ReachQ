<script lang="ts">
	import { IncrementalMarkdown } from '../src/index';
	import { SAMPLE } from './sample';

	let source = $state(SAMPLE);
	/** Milliseconds per character. Browsers clamp intervals to ~4ms anyway. */
	let speed = $state(20);
	let pos = $state(0);
	let timer = $state<ReturnType<typeof setInterval> | null>(null);

	const output = $derived(source.slice(0, pos));
	const running = $derived(timer !== null);
	const done = $derived(pos >= source.length);

	function pause() {
		if (timer !== null) {
			clearInterval(timer);
			timer = null;
		}
	}

	function start() {
		if (timer !== null || pos >= source.length) return;
		timer = setInterval(
			() => {
				pos += 1;
				if (pos >= source.length) pause();
			},
			Math.max(1, Math.floor(speed))
		);
	}

	function reset() {
		pause();
		pos = 0;
	}

	function onSpeedInput(e: Event) {
		speed = Number((e.target as HTMLInputElement).value);
		// Apply the new interval immediately when changed mid-stream.
		if (running) {
			pause();
			start();
		}
	}

	// Editing the source mid-stream: clamp progress to the new length.
	$effect(() => {
		if (pos > source.length) pos = source.length;
	});
</script>

<div class="page">
	<header class="topbar">
		<nav class="nav">
			<a href="/index.html">← Index</a>
			<a href="/static.html">Static</a>
		</nav>
		<div class="controls">
			<label>
				Speed
				<input type="number" min="1" value={speed} oninput={onSpeedInput} />
				ms/char
			</label>
			<button onclick={start} disabled={running || done}>Start</button>
			<button onclick={pause} disabled={!running}>Pause</button>
			<button onclick={reset} disabled={pos === 0 && !running}>Reset</button>
			<span class="progress" class:done>{pos} / {source.length} chars</span>
		</div>
	</header>
	<main class="split">
		<textarea class="source" bind:value={source} spellcheck="false"></textarea>
		<div class="rendered">
			<IncrementalMarkdown text={output} />
		</div>
	</main>
</div>
