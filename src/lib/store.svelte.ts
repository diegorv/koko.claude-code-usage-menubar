import { load, type Store } from '@tauri-apps/plugin-store';
import type { UsageData } from './usage';

let _usage = $state<UsageData | null>(null);
let _intervalSeconds = $state(120);
let _store: Store | null = null;

async function getStore(): Promise<Store> {
	if (!_store) {
		_store = await load('settings.json');
	}
	return _store;
}

export const appState = {
	get usage() {
		return _usage;
	},
	set usage(v: UsageData | null) {
		_usage = v;
	},

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
