import type { Config } from "tailwindcss";

// Tailwind v4 uses CSS-based configuration via @theme directive
// This config file is primarily for content paths and plugins
const config: Config = {
  content: [
    "./pages/**/*.{js,ts,jsx,tsx,mdx}",
    "./components/**/*.{js,ts,jsx,tsx,mdx}",
    "./app/**/*.{js,ts,jsx,tsx,mdx}",
  ],
  // Theme configuration moved to globals.css @theme directive
  plugins: [],
};

export default config;
