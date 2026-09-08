<script lang="ts">
	import { SvelteMap } from 'svelte/reactivity';
	import type { TypedSyntaxNode, TypedTree } from '../src/core/lezer/wrapper';

	let { tree }: { tree: TypedTree | null } = $props();

	// Expand-state overrides keyed by root-to-node index path (e.g. "0/2/1"),
	// which stays stable across re-parses for unchanged parts of the document.
	const overrides = new SvelteMap<string, boolean>();

	const DEFAULT_OPEN_DEPTH = 3;

	function isOpen(path: string, depth: number): boolean {
		return overrides.get(path) ?? depth < DEFAULT_OPEN_DEPTH;
	}

	function toggle(path: string, depth: number) {
		overrides.set(path, !isOpen(path, depth));
	}

	function childrenOf(node: TypedSyntaxNode<string>): TypedSyntaxNode<string>[] {
		const out: TypedSyntaxNode<string>[] = [];
		for (let c = node.firstChild; c; c = c.nextSibling) out.push(c);
		return out;
	}
</script>

{#snippet nodeRow(node: TypedSyntaxNode<string>, depth: number, path: string)}
	{@const children = childrenOf(node)}
	{@const open = isOpen(path, depth)}
	<div class="tree-node">
		<button
			class="tree-row"
			style="padding-left: {depth * 14 + 8}px"
			onclick={() => toggle(path, depth)}
			disabled={children.length === 0}
		>
			<span class="tree-toggle" class:hidden={children.length === 0}>
				{open ? '▾' : '▸'}
			</span>
			<span class="tree-name">{node.name}</span>
			<span class="tree-range">[{node.from}, {node.to}]</span>
		</button>
		{#if open}
			{#each children as child, i}
				{@render nodeRow(child, depth + 1, path + '/' + i)}
			{/each}
		{/if}
	</div>
{/snippet}

<div class="tree-view">
	{#if tree}
		{@render nodeRow(tree.topNode, 0, '0')}
	{:else}
		<p class="tree-empty">Nothing parsed yet.</p>
	{/if}
</div>
