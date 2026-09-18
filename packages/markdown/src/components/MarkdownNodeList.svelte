<script lang="ts">
	import { getContext } from 'svelte';
	import type { GetNodeNames } from '../core/lezer/wrapper';
	import { parser, type Node } from '../core/markdown';
	import { groupRenderUnits } from '../core/render';
	import { sanitizeHtml } from '../sanitize';
	import MarkdownNode from './MarkdownNode.svelte';

	let { nodes }: { nodes: Node<GetNodeNames<typeof parser>>[] } = $props();

	const sanitize = getContext('reach-md-sanitize') !== false;
	const units = $derived(groupRenderUnits(nodes));
</script>
{#each units as unit (unit.key)}
	{#if unit.node}
		<MarkdownNode node={unit.node} />
	{:else}
		{@html sanitize ? sanitizeHtml(unit.html) : unit.html}
	{/if}
{/each}
