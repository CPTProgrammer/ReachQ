import { invoke } from '@tauri-apps/api/core';

export interface TunnelConfig {
  id: string;
  tunnel_type: 'Local' | 'Remote' | 'Dynamic';
  local_port: number;
  remote_host: string;
  remote_port: number;
  /** Saved session this tunnel belongs to (persisted in the vault). */
  session_id?: string | null;
  /** Runtime connection binding (ephemeral, quick-connect tunnels only). */
  connection_id?: string | null;
  active: boolean;
}

/**
 * Create a tunnel. With `sessionId` it is persisted across restarts;
 * with only `connectionId` it is ephemeral (quick connect).
 */
export async function tunnelCreate(
  tunnelType: 'Local' | 'Remote' | 'Dynamic',
  localPort: number,
  remoteHost: string,
  remotePort: number,
  connectionId: string | null,
  sessionId: string | null
): Promise<TunnelConfig> {
  return invoke<TunnelConfig>('tunnel_create', { tunnelType, localPort, remoteHost, remotePort, connectionId, sessionId });
}

export async function tunnelStart(tunnelId: string, preferConnectionId?: string | null): Promise<void> {
  return invoke('tunnel_start', { tunnelId, preferConnectionId: preferConnectionId ?? null });
}

export async function tunnelStop(tunnelId: string): Promise<void> {
  return invoke('tunnel_stop', { tunnelId });
}

export async function tunnelDelete(tunnelId: string): Promise<void> {
  return invoke('tunnel_delete', { tunnelId });
}

export async function tunnelList(): Promise<TunnelConfig[]> {
  return invoke<TunnelConfig[]>('tunnel_list');
}
