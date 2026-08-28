// Deterministic build ID: Next.js generates a random one per build, which
// leaks into exported HTML/RSC payloads and makes rebuilds of identical
// source differ (breaks the web_term/ CI drift gate). The package version
// is deterministic; content changes still bust caches via hashed chunk names.
const pkg = require('./package.json')

/** @type {import('next').NextConfig} */
const nextConfig = {
  generateBuildId: () => pkg.version,
  // Enable React Strict Mode for better development experience
  reactStrictMode: true,
  // Static export for serving from Rust streaming server
  output: 'export',
  // Disable image optimization for static export
  images: {
    unoptimized: true,
  },
}

module.exports = nextConfig
