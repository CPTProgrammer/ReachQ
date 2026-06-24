<script lang="ts">
	interface Props {
		x: number;
		y: number;
		onclose: () => void;
		children: import('svelte').Snippet;
	}

	let { x, y, onclose, children }: Props = $props();
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="context-menu" style="left: {x}px; top: {y}px;" onclick={onclose} onkeydown={() => {}}>
	{@render children()}
</div>

<style>
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
