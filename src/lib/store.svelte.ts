import { load, type Store } from '@tauri-apps/plugin-store';
import {
	DEFAULT_PROVIDER_SETTINGS,
	normalizeProviderSettings,
	type ProviderSettings
} from './providers';

let _intervalSeconds = $state(120);
let _providerSettings = $state<ProviderSettings>(DEFAULT_PROVIDER_SETTINGS);
let _store: Store | null = null;

async function getStore(): Promise<Store> {
	if (!_store) {
		_store = await load('settings.json');
	}
	return _store;
}

export const appState = {
	get intervalSeconds() {
		return _intervalSeconds;
	},

	get providerSettings() {
		return _providerSettings;
	},

	async loadSettings() {
		try {
			const store = await getStore();
			const saved = await store.get<number>('intervalSeconds');
			if (saved && typeof saved === 'number') {
				_intervalSeconds = saved;
			}
			_providerSettings = normalizeProviderSettings(await store.get('providers'));
		} catch {
			// Use defaults on error
		}
	},

	async saveInterval(seconds: number) {
		_intervalSeconds = seconds;
		try {
			const store = await getStore();
			await store.set('intervalSeconds', seconds);
		} catch {
			// Silently fail
		}
	},

	// Saved explicitly rather than left to autosave: Rust reads this file at
	// startup, and a quit inside the autosave debounce would lose the change.
	async saveProviderSettings(settings: ProviderSettings) {
		_providerSettings = settings;
		try {
			const store = await getStore();
			await store.set('providers', settings);
			await store.save();
		} catch {
			// Silently fail
		}
	}
};
