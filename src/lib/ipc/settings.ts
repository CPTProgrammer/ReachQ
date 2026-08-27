/**
 * Settings IPC wrappers.
 * All operations are O(1) - settings stored encrypted in vault.
 */

import { invoke } from '@tauri-apps/api/core';

/**
 * App settings structure.
 * The legacy fixed AI fields (openrouter_*) were removed (design 04 §5);
 * agent configuration now lives in the vault's generic kv via the agent
 * commands. The struct is kept (empty) so `settings_get_all` /
 * `settings_save_all` stay wire-compatible.
 */
export interface AppSettings {}

/** Get all app settings. O(1) per setting. */
export async function getAll(): Promise<AppSettings> {
	return invoke<AppSettings>('settings_get_all');
}

/** Get a single setting by key. O(1). */
export async function get(key: string): Promise<string | null> {
	return invoke<string | null>('settings_get', { key });
}

/** Set a setting value. O(1). */
export async function set(key: string, value: string): Promise<void> {
	return invoke('settings_set', { key, value });
}

/** Delete a setting. O(1). */
export async function remove(key: string): Promise<void> {
	return invoke('settings_delete', { key });
}

/** Save all app settings at once. */
export async function saveAll(settings: Partial<AppSettings>): Promise<void> {
	return invoke('settings_save_all', { settings });
}

/** Enumerate system fonts. Returns deduplicated, sorted font family names. */
export async function listSystemFonts(): Promise<string[]> {
	return invoke<string[]>('list_system_fonts');
}
