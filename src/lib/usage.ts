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
	extraUsageEnabled: boolean;
	extraUsagePercent: number;
	lastUpdatedAt: number;
	errorMessage?: string;
	shapeWarning?: string;
}

export const COLOR_WARNING = '#e0a030';
export const COLOR_CRITICAL = '#e05050';

// Percentages at or above these switch a bar away from its identity color.
// Kept in sync with WARNING_THRESHOLD / CRITICAL_THRESHOLD in tray_icon.rs.
export const WARNING_THRESHOLD = 80;
export const CRITICAL_THRESHOLD = 95;

// Bar color for a percentage: the row's identity color normally, escalating
// to amber then red so a near-limit bar is obvious at a glance.
export function barColor(percent: number, base: string): string {
	if (percent >= CRITICAL_THRESHOLD) return COLOR_CRITICAL;
	if (percent >= WARNING_THRESHOLD) return COLOR_WARNING;
	return base;
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
