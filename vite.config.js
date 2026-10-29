import { defineConfig } from "vite";
import { sveltekit } from "@sveltejs/kit/vite";
import tailwindcss from "@tailwindcss/vite";
import process from "node:process";
const host = process.env.TAURI_DEV_HOST;

export default defineConfig(() => ({
	plugins: [tailwindcss(), sveltekit()],
	// https://svelte.dev/docs/svelte/testing: tests run Svelte's client build so runes stay reactive.
	resolve: process.env.VITEST ? { conditions: ["browser"] } : undefined,
	test: {
		environment: "jsdom",
		include: ["src/**/*.test.ts"],
	},

	// prevent Vite from obscuring rust errors
	clearScreen: false,
	// tauri expects a fixed port, fail if that port is not available
	server: {
		port: 1420,
		strictPort: true,
		host: host || "127.0.0.1",
		hmr: host
			? {
					protocol: "ws",
					host,
					port: 1421,
				}
			: undefined,
		watch: {
			ignored: ["**/src-tauri/**"],
		},
	},
}));
