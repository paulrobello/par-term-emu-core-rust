/** @type {import('next').NextConfig} */
const nextConfig = {
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
