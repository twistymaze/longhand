/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import license from "rollup-plugin-license";
import { mkdirSync, writeFileSync } from "fs";
import { dirname, resolve } from "path";
import { compare } from "./scripts/licences/group";

const host = process.env.TAURI_DEV_HOST;

// The JavaScript part of the full licence list (KTD2). The plugin sees only
// the modules with code in the bundle, so build tools stay out. The licence
// generator (scripts/licences/generate.ts) reads this file.
const jsPackagesFile = resolve(__dirname, "licences/generated/js-packages.json");

const licences = {
  ...license({
    thirdParty: {
      multipleVersions: true,
      // A package with no licence fails the build.
      allow: { test: (dependency) => Boolean(dependency.license), failOnUnlicensed: true },
      output: (dependencies) => {
        const packages = dependencies
          .map((dependency) => ({
            name: dependency.name ?? "",
            version: dependency.version ?? "",
            license: dependency.license,
            licenseText: dependency.licenseText,
            noticeText: dependency.noticeText,
          }))
          .sort((a, b) =>
            a.name === b.name ? compare(a.version, b.version) : compare(a.name, b.name),
          );
        mkdirSync(dirname(jsPackagesFile), { recursive: true });
        writeFileSync(jsPackagesFile, JSON.stringify(packages, null, 2) + "\n");
      },
    },
  }),
  apply: "build" as const,
};

// Tauri expects a fixed port and must see Rust errors, so the screen is never cleared.
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss(), licences],
  resolve: {
    alias: {
      "@": resolve(__dirname, "./src"),
    },
  },
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: {
      ignored: ["**/src-tauri/**", "**/target/**", "**/reference-source/**", "**/fixtures/**"],
    },
  },
  // Vitest runs only the frontend tests. It does not skip `target/`, which
  // holds the test files of vendored packages. The setup file registers
  // happy-dom before each test file.
  test: {
    include: ["src/**/*.test.{ts,tsx}"],
    environment: "node",
    setupFiles: ["./src/test/setup.ts"],
  },
}));
