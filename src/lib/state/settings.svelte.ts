import { invoke } from '@tauri-apps/api/core';

export interface Settings {
	theme: 'dark' | 'light' | 'system';
	fontSize: number;
	fontFamily: string;
	terminalTheme: string;
	defaultShell: string;
	openLastSession: boolean;
	locale: string;
	minimizeToTray: boolean;
	startWithSystem: boolean;
	setupComplete: boolean;
	pendingTursoOrg: string;
	pendingTursoApiToken: string;
}

const STORAGE_KEY = 'reach-settings';

const defaults: Settings = {
	theme: 'dark',
	fontSize: 14,
	fontFamily: 'JetBrains Mono',
	terminalTheme: 'Default',
	defaultShell: '/bin/bash',
	openLastSession: false,
	locale: 'en',
	minimizeToTray: false,
	startWithSystem: false,
	setupComplete: false,
	pendingTursoOrg: '',
	pendingTursoApiToken: ''
};

let settings = $state<Settings>({ ...defaults });

export function getSettings(): Settings {
	return settings;
}

export function updateSetting<K extends keyof Settings>(key: K, value: Settings[K]): void {
	settings[key] = value;
	saveSettings();
}

export function loadSettings(): void {
	if (typeof localStorage === 'undefined') return;

	try {
		const stored = localStorage.getItem(STORAGE_KEY);
		if (stored) {
			const parsed = JSON.parse(stored) as Partial<Settings>;
			settings.theme = parsed.theme ?? defaults.theme;
			settings.fontSize = parsed.fontSize ?? defaults.fontSize;
			settings.fontFamily = parsed.fontFamily ?? defaults.fontFamily;
			settings.terminalTheme = parsed.terminalTheme ?? defaults.terminalTheme;
			settings.defaultShell = parsed.defaultShell ?? defaults.defaultShell;
			settings.openLastSession = parsed.openLastSession ?? defaults.openLastSession;
			settings.locale = parsed.locale ?? defaults.locale;
			settings.minimizeToTray = parsed.minimizeToTray ?? defaults.minimizeToTray;
			settings.startWithSystem = parsed.startWithSystem ?? defaults.startWithSystem;
			settings.pendingTursoOrg = parsed.pendingTursoOrg ?? defaults.pendingTursoOrg;
			settings.pendingTursoApiToken = parsed.pendingTursoApiToken ?? defaults.pendingTursoApiToken;
			// Migration: existing users who already have localStorage data get setupComplete: true
			settings.setupComplete = parsed.setupComplete ?? true;
		}
	} catch {
		// If parsing fails, keep defaults
	}
}

export function saveSettings(): void {
	if (typeof localStorage === 'undefined') return;

	try {
		localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
	} catch {
		// Storage might be full or unavailable
	}
}

// --- Tray sync ---

/** Sync minimizeToTray setting to Rust backend AtomicBool. Call after loadSettings(). */
export async function syncTraySettings(): Promise<void> {
	try {
		await invoke('set_close_to_tray', { enabled: settings.minimizeToTray });
	} catch {
		// Backend not ready yet
	}
}

/** Restore local settings from vault AppSettings (after backup import + relaunch). */
export async function restoreLocalSettingsFromVault(): Promise<void> {
	try {
		const vaultSettings = await invoke<Record<string, unknown>>('vault_get_settings');
		let changed = false;
		if (typeof vaultSettings.minimizeToTray === 'boolean') {
			settings.minimizeToTray = vaultSettings.minimizeToTray;
			changed = true;
		}
		if (typeof vaultSettings.startWithSystem === 'boolean') {
			settings.startWithSystem = vaultSettings.startWithSystem;
			changed = true;
		}
		if (typeof vaultSettings.defaultShell === 'string') {
			settings.defaultShell = vaultSettings.defaultShell;
			changed = true;
		}
		if (typeof vaultSettings.openLastSession === 'boolean') {
			settings.openLastSession = vaultSettings.openLastSession;
			changed = true;
		}
		if (typeof vaultSettings.fontSize === 'number') {
			settings.fontSize = vaultSettings.fontSize;
			changed = true;
		}
		if (typeof vaultSettings.fontFamily === 'string') {
			settings.fontFamily = vaultSettings.fontFamily;
			changed = true;
		}
		if (typeof vaultSettings.terminalTheme === 'string') {
			settings.terminalTheme = vaultSettings.terminalTheme;
			changed = true;
		}
		if (typeof vaultSettings.locale === 'string') {
			settings.locale = vaultSettings.locale;
			changed = true;
		}
		if (changed) {
			saveSettings();
			await syncTraySettings();
		}
	} catch {
		// Vault not unlocked or settings not available
	}
}
