import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { svelteTesting } from "@testing-library/svelte/vite";

export default defineConfig({
	plugins: [
		svelte(),
		// Adds the `browser` resolve condition — without it Node picks
		// svelte's server build, where `mount()` is unavailable.
		svelteTesting(),
	],
	test: {
		// Component tests opt into jsdom individually via a docblock
		// (`// @vitest-environment jsdom`); everything else stays on node.
		environment: "node",
	},
});
