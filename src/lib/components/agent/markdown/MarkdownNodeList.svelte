<script lang="ts">
	import type { GetNodeNames } from '@reach/markdown';
	import { parser, type Node } from '@reach/markdown';
	import { groupRenderUnits, sanitizeHtml } from '@reach/markdown';
	import MarkdownNode from './MarkdownNode.svelte';
	import { cjkWbrHtml } from '$lib/utils/markdown';

	let { nodes }: { nodes: Node<GetNodeNames<typeof parser>>[] } = $props();

	const units = $derived(groupRenderUnits(nodes));
</script>
{#each units as unit (unit.key)}
	{#if unit.node}
		<MarkdownNode node={unit.node} />
	{:else}
		<!-- raw-HTML run: sanitize, then restore CJK break opportunities -->
		{@html cjkWbrHtml(sanitizeHtml(unit.html))}
	{/if}
{/each}
