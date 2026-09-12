<script lang="ts">
	import { getProviderIcon } from '$lib/data/provider-icons';

	let { preset, size = 12 }: { preset: string | undefined | null; size?: number } = $props();

	let icon = $derived(getProviderIcon(preset));
</script>

{#if icon}
	<svg width={size} height={size} viewBox={icon.viewBox} fill="currentColor" aria-hidden="true">
		{#each icon.paths as d}
			<path {d} />
		{/each}
	</svg>
{/if}
