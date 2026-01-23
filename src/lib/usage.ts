export interface ModelUsage {
	name: string;
	percent: number;
	resetsAt?: string;
}

export interface UsageData {
	status: 'ok' | 'error' | 'unauthorized' | 'missing_credentials';
	sessionPercent: number;
	sessionResetsAt?: string;
	weeklyPercent: number;
	weeklyResetsAt?: string;
	models: ModelUsage[];
	lastUpdatedAt: number;
	errorMessage?: string;
}

export function formatTimeRemaining(iso: string | undefined): string {
	if (!iso) return '';
	const now = Date.now();
	const target = new Date(iso).getTime();
	const diff = target - now;

	if (diff <= 0) return 'Now';

	const hours = Math.floor(diff / 3_600_000);
	const minutes = Math.floor((diff % 3_600_000) / 60_000);

	if (hours >= 24) {
		const days = Math.floor(hours / 24);
		const remainingHours = hours % 24;
		return `${days}d ${remainingHours}h`;
	}
	if (hours > 0) return `${hours}h ${minutes}m`;
	return `${minutes}m`;
}
