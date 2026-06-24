import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { sshConfirmHostKey } from '$lib/ipc/ssh';

export interface HostKeyVerifyRequest {
    host: string;
    port: number;
    fingerprint: string;
    keyType: string;
    isNew: boolean;
    oldFingerprint?: string;
    /** Unix timestamp in milliseconds when the SSH connection will time out. */
    deadlineMs: number;
}

let pending = $state<HostKeyVerifyRequest | null>(null);
let initialized = false;
let unlisten: UnlistenFn | undefined;

/** Start listening for host-key-verify events from the backend. */
export function initHostKeyListener(): void {
    if (initialized) return;
    initialized = true;

    listen<HostKeyVerifyRequest>('host-key-verify', (event) => {
        pending = event.payload;
    }).then((fn) => {
        unlisten = fn;
    });
}

/** Get the currently pending host key verification, if any. */
export function getPendingHostKey(): HostKeyVerifyRequest | null {
    return pending;
}

/** Send the user's decision back to the backend and clear the pending request. */
export async function respondHostKey(
    host: string,
    port: number,
    decision: 'accept' | 'accept-once' | 'reject'
): Promise<void> {
    await sshConfirmHostKey(host, port, decision);
    pending = null;
}

/** Dismiss the pending host-key dialog without sending a decision.
 * Used when the connection times out from the backend side. */
export function clearPendingHostKey(): void {
    pending = null;
}

/** Stop listening for events (called on unmount). */
export function stopHostKeyListener(): void {
    unlisten?.();
    initialized = false;
}
