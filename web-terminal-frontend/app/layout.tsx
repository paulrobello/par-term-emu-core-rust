import type { Metadata, Viewport } from "next";
import "./globals.css";

const isDevelopment = process.env.NODE_ENV === 'development';

export const metadata: Metadata = {
  title: "Par Terminal Streamer | Modern Web Terminal",
  description: "A sleek, modern web-based terminal emulator with real-time streaming support. Built with Next.js, xterm.js, and Rust.",
  icons: {
    icon: [
      { url: "/favicon.ico", sizes: "any" },
      { url: "/favicon.png", type: "image/png", sizes: "32x32" },
      { url: "/icon-192.png", type: "image/png", sizes: "192x192" },
      { url: "/icon-512.png", type: "image/png", sizes: "512x512" },
    ],
    apple: [
      { url: "/apple-touch-icon.png", sizes: "180x180", type: "image/png" },
    ],
  },
  manifest: "/manifest.json",
};

export const viewport: Viewport = {
  width: "device-width",
  initialScale: 1,
  maximumScale: 1,
  viewportFit: "cover", // Support notch/safe areas on modern phones
  themeColor: "#0a0a0a",
};

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html lang="en">
      <head>
        {/* Terminal fonts are intentionally not fetched from any CDN
            (audit SEC-006): font-family names in globals.css and the xterm
            config act as opt-in hooks for locally installed fonts, with
            system monospace fallbacks everywhere. */}

        {/* External theme file - editable without rebuild */}
        {/* eslint-disable-next-line @next/next/no-css-tags -- intentionally
            NOT a Next.js-bundled asset: /theme.css is served from public/ so
            it can be edited on a deployed instance without a rebuild. */}
        <link rel="stylesheet" href="/theme.css" />

        {/* WebSocket preconnect hints - reduces initial connection latency by 100-200ms */}
        {/* Only in development to avoid browser port scanning localhost in production */}
        {isDevelopment && (
          <>
            <link rel="preconnect" href="ws://localhost:8099" />
            <link rel="preconnect" href="wss://localhost:8099" />
          </>
        )}
      </head>
      <body className="font-mono">
        {children}
      </body>
    </html>
  );
}
