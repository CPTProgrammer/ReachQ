<script lang="ts">
	import type { Snippet } from 'svelte';

	interface Props {
		onclick: () => void;
		danger?: boolean;
		indent?: boolean;
		label: string;
		children?: Snippet;
	}

	let { onclick, danger = false, indent = false, label, children }: Props = $props();
</script>

<button class="context-item" class:danger class:indent onclick={onclick} type="button">
	{#if children}
		{@render children()}
	{:else}
		<span class="context-item-label">{label}</span>
	{/if}
</button>

<style>
	.context-item {
		display: flex;
		align-items: center;
		gap: 8px;
		width: 100%;
		padding: 6px 12px;
		border: none;
		background: transparent;
		color: var(--color-text-primary);
		font-family: var(--font-sans);
		font-size: 0.75rem;
		cursor: pointer;
		text-align: left;
		transition: background-color 0.1s ease;
	}

	.context-item:hover {
		background-color: rgba(255, 255, 255, 0.08);
	}

	.context-item.danger {
		color: var(--color-danger, #ff453a);
	}

	.context-item.danger:hover {
		background-color: rgba(255, 69, 58, 0.12);
	}

	.context-item.indent {
		padding-left: 20px;
	}

	:global(.context-item-label) {
		white-space: nowrap;
		overflow: hidden;
		text-overflow: ellipsis;
	}
</style>
