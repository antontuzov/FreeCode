// Flat config for eslint 9. The rule that matters most here is the one at the bottom: no color
// literal may appear in a component, because every surface has to survive light, dark and all four
// accents without a per-screen override.
import js from "@eslint/js";
import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["dist/**", "src-tauri/**", "node_modules/**"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    plugins: { "react-hooks": reactHooks },
    rules: {
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "error",
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_" }],
      "no-restricted-syntax": [
        "error",
        {
          selector: "Literal[value=/^#[0-9a-fA-F]{3,8}$/]",
          message: "Hardcoded colors are not allowed. Use a CSS variable from styles/tokens.css.",
        },
        {
          selector: "TemplateLiteral Quasi[value.raw=/#[0-9a-fA-F]{3,8}/]",
          message: "Hardcoded colors are not allowed. Use a CSS variable from styles/tokens.css.",
        },
      ],
    },
  },
  { files: ["src/**/*.tsx"], rules: { "react-hooks/rules-of-hooks": "error" } },
);
