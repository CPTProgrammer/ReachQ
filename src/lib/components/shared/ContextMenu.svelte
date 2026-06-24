<script lang="ts">
	import { untrack } from 'svelte';

	interface Props {
		x: number;
		y: number;
		onclose: () => void;
		children: import('svelte').Snippet;
	}

	let { x, y, onclose, children }: Props = $props();

	let menuEl = $state<HTMLDivElement>();
	let ready = $state(false);
	let posX = $state(untrack(() => x));
	let posY = $state(untrack(() => y));

	$effect(() => {
		const el = menuEl;
		if (!el) return;

		const rect = el.getBoundingClientRect();
		const vw = window.innerWidth;
		const vh = window.innerHeight;

		if (x + rect.width > vw) posX = vw - rect.width - 4;
		if (y + rect.height > vh) posY = y - rect.height;

		ready = true;
	});
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="context-backdrop" onmousedown={onclose}>
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div
		class="context-menu"
		bind:this={menuEl}
		style="left: {posX}px; top: {posY}px; visibility: {ready ? 'visible' : 'hidden'};"
		onmousedown={(e) => e.stopPropagation()}
	>
		{@render children()}
	</div>
</div>

<style>
	.context-backdrop {
		position: fixed;
		inset: 0;
		z-index: 999;
	}

	.context-menu {
		position: fixed;
		min-width: 160px;
		padding: 4px 0;
		background-color: var(--color-bg-elevated, #1c1c1e);
		border: 1px solid var(--color-border);
		border-radius: 8px;
		box-shadow: 0 8px 32px rgba(0, 0, 0, 0.35);
		z-index: 1000;
	}

	:global(.context-sep) {
		height: 1px;
		margin: 4px 0;
		background-color: var(--color-border);
	}
</style>
