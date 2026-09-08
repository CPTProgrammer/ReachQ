<script lang="ts">
	import type { GetNodeNames } from '../core/lezer/wrapper';
	import { parser, type Node } from '../core/markdown';
    import MarkdownNode from './MarkdownNode.svelte';

	let { nodes }: { nodes: Node<GetNodeNames<typeof parser>>[] } = $props();
</script>
{#each nodes as node (node.id)}
	<MarkdownNode node={node} />
{/each}
