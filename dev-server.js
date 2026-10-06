#!/usr/bin/env node
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
/**
 * 广学 · 本地开发服务器
 *
 * 为什么需要它：
 *   前端用绝对路径请求 /api/*（线上由 Nginx 反向代理到后端），
 *   所以不能直接双击 index.html 打开 —— file:// 协议下 /api 请求会 404，
 *   WASM 的 ES 模块加载也会被浏览器拦截。本脚本把「静态文件」与「API 代理」
 *   放在同一个 origin 下，等价于线上 Nginx 的形态：
 *       /                → 仓库根目录静态文件
 *       /api/auth/*      → http://127.0.0.1:8081（Rust 账号系统）
 *       /api/*           → http://127.0.0.1:8080（Go 主后端）
 *
 *   两个上游的划分与线上 Nginx 一致：账号系统是独立的 Rust 服务。
 *
 * 用法（在仓库根目录执行）：
 *   node dev-server.js                          # 默认 http://127.0.0.1:8899
 *   node dev-server.js --port 9000              # 换前端端口
 *   node dev-server.js --api-port 8081          # Go 后端换了端口
 *   node dev-server.js --auth-port 8082         # 账号服务换了端口
 *   PORT=9000 API_PORT=8081 AUTH_PORT=8082 node dev-server.js  # 也可用环境变量
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
// 账号系统（Rust 认证服务）：只接管 /api/auth/* 前缀
const AUTH_PORT = Number(argValue('--auth-port') || process.env.AUTH_PORT || 8081);
const AUTH_HOST = argValue('--auth-host') || process.env.AUTH_HOST_PROXY || '127.0.0.1';

// 两个上游：前缀 → 目标
const AUTH_PREFIX = '/api/auth/';
const UPSTREAM_AUTH = {
  name: '账号系统(Rust)',
  host: AUTH_HOST,
  port: AUTH_PORT,
  hint: `无法连接账号服务 ${AUTH_HOST}:${AUTH_PORT}，请在 backend-rust 目录执行 cargo run --release`
};
const UPSTREAM_API = {
  name: '主后端(Go)',
  host: API_HOST,
  port: API_PORT,
  hint: `无法连接后端 ${API_HOST}:${API_PORT}，请先在 backend-go 目录执行 go run main.go`
};

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

// 静态资源的缓存策略
//
// 开发期要同时满足两件事：改了代码马上生效、浏览器又能真的缓存住（否则每次导航
// 都要把整站重新下载一遍，个人中心那张 2.6MB 头像也是每次重下，首页的预取更是
// 永远命中不了缓存）。所以按「这类文件会不会边写边看」分开：
//   · 代码类（html / js / css / json）：no-cache + ETag —— 每次带条件请求来问，
//     没改回 304（不传正文，几毫秒），改了立刻就是新文件。
//   · 图片 / 字体 / wasm（不会边改边刷新）：直接给一天强缓存。
const CODE_EXT = new Set(['.html', '.js', '.mjs', '.css', '.json', '.txt', '.map']);

// 弱 ETag：文件大小 + 修改时间（毫秒），够本地开发用了
function etagOf(stat) {
  return 'W/"' + stat.size.toString(16) + '-' + Math.floor(stat.mtimeMs).toString(16) + '"';
}

// 统一响应出口。默认仍是 no-store（接口代理等动态内容），静态文件会显式传 headers
function send(res, code, type, body, headers) {
  res.writeHead(code, Object.assign({
    'Content-Type': type,
    'Cache-Control': 'no-store, must-revalidate'
  }, headers || {}));
  res.end(body);
}

// /api/* → 后端反向代理（按前缀选上游）
function proxy(req, res, target) {
  const upstream = http.request(
    {
      host: target.host,
      port: target.port,
      path: req.url,
      method: req.method,
      headers: req.headers
    },
    (upRes) => {
      // 注意：Set-Cookie 可能是数组（登录会一次下发两个 Cookie），
      // writeHead 直接透传 headers 即可，不要自己拼字符串
      res.writeHead(upRes.statusCode, upRes.headers);
      upRes.pipe(res);
    }
  );

  upstream.on('error', (err) => {
    // 后端没起来时给出明确提示，而不是让前端只报一个 fetch 失败
    const hint = err.code === 'ECONNREFUSED' ? target.hint : String(err.message || err);
    console.error('[proxy error] ' + req.method + ' ' + req.url + ' → ' + target.name + '：' + hint);
    send(res, 502, 'application/json; charset=utf-8',
      JSON.stringify({ code: 502, message: hint }));
  });

  // 后端提前断开时不要抛出未捕获异常
  res.on('close', () => upstream.destroy());
  req.pipe(upstream);
}

function serveStatic(req, res) {
  let rel = decodeURIComponent(req.url.split('?')[0]);
  if (rel === '/' || rel.endsWith('/')) rel += 'index.html';

  const filePath = path.join(ROOT, rel);
  // 阻止 ../ 越界访问仓库之外的文件
  if (!filePath.startsWith(ROOT)) {
    return send(res, 403, 'text/plain; charset=utf-8', '403 禁止访问');
  }

  const ext = path.extname(filePath).toLowerCase();
  const type = MIME[ext] || 'application/octet-stream';

  fs.stat(filePath, (statErr, stat) => {
    if (statErr || !stat.isFile()) {
      return send(res, 404, 'text/plain; charset=utf-8', '404 未找到: ' + rel);
    }

    const etag = etagOf(stat);
    const headers = {
      'ETag': etag,
      'Last-Modified': stat.mtime.toUTCString(),
      // 策略见文件上方「静态资源的缓存策略」注释
      'Cache-Control': CODE_EXT.has(ext) ? 'no-cache' : 'public, max-age=86400'
    };

    // 条件请求命中：回 304，正文一点都不传（这就是「缓存住 + 不霉」的关键）
    if (req.headers['if-none-match'] === etag) {
      res.writeHead(304, headers);
      return res.end();
    }

    fs.readFile(filePath, (err, buf) => {
      if (err) {
        return send(res, 404, 'text/plain; charset=utf-8', '404 未找到: ' + rel);
      }
      send(res, 200, type, buf, headers);
    });
  });
}

const server = http.createServer((req, res) => {
  const path = req.url.split('?')[0];
  if (path.startsWith(AUTH_PREFIX) || path === '/api/auth') {
    // 账号系统（Rust）：与线上 Nginx 的 location /api/auth/ 分流一致
    return proxy(req, res, UPSTREAM_AUTH);
  }
  if (path.startsWith('/api/')) {
    return proxy(req, res, UPSTREAM_API);
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
  console.log('  /api/auth/* → http://' + AUTH_HOST + ':' + AUTH_PORT + '  (账号系统 Rust)');
  console.log('  /api/*      → http://' + API_HOST + ':' + API_PORT + '  (主后端 Go)');
  console.log('  若页面提示复习功能加载失败: 确认 Go 后端已启动（cd backend-go && go run main.go）');
  console.log('  若登录/注册报 502: 确认账号服务已启动（cd backend-rust && cargo run --release）');
});
