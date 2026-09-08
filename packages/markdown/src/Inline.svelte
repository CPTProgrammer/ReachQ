<script lang="ts" generics="N extends string">
	import type { Slice } from './core/markdown';
	import Inline from './Inline.svelte';
	import { type TypedSyntaxNode } from './core/lezer/wrapper';

	let { node, slice }: { node: TypedSyntaxNode<N>; slice: Slice } = $props();

	type Token = { kind: 'text'; text: string } | { kind: 'node'; child: TypedSyntaxNode<N> };

	// Lezer inline content: plain text lives in the gaps BETWEEN child nodes,
	// so tokenize into alternating text slices and child nodes, in source order.
	const tokens = $derived.by(() => {
		const out: Token[] = [];
		let pos = node.from;
		for (let c = node.firstChild; c; c = c.nextSibling) {
			if (c.from > pos) out.push({ kind: 'text', text: slice(pos, c.from) });
			out.push({ kind: 'node', child: c });
			pos = c.to;
		}
		if (node.to > pos) out.push({ kind: 'text', text: slice(pos, node.to) });
		return out;
	});
</script>

{#each tokens as t (t.kind === 'node' ? t.child.from : t)}
	{#if t.kind === 'text'}
		{t.text}
	{:else if t.child.name.endsWith('Mark') || t.child.name === 'CodeInfo'}
		<!-- delimiter tokens (**, `, #, ...) render as nothing -->
	{:else if t.child.name === 'StrongEmphasis'}
		<strong><Inline node={t.child} {slice} /></strong>
	{:else if t.child.name === 'Emphasis'}
		<em><Inline node={t.child} {slice} /></em>
	{:else if t.child.name === 'Strikethrough'}
		<del><Inline node={t.child} {slice} /></del>
	{:else if t.child.name === 'CodeText'}
		<code><Inline node={t.child} {slice} /></code>
	{:else if t.child.name === 'HardBreak'}
		<br>
	{:else}
		<!-- Fallback (Link, Image, Entity, ...): raw text, auto-escaped -->
		{slice(t.child.from, t.child.to)}
	{/if}
{/each}
