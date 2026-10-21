import { bindings, defineConfig } from "cf/config";

export default defineConfig({
	worker: {
		name: "aa-web",
		compatibilityDate: "2026-10-01",
		entrypoint: "worker.js",
		workersDev: true,
		observability: {
			enabled: true,
		},
		assets: {
			runWorkerFirst: [
				"/pak/*",
				"/net/*",
				"/admin/*",
				"/client/pkg/aa-client_bg.wasm",
			],
		},
		env: {
			PAK: bindings.r2({
				name: "aa-web",
			}),
			ASSETS: bindings.assets(),
			// upload.mjs authenticates with it; deploy with --secrets-file (see README.md)
			ADMIN_TOKEN: bindings.secret(),
			// the login screen's default server, kept out of the repo: NET_SERVER "host:port"; NET_PORTS the ports
			// players may reach on any host ("1237,1239,1250"); both in the secrets file
			NET_SERVER: bindings.secret(),
			NET_PORTS: bindings.secret(),
		},
	},
});
