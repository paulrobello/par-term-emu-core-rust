/** @type {import('next').NextConfig} */
const nextConfig = {
  output: 'export',
  // Disable image optimization for static export
  images: {
    unoptimized: true,
  },
  // Optional: Change the output directory (default is 'out')
  // distDir: 'dist',
}

module.exports = nextConfig
