import type { Config } from "tailwindcss";

const config: Config = {
  content: [
    "./pages/**/*.{js,ts,jsx,tsx,mdx}",
    "./components/**/*.{js,ts,jsx,tsx,mdx}",
    "./app/**/*.{js,ts,jsx,tsx,mdx}",
  ],
  theme: {
    extend: {
      colors: {
        terminal: {
          bg: "#0a0a0a",
          surface: "#1a1a1a",
          border: "#2a2a2a",
          accent: "#3a3a3a",
          text: "#e0e0e0",
          muted: "#888888",
          success: "#4ade80",
          error: "#f87171",
          warning: "#fbbf24",
          info: "#60a5fa",
        },
      },
      fontFamily: {
        mono: [
          "NerdFontsSymbols Nerd Font",
          "JetBrains Mono",
          "Fira Code",
          "ui-monospace",
          "SFMono-Regular",
          "Menlo",
          "Monaco",
          "Consolas",
          "monospace",
        ],
      },
      animation: {
        "pulse-slow": "pulse 3s cubic-bezier(0.4, 0, 0.6, 1) infinite",
      },
    },
  },
  plugins: [],
};

export default config;
