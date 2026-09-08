import { fileURLToPath } from 'node:url';
import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Dev-only playground for the StreamingMarkdown component.
// Run with: npm run dev -w @reach/markdown
export default defineConfig({
	plugins: [svelte()],
	server: {
		port: 5180,
		strictPort: true
	},
	build: {
		rollupOptions: {
			input: {
				index: fileURLToPath(new URL('./index.html', import.meta.url)),
				stream: fileURLToPath(new URL('./stream.html', import.meta.url)),
				static: fileURLToPath(new URL('./static.html', import.meta.url))
			}
		}
	}
});
