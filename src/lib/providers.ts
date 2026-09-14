// Which providers are fetched and which reach the tray icon. Mirrors
// ProviderSettings in src-tauri/src/state/provider_settings.rs: the frontend
// persists it in settings.json, Rust keeps the copy the poller reads.

export type ProviderId = 'claude' | 'kimi' | 'gpt';

export interface ProviderToggle {
	enabled: boolean;
	tray: boolean;
}

export type ProviderSettings = Record<ProviderId, ProviderToggle>;

export const PROVIDER_IDS: ProviderId[] = ['claude', 'kimi', 'gpt'];

export const PROVIDER_NAMES: Record<ProviderId, string> = {
	claude: 'Claude',
	kimi: 'Kimi',
	gpt: 'GPT'
};

// Rows the 22px tray icon fits. Kept in sync with MAX_ROWS in tray_icon.rs.
export const MAX_TRAY_PROVIDERS = 2;

// Same defaults as Rust: GPT is opt-in, see ProviderSettings::default.
export const DEFAULT_PROVIDER_SETTINGS: ProviderSettings = {
	claude: { enabled: true, tray: true },
	kimi: { enabled: true, tray: true },
	gpt: { enabled: false, tray: false }
};

export function trayCount(settings: ProviderSettings): number {
	return PROVIDER_IDS.filter((id) => settings[id].enabled && settings[id].tray).length;
}

// Turning a provider off also takes it off the tray, freeing its slot.
export function withEnabled(
	settings: ProviderSettings,
	id: ProviderId,
	enabled: boolean
): ProviderSettings {
	return { ...settings, [id]: { enabled, tray: enabled && settings[id].tray } };
}

// Refuses a disabled provider, and a third tray provider: the icon has no room.
export function withTray(
	settings: ProviderSettings,
	id: ProviderId,
	tray: boolean
): ProviderSettings {
	const current = settings[id];
	if (tray && (!current.enabled || (!current.tray && trayCount(settings) >= MAX_TRAY_PROVIDERS))) {
		return settings;
	}
	return { ...settings, [id]: { ...current, tray } };
}

// settings.json from a build before this setting has no `providers` key.
export function normalizeProviderSettings(saved: unknown): ProviderSettings {
	const result = { ...DEFAULT_PROVIDER_SETTINGS };
	if (!saved || typeof saved !== 'object') return result;
	for (const id of PROVIDER_IDS) {
		const toggle = (saved as Record<string, unknown>)[id] as Partial<ProviderToggle> | undefined;
		if (toggle && typeof toggle.enabled === 'boolean' && typeof toggle.tray === 'boolean') {
			result[id] = { enabled: toggle.enabled, tray: toggle.tray };
		}
	}
	return result;
}
