# Web Terminal Frontend

Static web assets for the PAR Terminal Emulator streaming server.

## Overview

This directory contains the pre-built Next.js static export that provides the web-based terminal interface for the PAR Terminal Emulator streaming server.

## Usage with Streaming Server

### Option 1: Extract and Serve

1. Download the web frontend archive from the [GitHub releases](https://github.com/paulrobello/par-term-emu-core-rust/releases):
   - `par-term-web-frontend-vX.X.X.tar.gz` (Linux/macOS)
   - `par-term-web-frontend-vX.X.X.zip` (Windows)

2. Extract to a directory:
   ```bash
   # Linux/macOS
   tar -xzf par-term-web-frontend-vX.X.X.tar.gz -C /path/to/web_term

   # Windows
   unzip par-term-web-frontend-vX.X.X.zip -d C:\path\to\web_term
   ```

3. Run the streaming server with the web root:
   ```bash
   par-term-streamer --web-root /path/to/web_term
   ```

### Option 2: Repository Checkout

If you've cloned the repository, the `web_term` directory is already present:

```bash
par-term-streamer --web-root ./web_term
```

## Directory Structure

```
web_term/
├── index.html              # Main entry point
├── _next/                  # Next.js static assets
│   ├── static/            # Static resources
│   └── ...
├── favicon.ico            # Browser icon
├── manifest.json          # PWA manifest
└── *.png                  # App icons
```

## Configuration

The web terminal connects to the streaming server via WebSocket. The connection URL is automatically configured based on the page URL:

- HTTP page → `ws://` WebSocket
- HTTPS page → `wss://` WebSocket
- Port is inherited from the page URL

## Browser Compatibility

- Chrome/Edge: ✅ Full support
- Firefox: ✅ Full support
- Safari: ✅ Full support
- Mobile browsers: ✅ Touch-optimized

## Development

To rebuild the web frontend from source:

```bash
cd web-terminal-frontend
npm install
npm run build
```

The static export will be generated in `web-terminal-frontend/out/`, which should be copied to `web_term/`.

## Versioning

The web frontend version matches the par-term-emu-core-rust package version. Always use matching versions of the web frontend and streaming server binary for best compatibility.

## License

Same as the parent project - see the repository LICENSE file.
