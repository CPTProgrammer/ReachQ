//! Active agent backend seam (design 01 §4.1): the Tauri backend in the real
//! app, swapped for the mock by /preview pages. Kept in its own module so
//! consumers that only need the seam (e.g. agent-drafts) don't pull in the
//! whole conversation state.

import { tauriBackend, type AgentBackend } from '$lib/ipc/agent-backend';

let backend: AgentBackend = tauriBackend;

export function getAgentBackend(): AgentBackend {
	return backend;
}

/** Swap the backend (preview pages only; restore on destroy). */
export function setAgentBackend(b: AgentBackend): void {
	backend = b;
}
