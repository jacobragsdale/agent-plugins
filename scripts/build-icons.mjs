// Generates every icon asset from one geometry definition.
//
//   pnpm icons
//
// Writes assets/icon/*.svg (the readable sources), assets/icon/app-icon.png
// (the 1024px master that `tauri icon` expands into the platform set),
// src-tauri/icons/tray.png (the macOS menu bar template image), and the web
// portal's website/public/icon.svg and og.png.

import { Resvg } from "@resvg/resvg-js";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const assets = join(root, "assets", "icon");
const trayOut = join(root, "src-tauri", "icons", "tray.png");
const webPublic = join(root, "website", "public");

// The mark: a chat bubble (the agent) with plug prongs (a plugin). Drawn on
// the 1024 unit app icon grid.
const PRONGS = `<rect x="396" y="176" width="64" height="200" rx="32"/><rect x="564" y="176" width="64" height="200" rx="32"/>`;
const BUBBLE = `<path d="M372 330 H652 A140 140 0 0 1 792 470 V630 A140 140 0 0 1 652 770 H470 L322 862 L346 770 A140 140 0 0 1 232 630 V470 A140 140 0 0 1 372 330Z"/>`;

// Radix blue 9 to blue 11, the app's own accent (main.tsx accentColor="blue").
const BLUE = "#0090ff";
const DEEP_BLUE = "#0d74ce";

const APP_SIZE = 1024;
const APP_CORNER = 224;

const appIconSvg = `<svg xmlns="http://www.w3.org/2000/svg" width="${APP_SIZE}" height="${APP_SIZE}" viewBox="0 0 ${APP_SIZE} ${APP_SIZE}">
  <defs>
    <linearGradient id="tile" x1="0" y1="0" x2="0.35" y2="1">
      <stop offset="0" stop-color="${BLUE}"/>
      <stop offset="1" stop-color="${DEEP_BLUE}"/>
    </linearGradient>
  </defs>
  <rect width="${APP_SIZE}" height="${APP_SIZE}" rx="${APP_CORNER}" fill="url(#tile)"/>
  <g fill="#ffffff">${PRONGS}${BUBBLE}</g>
</svg>
`;

// The tray image is a silhouette: macOS template mode throws away colour and
// keeps only the alpha channel. The tray shows the plug bubble framed square
// around its bounds.
const TRAY_PX = 44;
const TRAY_VIEW = "132 139 760 760";

const traySvg = `<svg xmlns="http://www.w3.org/2000/svg" width="${TRAY_PX}" height="${TRAY_PX}" viewBox="${TRAY_VIEW}">
  <g fill="#000000">${PRONGS}${BUBBLE}</g>
</svg>
`;

// Link preview card for the web portal: the icon and the name on a pale
// ground. Text uses whatever system sans resvg finds.
const OG_W = 1200;
const OG_H = 630;
const OG_FONT = "Segoe UI, Inter, Helvetica, Arial, Adwaita Sans, FreeSans, sans-serif";
const ogSvg = `<svg xmlns="http://www.w3.org/2000/svg" width="${OG_W}" height="${OG_H}" viewBox="0 0 ${OG_W} ${OG_H}">
  <rect width="${OG_W}" height="${OG_H}" fill="#f3f7fa"/>
  <svg x="96" y="175" width="280" height="280" viewBox="0 0 ${APP_SIZE} ${APP_SIZE}">${appIconSvg.replace(/^<svg[^>]*>|<\/svg>\s*$/g, "")}</svg>
  <text x="430" y="310" font-family="${OG_FONT}" font-size="92" font-weight="700" fill="#10212e">Agent Plugins</text>
  <text x="432" y="380" font-family="${OG_FONT}" font-size="36" fill="${DEEP_BLUE}">Skills and MCP servers for your AI apps</text>
</svg>`;

function png(svg, width) {
  return new Resvg(svg, { fitTo: { mode: "width", value: width } }).render().asPng();
}

mkdirSync(assets, { recursive: true });
writeFileSync(join(assets, "app-icon.svg"), appIconSvg);
writeFileSync(join(assets, "tray.svg"), traySvg);
writeFileSync(join(assets, "app-icon.png"), png(appIconSvg, APP_SIZE));
writeFileSync(trayOut, png(traySvg, TRAY_PX));
writeFileSync(join(webPublic, "icon.svg"), appIconSvg);
writeFileSync(join(webPublic, "og.png"), png(ogSvg, OG_W));

console.log(`assets/icon/app-icon.png (${APP_SIZE}px)`);
console.log(`src-tauri/icons/tray.png (${TRAY_PX}px template)`);
console.log("website/public/icon.svg, website/public/og.png");
