import { FlatCompat } from "@eslint/eslintrc";

const compat = new FlatCompat({
  baseDirectory: import.meta.dirname,
});

const eslintConfig = [
  ...compat.extends("next/core-web-vitals", "next/typescript"),
  {
    ignores: [
      ".next/**",
      "node_modules/**",
      "next-env.d.ts",
      // Plain-Node CommonJS payload file, spawned directly by the desktop
      // launcher (not bundled by Next); CJS require() is intentional.
      "lan-gate.js",
    ],
  },
];

export default eslintConfig;
