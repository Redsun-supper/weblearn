// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
// 浏览器端到端检查（由 scripts/verify-pool.ps1 -Browser 调用）
//
// 前置：Edge 已带 --remote-debugging-port=<port> 起好；Go(8080)/Rust(8081)/dev-server(8899) 都在跑；
//       开发库的进度已被 verify-pool.ps1 清空。
//
// 检查的事（对照 docs/review-pool-plan.md 第 7 节，浏览器侧那部分）：
//   1. 登录 → 打开 SPA（**必须走 /index.html**，直接开学科片段没有 main.js）→ 点开始复习
//   2. 卡片正常出现（新引擎 + 新接口链路通）
//   3. 顶栏三个数字 = 今日已复习 / 池内到期 / 池内总数
//   4. 连续评分：左下角「今日置顶 N/5」逐步涨到 5/5，然后变成「整池已过一遍，继续复习不受限」
//   5. 每评一张都真的落了库（服务端 today_reviewed 跟着涨）
//   6. 控制台 0 error、0 未捕获异常
//   7. localStorage 里不再写 reviewDailyPlan
//
// 用法：node scripts/browser-e2e.mjs --port 9333 --shot <可选截图路径>
// 输出：最后一行是 JSON（供 PowerShell 侧判定），中间是给人看的日志。

const argv = process.argv.slice(2);
const argOf = (name, fallback) => {
  const i = argv.indexOf(name);
  return i >= 0 && argv[i + 1] ? argv[i + 1] : fallback;
};
const PORT = argOf('--port', '9333');
const SHOT = argOf('--shot', '');
const BASE = 'http://127.0.0.1:8899';
const EMAIL = '2262997289@qq.com';
const PASSWORD = '7289HR_RedSun';
const DAILY_TARGET = 5;

const results = [];
const record = (name, ok, detail) => {
  results.push({ name, ok, detail });
  console.log(`  [${ok ? 'PASS' : 'FAIL'}] ${name}  ${detail}`);
};

// ---------- 连 CDP ----------
const list = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
const page = list.find((p) => p.type === 'page');
if (!page) {
  console.error('找不到可用的标签页，Edge 是否带 --remote-debugging-port 启动？');
  process.exit(2);
}
const ws = new WebSocket(page.webSocketDebuggerUrl);
let id = 0;
const pending = new Map();
const events = [];
const send = (method, params = {}) => new Promise((resolve) => {
  const msgId = ++id;
  pending.set(msgId, resolve);
  ws.send(JSON.stringify({ id: msgId, method, params }));
});
ws.addEventListener('message', (ev) => {
  const msg = JSON.parse(ev.data);
  if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg.result); pending.delete(msg.id); return; }
  if (msg.method) events.push(msg);
});
await new Promise((r) => ws.addEventListener('open', r));

const evalJs = async (expression) => {
  const r = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  if (r.exceptionDetails) return { __error: r.exceptionDetails.exception?.description || r.exceptionDetails.text };
  return r.result.value;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

await send('Page.enable');
await send('Runtime.enable');
await send('Network.enable');
// 记录复习相关请求：诊断「界面上的今日置顶 N/5」与服务端 daily_done 对不上时必须看这个 ——
// 有一回基线是 2 而服务端明明是 0，就是靠抓 ?today= 与响应体检出来的。
const netLog = [];
ws.addEventListener('message', (ev) => {
  const m = JSON.parse(ev.data);
  if (m.method === 'Network.responseReceived' && /\/api\/reviews\//.test(m.params.response.url)) {
    netLog.push({ url: m.params.response.url, status: m.params.response.status });
  }
});
await send('Log.enable');
await send('Network.setCacheDisabled', { cacheDisabled: true });
await send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 800, deviceScaleFactor: 1, mobile: false });

// ---------- 1) 登录 ----------
await send('Page.navigate', { url: `${BASE}/index.html` });
await sleep(2500);
const login = await evalJs(`(async () => {
  const r = await fetch('/api/auth/login', {
    method: 'POST', credentials: 'include',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ email: ${JSON.stringify(EMAIL)}, password: ${JSON.stringify(PASSWORD)}, device_label: 'verify-pool-e2e' })
  });
  const j = await r.json();
  return { status: r.status, code: j.code };
})()`);
record('浏览器：登录管理员账号', login && login.status === 200, `HTTP ${login && login.status}`);

// ---------- 2) 打开英语页并开始复习 ----------
await send('Network.clearBrowserCache');
// 从空白页开始，避免上一轮的 DOM / 模块缓存影响
await send('Page.navigate', { url: `${BASE}/index.html` });
await sleep(3000);

const readUi = `(() => {
  const txt = (id) => { const e = document.getElementById(id); return e ? e.textContent.trim() : '(缺)' };
  const wordEl = document.querySelector('.study-word');
  return {
    word: wordEl ? wordEl.textContent.trim() : '',
    todayReviewed: txt('statTodayNew'),
    poolDue: txt('statTodayReview'),
    poolSize: txt('statRest'),
    planText: txt('studyPlanText'),
    status: txt('reviewStatus'),
    planCache: localStorage.getItem('reviewDailyPlan'),
    btn: txt('studyStartBtn')
  };
})()`;

const before = await evalJs(readUi);
record('浏览器：起始页有「开始复习单词」', !!(before && before.btn && before.btn.includes('开始复习')), `按钮=${before && before.btn}`);

const clicked = await evalJs(`(() => {
  const b = Array.from(document.querySelectorAll('button')).find(x => (x.textContent || '').includes('开始复习'));
  if (!b) return 'no-button';
  b.click();
  return 'clicked';
})()`);
await sleep(3500);

const ui = await evalJs(readUi);
record('浏览器：卡片正常渲染（新引擎生效）', !!(ui && ui.word && ui.word.length > 1), `单词=${ui && ui.word}`);
record('浏览器：顶栏池内总数 = 词表行数', !!(ui && ui.poolSize === '100'), `池内总数=${ui && ui.poolSize}`);
record('浏览器：左下角是「今日置顶 N/5」', !!(ui && /今日置顶 \d+\/5/.test(ui.planText)), `文案="${ui && ui.planText}"`);
record('浏览器：不再写 localStorage.reviewDailyPlan', !!(ui && ui.planCache === null), `reviewDailyPlan=${ui && ui.planCache}`);

// 起点可能不是 0：`state.dailyRated` 建会话时用服务端的 `daily_done` 初始化过
// （今天早些时候已经评过的置顶卡要算数），所以先读一次基线。
const baseText = ui && ui.planText ? ui.planText : '';
const baseDone = parseInt((baseText.match(/今日置顶 (\d+)/) || [])[1] || '0', 10);
record('浏览器：左下角显示「今日置顶 N/5」', /今日置顶 \d+\/5|整池已过一遍/.test(baseText), `文案="${baseText}"（基线 ${baseDone}/5）`);

const startProgress = baseDone;

// ---------- 3) 连续评分，直到置顶走完 ----------
//
// ⚠️ 为什么起点可能不是 0：`state.dailyRated` 建会话时用服务端的 `daily_done` 初始化
// （今天早些时候已经评过的置顶卡要算数）。所以这里断言的是**相对增量**，
// 而且必须真的把剩下的置顶卡都评完 —— 早先的临时脚本只评 2 张就下结论，
// 结果漏掉了「置顶卡被埋进池子中段、永远轮不到」这个真实 bug。
//
// 每张都记下「选了哪张、引擎报的来源、以及置顶标记」，失败时能一眼看出是被谁挤掉的。
// `planText` 是评分那一刻左下角的文案：把它和来源并排看，就能判断
// 「置顶卡到底有没有被抽到」与「计数器有没有跟上」这两件事分别对不对。
const ratedCards = [];
const rateOnce = async () => {
  const revealed = await evalJs(`(() => {
    const b = Array.from(document.querySelectorAll('button')).find(x => (x.textContent || '').includes('显示答案'));
    if (!b) return false;
    b.click(); return true;
  })()`);
  if (!revealed) return false;
  await sleep(500);
  // 评分**之前**读一次当前卡的状态（评分会换卡）
  const cardInfo = await evalJs(`(() => {
    const wordEl = document.querySelector('.study-word');
    const planEl = document.getElementById('studyPlanText');
    return { word: wordEl ? wordEl.textContent.trim() : '', plan: planEl ? planEl.textContent.trim() : '' };
  })()`);
  const rated = await evalJs(`(() => {
    const b = Array.from(document.querySelectorAll('button')).find(x => /一般|良好|简单|忘记/.test((x.textContent || '').trim()));
    if (!b) return false;
    b.click(); return true;
  })()`);
  await sleep(1600);
  if (rated) ratedCards.push(cardInfo);
  return rated;
};

let rated = 0;
let lastPlanText = '';
for (let i = 0; i < DAILY_TARGET + 2; i++) {
  if (!(await rateOnce())) break;
  rated++;
  lastPlanText = (await evalJs(readUi)).planText;
  const m = lastPlanText.match(/今日置顶 (\d+)/);
  const done = m ? parseInt(m[1], 10) : DAILY_TARGET;
  if (done >= DAILY_TARGET) break;
}

const final = await evalJs(readUi);
const finalDone = final.planText.match(/今日置顶 (\d+)/);
const reachedTarget = finalDone ? parseInt(finalDone[1], 10) >= DAILY_TARGET : /整池已过一遍/.test(final.planText);
record(`浏览器：连评 ${rated} 张后置顶进度涨到 5/5`, reachedTarget, `文案="${final.planText}"（起点 ${startProgress}/5）`);
console.log('  本轮评过的卡：');
for (const c of ratedCards) console.log(`    ${c.word.padEnd(14)} 评分前左下角="${c.plan}"`);
console.log(`  复习接口请求：${netLog.map((r) => `${r.status} ${r.url.replace(/^https?:\/\/[^/]+/, '')}`).join(' | ')}`);
record('浏览器：顶栏「今日已复习」跟着涨', !!(final && parseInt(final.todayReviewed, 10) >= startProgress + rated), `今日已复习=${final && final.todayReviewed}`);

// ---------- 4) 服务端口径 ----------
const stats = await evalJs(`(async () => {
  const d = new Date(); const m = d.getMonth() + 1, day = d.getDate();
  const today = '' + d.getFullYear() + (m < 10 ? '0' + m : m) + (day < 10 ? '0' + day : day);
  const r = await fetch('/api/reviews/stats?today=' + today, { credentials: 'include' });
  const j = await r.json();
  return { status: r.status, today_reviewed: j.data.today_reviewed, daily_done: j.data.daily_done, daily_target: j.data.daily_target, pool_size: j.data.pool_size, pool_due: j.data.pool_due, today_new: j.data.today_new };
})()`);
record('浏览器：服务端 today_reviewed 与本轮评分一致',
  !!(stats && stats.today_reviewed >= rated), `today_reviewed=${stats && stats.today_reviewed}（本轮评了 ${rated} 张）`);
record('浏览器：服务端 daily_done 达到 5', !!(stats && stats.daily_done >= DAILY_TARGET), `daily_done=${stats && stats.daily_done}/${stats && stats.daily_target}`);
record('浏览器：今日新学没有被重置重学冲高', !!(stats && stats.today_new <= rated), `today_new=${stats && stats.today_new} ≤ 本轮 ${rated} 张`);

// ---------- 5) 控制台 ----------
// ⚠️ 只算「应用自己的问题」，不算资源加载的噪音：
//   · chrome-error / favicon 之类由 Chrome 自己发的 console error，拿不到 URL
//   · 登录前的那次 /api/auth/me 401、favicon 404 都是**预期**的（先开门禁再登录）
//   · 4xx 响应本身用 Network 层按 URL 精确判定（见下面 badResponses）
const errs = events.filter((e) => e.method === 'Log.entryAdded' && e.params.entry.level === 'error')
  .map((e) => e.params.entry.text)
  .filter((t) => !/Failed to load resource/.test(t));
const exceptions = events.filter((e) => e.method === 'Runtime.exceptionThrown')
  .map((e) => (e.params.exceptionDetails.exception?.description || e.params.exceptionDetails.text || '').split('\n')[0]);
record('浏览器：控制台无应用报错', errs.length === 0, errs.length ? errs.slice(0, 3).join(' | ') : '无（资源加载噪音已排除）');
record('浏览器：0 未捕获异常', exceptions.length === 0, exceptions.length ? exceptions.slice(0, 2).join(' | ') : '无');

// 复习接口不得出现 4xx/5xx（401/404 之类的门禁与探针请求不算在内）
const badResponses = events
  .filter((e) => e.method === 'Network.responseReceived')
  .map((e) => ({ url: e.params.response.url, status: e.params.response.status }))
  .filter((r) => r.status >= 400 && /\/api\/reviews\//.test(r.url));
record('浏览器：复习接口无 4xx/5xx', badResponses.length === 0,
  badResponses.length ? badResponses.map((r) => `${r.status} ${r.url}`).join(' | ') : '全部 2xx');

// ---------- 6) 截图（可选） ----------
if (SHOT) {
  const shot = await send('Page.captureScreenshot', { format: 'png' });
  const fs = await import('node:fs');
  fs.writeFileSync(SHOT, Buffer.from(shot.data, 'base64'));
  console.log(`  截图已保存：${SHOT}`);
}

ws.close();
const failed = results.filter((r) => !r.ok);
console.log(JSON.stringify({ total: results.length, failed: failed.length, results }));
process.exit(failed.length ? 1 : 0);
