<script lang="ts">
	import { IncrementalMarkdown } from '../src/index';
	import { parser } from '../src/core/markdown';
	import TreeView from './TreeView.svelte';
	import { SAMPLE } from './sample';

	let text = $state(SAMPLE);

	// Parsed separately from the renderer above — the playground pays for two
	// parses so the tree panel stays decoupled from IncrementalMarkdown internals.
	let tree = $derived(parser.parse(text));
</script>

<div class="page">
	<header class="topbar">
		<nav class="nav">
			<a href="/index.html">← Index</a>
			<a href="/stream.html">Streaming</a>
		</nav>
		<span class="hint">Edit on the left — rendered in the middle, syntax tree on the right.</span>
	</header>
	<main class="split">
		<textarea class="source" bind:value={text} spellcheck="false"></textarea>
		<div class="rendered">
			<IncrementalMarkdown {text} />
		</div>
		<aside class="tree-panel">
			<div class="tree-header">Syntax Tree</div>
			<div class="tree-scroll">
				<TreeView {tree} />
			</div>
		</aside>
	</main>
</div>
