import { load, type Store } from '@tauri-apps/plugin-store';

let _intervalSeconds = $state(120);
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

	async loadSettings() {
		try {
			const store = await getStore();
			const saved = await store.get<number>('intervalSeconds');
			if (saved && typeof saved === 'number') {
				_intervalSeconds = saved;
			}
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
	}
};
