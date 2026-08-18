import { dev } from '$app/environment';
import { error } from '@sveltejs/kit';

// Dev-only preview tool: no prerender, 404 in production builds so it
// never ships inside the Tauri app.
export const ssr = false;
export const prerender = false;

export function load() {
	if (!dev) error(404, 'Not found');
}
