// 把 huzc 编出的 huzi-lsp 二进制同步到本插件 server/ 目录，供 vsce 打包。
// 用法：npm run sync-server [--debug]
// 默认取 release，--debug 取 debug。两目录是相对路径，不写死盘符。
const fs = require('fs');
const path = require('path');

const useDebug = process.argv.includes('--debug');
const profile = useDebug ? 'debug' : 'release';
const hzRoot = path.resolve(__dirname, '..', '..', '..', 'huzc');
const serverDir = path.resolve(__dirname, '..', 'server');

const candidates =
  process.platform === 'win32'
    ? ['huzi-lsp.exe']
    : ['huzi-lsp', 'huzi-lsp.exe'];

fs.mkdirSync(serverDir, { recursive: true });

let copied = null;
for (const name of candidates) {
  const src = path.join(hzRoot, 'target', profile, name);
  if (fs.existsSync(src)) {
    const dest = path.join(serverDir, name);
    fs.copyFileSync(src, dest);
    copied = `${src} -> ${dest}`;
    break;
  }
}

if (!copied) {
  console.error(
    `[sync-server] not found: ${candidates
      .map((n) => path.join(hzRoot, 'target', profile, n))
      .join(' / ')}\n` +
      `先跑: cargo build ${useDebug ? '' : '--release '} -p huzi-lsp（在 huzc 目录）`,
  );
  process.exit(1);
}
console.log(`[sync-server] ${copied}`);
