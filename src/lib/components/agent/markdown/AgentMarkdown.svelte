<script lang="ts">
	import type { MdNode } from '$lib/state/agent-markdown.svelte';
	import MarkdownNodeList from './MarkdownNodeList.svelte';

	/**
	 * Renders one assistant text block. The parsed node list comes from the
	 * block's `MarkdownSession` (attached in `state/agent-markdown.svelte.ts`),
	 * which streaming deltas feed via `append` — this component is a pure view.
	 */
	let { nodes }: { nodes: MdNode[] } = $props();
</script>

<MarkdownNodeList {nodes} />
