import eslint from "@eslint/js";
import tseslint from "@typescript-eslint/eslint-plugin";
import tsParser from "@typescript-eslint/parser";
import vuePlugin from "eslint-plugin-vue";
import vueParser from "vue-eslint-parser";
import globals from "globals";

const browserNodeGlobals = {
  ...globals.browser,
  ...globals.node,
  ...globals.es2024,
};

export default [
  {
    // server/ и packages/ — код веб-части (сессия darwaza-web): у него свой
    // тулчейн и свои правила. Линтовать его конфигурацией десктопного
    // фронтенда — значит получать чужие ошибки в своей сборке и наоборот.
    ignores: [
      "dist/**",
      "node_modules/**",
      "src-tauri/target/**",
      "server/**",
      "packages/**",
      "landing/**",
      "web/**",
    ],
  },
  eslint.configs.recommended,
  {
    files: ["**/*.ts", "**/*.tsx"],
    languageOptions: {
      parser: tsParser,
      sourceType: "module",
      ecmaVersion: "latest",
      globals: browserNodeGlobals,
    },
    plugins: { "@typescript-eslint": tseslint },
    rules: {
      // Базовое no-unused-vars не понимает type-параметры, поэтому используем только TS-версию.
      "no-unused-vars": "off",
      "@typescript-eslint/no-unused-vars": [
        "error",
        { argsIgnorePattern: "^_", varsIgnorePattern: "^_" },
      ],
      "@typescript-eslint/no-explicit-any": "warn",
    },
  },
  {
    files: ["**/*.vue"],
    languageOptions: {
      parser: vueParser,
      parserOptions: {
        parser: tsParser,
        sourceType: "module",
        ecmaVersion: "latest",
      },
      globals: browserNodeGlobals,
    },
    plugins: { vue: vuePlugin, "@typescript-eslint": tseslint },
    rules: {
      ...vuePlugin.configs["vue3-recommended"].rules,
      "vue/multi-word-component-names": "off",
      "vue/html-self-closing": "off",
    },
  },
];
