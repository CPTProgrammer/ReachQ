<script lang="ts">
	import { MarkdownSession, type Block } from './core/markdown';
    import MarkdownNodeList from './components/MarkdownNodeList.svelte';
    import { untrack } from 'svelte';

	/** The parent streams by growing this prop (e.g. text += delta). */
	let { text }: { text: string } = $props();

	const session = new MarkdownSession();
	let nodes = $state<ReturnType<typeof session.update>>([]);

	$effect(() => {
		nodes = session.update(text);
		// console.log(untrack(() => nodes));
		// console.log(untrack(() => session.syntaxTree));
	});
</script>

<div class="streaming-markdown">
	<MarkdownNodeList nodes={nodes} />
	<!-- {#each nodes as node (node)}
		<BlockNode node={node} slice={session.slice} />
	{/each} -->
</div>
