#!/usr/bin/env node
/**
 * 广学 · 本地开发服务器
 *
 * 为什么需要它：
 *   前端用绝对路径请求 /api/*（线上由 Nginx 反向代理到 Go 后端），
 *   所以不能直接双击 index.html 打开 —— file:// 协议下 /api 请求会 404，
 *   WASM 的 ES 模块加载也会被浏览器拦截。本脚本把「静态文件」与「API 代理」
 *   放在同一个 origin 下，等价于线上 Nginx 的形态：
 *       /            → 仓库根目录静态文件
 *       /api/*       → http://127.0.0.1:8080
 *
 * 用法（在仓库根目录执行）：
 *   node dev-server.js                          # 默认 http://127.0.0.1:8899
 *   node dev-server.js --port 9000              # 换前端端口
 *   node dev-server.js --api-port 8081          # 后端换了端口
 *   PORT=9000 API_PORT=8081 node dev-server.js  # 也可用环境变量
 *
 * 依赖：仅使用 Node 内置模块（http / fs / path），无需 npm install。
 * 注意：本文件仅用于本地开发，部署时不需要上传。
 */
'use strict';

const http = require('http');
const fs = require('fs');
const path = require('path');

// 静态文件根目录 = 本脚本所在目录（仓库根目录）
const ROOT = path.resolve(__dirname);
const HOST = '127.0.0.1';

// 读取命令行参数（--key value 形式），没有则回退到环境变量与默认值
function argValue(name) {
  const i = process.argv.indexOf(name);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : null;
}

const PORT = Number(argValue('--port') || process.env.PORT || 8899);
const API_PORT = Number(argValue('--api-port') || process.env.API_PORT || 8080);
const API_HOST = argValue('--api-host') || process.env.API_HOST || '127.0.0.1';

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  // WASM 必须是 application/wasm，否则浏览器会退化为较慢的 instantiate 路径
  '.wasm': 'application/wasm',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.gif': 'image/gif',
  '.svg': 'image/svg+xml',
  '.webp': 'image/webp',
  '.ico': 'image/x-icon',
  '.woff2': 'font/woff2',
  '.txt': 'text/plain; charset=utf-8'
};

// 统一响应出口：开发期一律 no-store，避免「改了代码浏览器还跑旧文件」
function send(res, code, type, body) {
  res.writeHead(code, {
    'Content-Type': type,
    'Cache-Control': 'no-store, must-revalidate'
  });
  res.end(body);
}

// /api/* → Go 后端反向代理
function proxy(req, res) {
  const upstream = http.request(
    {
      host: API_HOST,
      port: API_PORT,
      path: req.url,
      method: req.method,
      headers: req.headers
    },
    (upRes) => {
      res.writeHead(upRes.statusCode, upRes.headers);
      upRes.pipe(res);
    }
  );

  upstream.on('error', (err) => {
    // 后端没起来时给出明确提示，而不是让前端只报一个 fetch 失败
    const hint = err.code === 'ECONNREFUSED'
      ? `无法连接后端 ${API_HOST}:${API_PORT}，请先在 backend-go 目录执行 go run main.go`
      : String(err.message || err);
    console.error('[proxy error] ' + req.method + ' ' + req.url + ' → ' + hint);
    send(res, 502, 'application/json; charset=utf-8',
      JSON.stringify({ code: 502, message: hint }));
  });

  // 后端提前断开时不要抛出未捕获异常
  res.on('close', () => upstream.destroy());
  req.pipe(upstream);
}

// 静态文件
function serveStatic(req, res) {
  let rel = decodeURIComponent(req.url.split('?')[0]);
  if (rel === '/' || rel.endsWith('/')) rel += 'index.html';

  const filePath = path.join(ROOT, rel);
  // 阻止 ../ 越界访问仓库之外的文件
  if (!filePath.startsWith(ROOT)) {
    return send(res, 403, 'text/plain; charset=utf-8', '403 禁止访问');
  }

  fs.readFile(filePath, (err, buf) => {
    if (err) {
      return send(res, 404, 'text/plain; charset=utf-8', '404 未找到: ' + rel);
    }
    const type = MIME[path.extname(filePath).toLowerCase()] || 'application/octet-stream';
    send(res, 200, type, buf);
  });
}

const server = http.createServer((req, res) => {
  if (req.url.split('?')[0].startsWith('/api/')) {
    return proxy(req, res);
  }
  serveStatic(req, res);
});

server.on('error', (err) => {
  if (err.code === 'EADDRINUSE') {
    console.error(`端口 ${PORT} 已被占用，请换一个：node dev-server.js --port 9000`);
    process.exit(1);
  }
  throw err;
});

server.listen(PORT, HOST, () => {
  console.log('广学本地开发服务器已启动');
  console.log('  前端页面: http://' + HOST + ':' + PORT + '/');
  console.log('  静态根目录: ' + ROOT);
  console.log('  API 代理: /api/* → http://' + API_HOST + ':' + API_PORT);
  console.log('  若页面提示复习功能加载失败，请确认后端已启动（cd backend-go && go run main.go）');
});
