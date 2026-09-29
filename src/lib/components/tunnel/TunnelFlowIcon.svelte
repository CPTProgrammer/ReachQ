<script lang="ts">
	import type { TunnelTypeName } from '$lib/utils/tunnel';

	interface Props {
		type: TunnelTypeName;
	}

	let { type }: Props = $props();
</script>

<!-- Elbow arrow shared by TunnelCard and TunnelManager's preview.
     Same └-shaped line for all types; only the arrowhead end differs.
     Local/Dynamic: head on the right end (flow up into the server row).
     Remote: head on the bottom end (flow down to the local row). -->
<svg class="flow-icon" width="14" height="14" viewBox="0 0 24 24" fill="none">
	{#if type === 'Remote'}
		<path d="M9 23v-9a4 4 0 0 1 4-4h6M5 19l4 4 4-4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
	{:else}
		<path d="M9 20v-6a4 4 0 0 1 4-4h9M18 6l4 4-4 4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" />
	{/if}
</svg>

<style>
	.flow-icon {
		flex-shrink: 0;
		color: var(--color-text-secondary);
		opacity: 0.6;
	}
</style>
