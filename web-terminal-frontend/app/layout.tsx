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
        {/* Font preconnects for faster font loading */}
        <link rel="preconnect" href="https://fonts.googleapis.com" />
        <link rel="preconnect" href="https://fonts.gstatic.com" crossOrigin="anonymous" />

        {/* WebSocket preconnect hints - reduces initial connection latency by 100-200ms */}
        {/* Only in development to avoid browser port scanning localhost in production */}
        {isDevelopment && (
          <>
            <link rel="preconnect" href="ws://localhost:8099" />
            <link rel="preconnect" href="wss://localhost:8099" />
          </>
        )}

        {/* Preload terminal fonts to avoid layout shift and font flash */}
        <link
          rel="preload"
          href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;700&display=swap"
          as="style"
        />
        <link
          href="https://fonts.googleapis.com/css2?family=JetBrains+Mono:wght@400;700&display=swap"
          rel="stylesheet"
        />
      </head>
      <body className="font-mono">
        {children}
      </body>
    </html>
  );
}
