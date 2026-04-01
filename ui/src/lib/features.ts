export type DeploymentMode = 'standalone' | 'cloud';

export interface FeatureContext {
	mode: DeploymentMode;
	baseUrl: string;
	features: {
		billing: boolean;
		tenantManagement: boolean;
		oauth: boolean;
		byok: boolean;
		usage: boolean;
	};
}

const mode: DeploymentMode =
	(import.meta.env.VITE_DEPLOYMENT_MODE as DeploymentMode) || 'standalone';

export const features: FeatureContext = {
	mode,
	baseUrl: import.meta.env.VITE_API_URL || '',
	features: {
		billing: mode === 'cloud',
		tenantManagement: mode === 'cloud',
		oauth: mode === 'cloud',
		byok: mode === 'cloud',
		usage: true
	}
};
