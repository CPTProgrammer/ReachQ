<script lang="ts">
	import Input from '$lib/components/shared/Input.svelte';
	import { t } from '$lib/state/i18n.svelte';

	interface Props {
		host: string;
		port: string;
		disabled?: boolean;
	}

	let { host = $bindable(''), port = $bindable('22'), disabled = false }: Props = $props();
</script>

<div class="row">
	<div class="field-host">
		<Input label={t('session.host')} bind:value={host} placeholder="192.168.1.1" {disabled} />
	</div>
	<div class="field-port">
		<Input label={t('session.port')} bind:value={port} type="number" placeholder="22" {disabled} />
	</div>
</div>

<style>
	.row {
		display: flex;
		gap: 10px;
		align-items: flex-start;
	}
	.field-host {
		flex: 1;
		min-width: 0;
	}
	.field-port {
		width: 80px;
		flex-shrink: 0;
	}
</style>
