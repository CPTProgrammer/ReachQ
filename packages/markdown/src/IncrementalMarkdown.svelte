<script lang="ts">
	import { MarkdownSession, type Block } from './core/markdown';
    import MarkdownNodeList from './components/MarkdownNodeList.svelte';
    import { setContext, untrack } from 'svelte';

	/** The parent streams by growing this prop (e.g. text += delta). */
	let { text, sanitize = true }: { text: string; sanitize?: boolean } = $props();

	// Consumed once at component init (context is static per tree); toggling
	// the prop on a mounted tree has no effect. sanitize=false exists for the
	// spec-conformance suite, which compares against cmark's raw passthrough.
	setContext('reach-md-sanitize', untrack(() => sanitize));

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
