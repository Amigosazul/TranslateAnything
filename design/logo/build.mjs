// 随译 Logo 生成脚本：改字形或配色后运行 `node design/logo/build.mjs` 重新生成 SVG；
// 加 `--apply` 同时替换 src-tauri/icons 下的全套应用图标、icon.ico 小尺寸和托盘图标。
// 字形"文中藏 A"：点、横、上半交叉属于"文"；交叉点以下两条腿加一道横就是"A"。
// 扫描线横穿交叉点：线上是已译好的"文"（蓝），线下是还没翻译的原文"A"（灰）。

import { execSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const OUT = dirname(fileURLToPath(import.meta.url));

// 字形，画在 100×100 的格子里
const GLYPH = [
  'M50 3V13',                                            // 点
  'M10 25H90',                                           // 横
  'M73 34C68 44 60 52 50 58C40 52 32 44 27 34',          // 交叉的上半（V）
  'M50 58C38 68 26 82 13 96M50 58C62 68 74 82 87 96',    // 交叉的下半（A 的两条腿）
  'M29 80H71',                                           // A 的横
].join('');
const CROSS_Y = 58; // 交叉点高度，扫描线从这里穿过

const COLOR = {
  blue1: '#3d7bff', blue2: '#27c6ec',   // 强调色渐变（与设置界面主色 #2f8cff 同色系）
  gray: '#a3adc2',                      // 原文"A"
  tile1: '#ffffff', tile2: '#edf2fb',   // 白底板渐变
  border: '#1e3a78',
};

const TILE = 'M64 8H192A56 56 0 0 1 248 64V192A56 56 0 0 1 192 248H64A56 56 0 0 1 8 192V64A56 56 0 0 1 64 8Z';

const r = (n) => +n.toFixed(2);

/**
 * 字形 + 扫描线
 * size: 字形边长；sw: 笔画粗细；gap: 扫描线上下留缝；ext: 扫描线伸出字形的长度（格子单位）
 * top / bottom / line: 三部分的颜色
 */
function mark({ cx = 128, cy = 128, size, sw, gap, ext, lineW, top, bottom, bottomOpacity = 1, line }) {
  const k = size / 100, x0 = cx - size / 2, y0 = cy - size / 2;
  const y = y0 + CROSS_Y * k;
  const glyph = (stroke, extra = '') =>
    `<path transform="translate(${r(x0)} ${r(y0)}) scale(${r(k)})" d="${GLYPH}" fill="none" stroke="${stroke}" stroke-width="${r(sw / k)}" stroke-linecap="round" stroke-linejoin="round"${extra}/>`;
  return `
  <clipPath id="top"><rect x="0" y="0" width="256" height="${r(y - gap)}"/></clipPath>
  <clipPath id="bottom"><rect x="0" y="${r(y + gap)}" width="256" height="256"/></clipPath>
  <g clip-path="url(#top)">${glyph(top)}</g>
  <g clip-path="url(#bottom)">${glyph(bottom, bottomOpacity < 1 ? ` stroke-opacity="${bottomOpacity}"` : '')}</g>
  <path d="M${r(x0 - ext * k)} ${r(y)}H${r(x0 + (100 + ext) * k)}" stroke="${line}" stroke-width="${r(lineW)}" stroke-linecap="round"/>`;
}

const accentDef = (x1, y1, x2, y2) => `
  <linearGradient id="acc" gradientUnits="userSpaceOnUse" x1="${x1}" y1="${y1}" x2="${x2}" y2="${y2}">
    <stop offset="0" stop-color="${COLOR.blue1}"/><stop offset="1" stop-color="${COLOR.blue2}"/>
  </linearGradient>`;

const svg = (viewBox, body) =>
  `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${viewBox}" width="1024" height="1024">${body}\n</svg>\n`;

// 应用主图标：白底圆角方块
const ICON_MARK = { size: 136, sw: 15, gap: 5, ext: 10, lineW: 5.7 };
const icon = svg('0 0 256 256', `
  <defs>
  <linearGradient id="tile" gradientUnits="userSpaceOnUse" x1="8" y1="8" x2="8" y2="248">
    <stop offset="0" stop-color="${COLOR.tile1}"/><stop offset="1" stop-color="${COLOR.tile2}"/>
  </linearGradient>${accentDef(40, 40, 216, 216)}
  </defs>
  <path d="${TILE}" fill="url(#tile)"/>
  <path d="${TILE}" fill="none" stroke="${COLOR.border}" stroke-opacity=".10" stroke-width="2"/>
  ${mark({ ...ICON_MARK, top: 'url(#acc)', bottom: COLOR.gray, line: 'url(#acc)' })}`);

// 亮色底备用版：蓝青渐变底 + 白色图形
const iconBright = svg('0 0 256 256', `
  <defs>${accentDef(40, 40, 216, 216)}</defs>
  <path d="${TILE}" fill="url(#acc)"/>
  <path d="${TILE}" fill="none" stroke="#fff" stroke-opacity=".25" stroke-width="2"/>
  ${mark({ ...ICON_MARK, top: '#fff', bottom: '#fff', bottomOpacity: 0.5, line: '#fff' })}`);

// 纯标志（透明底），用于网页、文档、与文字组合
const markOnly = svg('36 36 184 184', `
  <defs>${accentDef(40, 40, 216, 216)}</defs>
  ${mark({ ...ICON_MARK, top: 'url(#acc)', bottom: COLOR.gray, line: 'url(#acc)' })}`);

// 小尺寸版（16～40px，打进 icon.ico 和 32x32.png）：字形放大、笔画加粗、改用实色，
// 底板描边加深，浅色任务栏和白色标题栏上也能看出轮廓
const iconSmall = svg('0 0 256 256', `
  <defs>
  <linearGradient id="tile" gradientUnits="userSpaceOnUse" x1="8" y1="8" x2="8" y2="248">
    <stop offset="0" stop-color="${COLOR.tile1}"/><stop offset="1" stop-color="${COLOR.tile2}"/>
  </linearGradient>
  </defs>
  <path d="${TILE}" fill="url(#tile)"/>
  <path d="${TILE}" fill="none" stroke="${COLOR.border}" stroke-opacity=".22" stroke-width="5"/>
  ${mark({ size: 168, sw: 25, gap: 6, ext: 4, lineW: 10, top: '#2f6cf6', bottom: '#7f8aa3', line: '#2f6cf6' })}`);

// 托盘图标：透明底、笔画加粗、扫描线收短，16～24px 下也能看清；
// 灰色调深一点，深色和浅色任务栏上都可见
const TRAY = { size: 200, sw: 26, gap: 6, ext: 3, lineW: 11 };
const tray = svg('20 20 216 216', `
  ${mark({ ...TRAY, top: COLOR.blue1, bottom: '#8792aa', line: COLOR.blue1 })}`);

const files = {
  'icon.svg': icon, 'icon-small.svg': iconSmall, 'icon-bright.svg': iconBright, 'mark.svg': markOnly, 'tray.svg': tray,
};
for (const [name, content] of Object.entries(files)) {
  writeFileSync(join(OUT, name), content);
  console.log('已生成', name);
}

// ===== --apply：生成并替换 src-tauri/icons 下的应用图标 =====
// 大尺寸用 icon.svg，16～40px 用 icon-small.svg，两者拼进同一个 icon.ico；托盘用 tray.svg。
if (process.argv.includes('--apply')) {
  const ROOT = join(OUT, '..', '..');
  const ICONS = join(ROOT, 'src-tauri', 'icons');
  const TMP = mkdtempSync(join(tmpdir(), 'ta-logo-'));
  const run = (cmd) => execSync(cmd, { cwd: ROOT, stdio: ['ignore', 'ignore', 'inherit'], shell: true });
  const pngs = (svgName, sizes) => {
    const dir = join(TMP, svgName);
    run(`pnpm tauri icon "${join(OUT, svgName)}" -o "${dir}" ${sizes.map((s) => `-p ${s}`).join(' ')}`);
    return Object.fromEntries(sizes.map((s) => [s, readFileSync(join(dir, `${s}x${s}.png`))]));
  };

  // 1. 全套图标（各平台 png / icns / ico）
  run(`pnpm tauri icon "${join(OUT, 'icon.svg')}"`);
  for (const extra of ['android', 'ios', '64x64.png']) rmSync(join(ICONS, extra), { recursive: true, force: true });

  // 2. icon.ico：小尺寸换成加粗版
  const small = pngs('icon-small.svg', [16, 20, 24, 30, 32, 40]);
  const large = pngs('icon.svg', [48, 64, 128, 256]);
  const entries = [16, 20, 24, 32, 40].map((s) => [s, small[s]]).concat([48, 64, 128, 256].map((s) => [s, large[s]]));
  writeFileSync(join(ICONS, 'icon.ico'), packIco(entries));
  writeFileSync(join(ICONS, '32x32.png'), small[32]);
  writeFileSync(join(ICONS, 'Square30x30Logo.png'), small[30]);

  // 3. 托盘
  writeFileSync(join(ICONS, 'tray.png'), pngs('tray.svg', [32])[32]);

  rmSync(TMP, { recursive: true, force: true });
  console.log('已更新 src-tauri/icons（icon.ico 内含尺寸：', entries.map((e) => e[0]).join('/'), '）');
}

/** 把若干 PNG 拼成 .ico（PNG 压缩格式，Windows Vista 起支持） */
function packIco(entries) {
  const head = Buffer.alloc(6 + 16 * entries.length);
  head.writeUInt16LE(0, 0);
  head.writeUInt16LE(1, 2);
  head.writeUInt16LE(entries.length, 4);
  let offset = head.length;
  entries.forEach(([size, png], i) => {
    const o = 6 + 16 * i;
    head.writeUInt8(size >= 256 ? 0 : size, o);
    head.writeUInt8(size >= 256 ? 0 : size, o + 1);
    head.writeUInt16LE(1, o + 4);   // 色彩平面
    head.writeUInt16LE(32, o + 6);  // 位深
    head.writeUInt32LE(png.length, o + 8);
    head.writeUInt32LE(offset, o + 12);
    offset += png.length;
  });
  return Buffer.concat([head, ...entries.map((e) => e[1])]);
}
