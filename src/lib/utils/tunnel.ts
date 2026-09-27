/**
 * Tunnel type helpers — shared by TunnelManager and TunnelCard.
 */

import { t } from '$lib/state/i18n.svelte';
import type { TunnelConfig } from '$lib/ipc/tunnel';

export type TunnelTypeName = TunnelConfig['tunnel_type'];

export const TUNNEL_TYPES: TunnelTypeName[] = ['Local', 'Remote', 'Dynamic'];

/** Localized display name for a tunnel type (e.g. "Local Forward" / "本地转发"). */
export function tunnelTypeLabel(type: TunnelTypeName): string {
	switch (type) {
		case 'Local':
			return t('tunnel.local_forward');
		case 'Remote':
			return t('tunnel.remote_forward');
		case 'Dynamic':
			return t('tunnel.dynamic');
	}
}
