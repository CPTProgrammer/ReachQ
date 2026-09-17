<script lang="ts">
	import type { GetNodeNames } from '../core/lezer/wrapper';
	import { parser, type Node } from '../core/markdown';
	import { groupRenderUnits } from '../core/render';
	import MarkdownNode from './MarkdownNode.svelte';

	let { nodes }: { nodes: Node<GetNodeNames<typeof parser>>[] } = $props();

	const units = $derived(groupRenderUnits(nodes));
</script>
{#each units as unit (unit.key)}
	{#if unit.node}
		<MarkdownNode node={unit.node} />
	{:else}
		{@html unit.html}
	{/if}
{/each}
