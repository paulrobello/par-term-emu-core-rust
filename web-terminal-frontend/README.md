# Terminal Streaming Web Frontend

A modern, sleek web-based terminal emulator built with Next.js, TypeScript, Tailwind CSS, and xterm.js.

## Features

- 🚀 **Modern Stack**: Built with Next.js 14, React 18, and TypeScript
- 🎨 **Beautiful UI**: Sleek design with Tailwind CSS and glass morphism effects
- 💻 **Full Terminal Emulation**: Powered by xterm.js with WebGL rendering
- 🔌 **WebSocket Streaming**: Real-time bidirectional communication
- 🎯 **Nerd Fonts Support**: Full support for file/folder icons and symbols
- 📱 **Responsive Design**: Works on desktop and mobile devices
- ⚡ **Performance**: WebGL rendering for smooth performance
- 🔗 **Link Detection**: Clickable URLs in terminal output
- 🌈 **Unicode Support**: Full Unicode 11 support including emojis
- 📲 **PWA Support**: Installable as a progressive web app with custom icons

## Getting Started

### Prerequisites

- Node.js 18+ or Bun
- A running terminal streaming server (from `par-term-emu-core-rust`)

### Installation

```bash
# Install dependencies with npm
npm install

# Or with yarn
yarn install

# Or with pnp
pnpm install

# Or with bun
bun install
```

### Development

```bash
# Start the development server
npm run dev

# Or with yarn/pnpm/bun
yarn dev
pnpm dev
bun dev
```

Open [http://localhost:3000](http://localhost:3000) in your browser.

### Production Build

```bash
# Build for production
npm run build

# Start production server
npm run start
```

## Configuration

### WebSocket URL

By default, the application connects to `ws://127.0.0.1:8080`. You can change this in the UI or by modifying the default value in `app/page.tsx`.

### Terminal Theme

The terminal emulator theme (ANSI colors, foreground, background) is sent from the streaming server. Use the `--theme` flag when running the server (e.g., `--theme dracula`).

### UI Theme

UI chrome colors are defined in `public/theme.css` as CSS custom properties. See [UI Color Theme](#ui-color-theme) section below.

## Project Structure

```
web-terminal-frontend/
├── app/
│   ├── globals.css       # Global styles and Tailwind directives
│   ├── layout.tsx        # Root layout with metadata and icons
│   └── page.tsx          # Main page component
├── components/
│   └── Terminal.tsx      # Terminal component with xterm.js
├── public/
│   ├── theme.css         # External UI theme (editable after build)
│   ├── favicon.ico       # Multi-size favicon (16x16, 32x32, 48x48)
│   ├── favicon.png       # 32x32 PNG favicon
│   ├── apple-touch-icon.png  # 180x180 Apple touch icon
│   ├── icon-192.png      # 192x192 PWA icon
│   ├── icon-512.png      # 512x512 PWA icon
│   ├── icon-1024.png     # Original high-res icon
│   └── manifest.json     # PWA manifest
├── next.config.js       # Next.js configuration
├── tailwind.config.ts   # Tailwind CSS configuration
├── tsconfig.json        # TypeScript configuration
└── package.json         # Dependencies and scripts
```

## Technologies Used

- **Next.js 14**: React framework with App Router
- **TypeScript**: Type-safe JavaScript
- **Tailwind CSS**: Utility-first CSS framework
- **xterm.js**: Terminal emulator for the browser
  - `@xterm/addon-fit`: Auto-sizing addon
  - `@xterm/addon-webgl`: WebGL renderer for performance
  - `@xterm/addon-web-links`: Clickable URL detection
  - `@xterm/addon-unicode11`: Unicode 11 support

## Customization

### Fonts

Fonts are loaded from CDN in `app/globals.css`:
- **JetBrains Mono**: Programming font from Google Fonts
- **Fira Code**: Alternative programming font
- **Nerd Fonts Symbols**: Icon glyphs for file/folder icons

To use local fonts, place them in `public/fonts/` and update the `@font-face` declarations.

### UI Color Theme

The UI chrome theme (status bar, buttons, containers) is defined in `public/theme.css`. This file is **not bundled** during build, so you can customize it after deployment without rebuilding:

```css
:root {
  --terminal-bg: #0a0a0a;      /* Main background */
  --terminal-surface: #1a1a1a; /* Surface/card background */
  --terminal-border: #2a2a2a;  /* Border color */
  --terminal-accent: #3a3a3a;  /* Accent color (scrollbar, etc.) */
  --terminal-text: #e0e0e0;    /* Primary text color */
}
```

After running `make web-build-static`, edit `web_term/theme.css` to customize colors. Changes take effect on page refresh.

**Note:** Terminal emulator colors (ANSI palette, foreground/background) are controlled by the streaming server's `--theme` option, not this file.

### Terminal Options

Modify terminal options in `components/Terminal.tsx`:

```typescript
const term = new XTerm({
  fontSize: 14,
  fontFamily: '...',
  cursorBlink: false,
  // ... other options
});
```

## Troubleshooting

### Icons not showing

Make sure:
1. The Nerd Fonts CSS is loaded (check browser console)
2. The font family includes `'NerdFontsSymbols Nerd Font'` first
3. Your server is sending the correct icon characters

### WebSocket connection fails

Check that:
1. The streaming server is running on the specified URL
2. The WebSocket URL is correct (ws:// not wss:// for local)
3. No firewall is blocking the connection

### Fonts not loading

Ensure:
1. `document.fonts.ready` is being awaited
2. Font CDN URLs are accessible
3. Check browser console for font loading errors

## License

MIT

## Contributing

Contributions are welcome! Please open an issue or submit a PR.

## Related Projects

- [par-term-emu-core-rust](https://github.com/paulrobello/par-term-emu-core-rust) - The terminal emulator core and streaming server
- [xterm.js](https://github.com/xtermjs/xterm.js) - The terminal emulator library
