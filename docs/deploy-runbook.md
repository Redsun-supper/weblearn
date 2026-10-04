# 上线部署手册（P0-4）

> **这份文档的目标**：让「第一次上服务器」变成照着敲命令，而不是临场发挥。
> 每一步都写了**怎么验证这一步成功了**——没验证的步骤等于没做。
>
> ⚠️ 本手册**在本机（Windows）无法完整执行**：需要一台 Linux 服务器、一个域名、
> 证书与 DNS 权限。代码侧的准备工作（生产开关、Gin 模式、加固告警、配置模板、
> 测试）已经做完并验证过，本手册负责把「上服务器之后要做什么」写清楚。

## 0. 目标架构

```
                     ┌─ /            → /var/www/guangxue 静态文件（Nginx 直接托管）
浏览器 ──HTTPS──→    │
        Nginx(443)   ├─ /api/auth/*  → 127.0.0.1:8081  Rust 账号服务（systemd: guangxue-auth）
                     └─ /api/*       → 127.0.0.1:8080  Go 主后端  （systemd: guangxue-api）
```

两个后端**只监听 127.0.0.1**，公网上只有 Nginx 的 443/80。

## 1. 服务器准备

```bash
# 一台 Ubuntu 22.04+ / Debian 12+ 的机器，1C1G 就够（静态站 + 两个小服务 + SQLite）
sudo apt update && sudo apt install -y nginx certbot python3-certbot-nginx rsync
sudo adduser --system --group --home /opt/guangxue guangxue
```

DNS：把域名的 A 记录指向服务器 IP（`@` 与 `www` 各一条）。
**验证**：`dig +short 你的域名` 返回服务器 IP。这一步没通，第 5 步的证书一定失败。

## 2. 拿到代码与二进制

```bash
sudo mkdir -p /opt/guangxue && sudo chown guangxue:guangxue /opt/guangxue
sudo -u guangxue git clone <你的仓库地址> /opt/guangxue
```

在**服务器上**编译（最省事，省得处理跨平台产物）：

```bash
# Rust 账号服务（首次编译较慢，1C1G 大约 10~20 分钟）
cd /opt/guangxue/backend-rust
sudo -u guangxue cargo build --release
cp target/release/guangxue-auth ./guangxue-auth

# Go 主后端与备份工具
cd /opt/guangxue/backend-go
sudo -u guangxue go build -o guangxue-api .
sudo -u guangxue go build -o guangxue-backup ./cmd/backup
```

**验证**：

```bash
ls -l /opt/guangxue/backend-rust/guangxue-auth \
      /opt/guangxue/backend-go/guangxue-api \
      /opt/guangxue/backend-go/guangxue-backup        # 三个文件都在且可执行
file /opt/guangxue/backend-go/guangxue-api            # 要显示 ELF 64-bit ... x86-64
/opt/guangxue/backend-go/guangxue-backup -h 2>&1 | head -3   # 能跑起来（-h 会打印用法）
```

> ⚠️ 如果 `file` 显示的是 `PE32+`（Windows 格式），说明你在本机交叉编译后传上来了 ——
> 服务器上跑不了，回去重新编译。
>
> ⚠️ `go build -o guangxue-api .` 编译成功后**不会打印任何东西**（Go 的惯例）。
> 别以为它没跑，用上面的 `ls` 确认产物。

> 本机（Windows）也能交叉编译，但 `rusqlite` 是 bundled C 代码，交叉编译要装
> x86_64-unknown-linux-gnu 工具链；直接在服务器上编译更省心。

## 3. 配置两个 `.env`

生产配置**没有 .env 也能跑**（环境变量优先），但把密钥放 `.env` 最省事。
`chmod 600` 是硬要求——这两个文件里有 JWT 密钥和 SMTP 授权码。

```bash
cd /opt/guangxue/backend-rust
sudo -u guangxue cp .env.example .env
sudo -u guangxue nano .env      # 按下面的清单改
chmod 600 .env
```

**`backend-rust/.env`（账号服务）**：

```env
APP_ENV=production
AUTH_HOST=127.0.0.1                 # ⚠️ 只让 Nginx 访问
AUTH_PORT=8081
AUTH_DB_PATH=auth.db
AUTH_JWT_SECRET=<用 openssl rand -base64 48 生成>
AUTH_ALLOWED_ORIGINS=https://你的域名   # ⚠️ 漏配 = 所有写请求 403
AUTH_COOKIE_SECURE=true             # production 下默认就是 true，写出来更醒目
AUTH_MAIL_MODE=smtp                 # 见 P0-3；没配好就还是 log
AUTH_SMTP_HOST=...
AUTH_SMTP_PORT=465
AUTH_SMTP_USERNAME=no-reply@你的域名
AUTH_SMTP_PASSWORD=<邮箱授权码>
AUTH_SMTP_FROM="广学 <no-reply@你的域名>"   # ⚠️ 带空格必须加引号
AUTH_SMTP_TLS=implicit
AUTH_REQUIRE_INVITE=true            # ⚠️ 邀请制；默认 false，生产必须显式打开
AUTH_DEV_ENDPOINTS=false            # production 强制关闭（开了根本起不来）
AUTH_ADMIN_EMAIL=你的管理员邮箱
AUTH_ADMIN_PASSWORD=<强口令>         # ⚠️ 默认口令在生产会被拒绝启动
AUTH_SEED_ADMIN=true
```

**`backend-go/.env`（主后端）**：

```env
APP_ENV=production
SERVER_HOST=127.0.0.1               # ⚠️ 默认是 0.0.0.0，会把 8080 暴露到公网
SERVER_PORT=8080
DB_PATH=guangxue.db
AUTH_JWT_SECRET=<与上面 backend-rust 里**完全一致**的那串>
AUTH_ALLOWED_ORIGINS=https://你的域名
```

> ⚠️ **两处 `AUTH_ALLOWED_ORIGINS` 必须都改**：它同时是 CSRF 白名单与 CORS 依据。
> 只改一个的现象是「能登录，但一提交复习就 403」。
>
> ⚠️ **两处 `AUTH_JWT_SECRET` 必须完全一致**：Go 侧是本地验签（不查库、不问账号服务）。
> 不一致的现象是「登录成功，但复习接口一直 401」。

**验证**：`cd /opt/guangxue/backend-rust && ./guangxue-auth --help` 之类能跑起来；
真正的验证在第 4 步看启动日志里的告警。

## 4. 装 systemd 服务

```bash
cd /opt/guangxue
sudo cp deploy/systemd/guangxue-auth.service deploy/systemd/guangxue-api.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now guangxue-auth guangxue-api
```

**验证（这四条都要过）**：

```bash
systemctl is-active guangxue-auth guangxue-api          # 两个都 active
journalctl -u guangxue-auth -n 30                        # 看启动横幅
curl -sS http://127.0.0.1:8081/api/auth/health           # 200
curl -sS http://127.0.0.1:8080/api/health                # 200
```

启动横幅里要逐项核对：

| 横幅里的行 | 期望 | 不对的话 |
|---|---|---|
| `邮件模式` | `smtp（真实发信）` | 还是 log 就回去看 `.env` |
| `邮件自检` | `能连上 smtp.xxx:465（TLS 模式 implicit）` | 会打印 `⚠️ 连不上…`，检查主机/端口/防火墙 |
| `Cookie` | 带 `+ Secure` | 没带就是 `APP_ENV` 没写对 |
| Go 侧日志 | 没有 `⚠️` 开头的告警 | 告警文案直接说了哪一项配错了 |

> ⚠️ 服务器上**没有浏览器**，所以这一屏靠 `journalctl` 看。启动横幅里不打印任何密钥。

**再验一次「生产开关真的生效」**（这两条必须失败，否则等于没锁）：

```bash
cd /opt/guangxue/backend-rust
sudo -u guangxue env AUTH_DEV_ENDPOINTS=true ./guangxue-auth   # 期望：拒绝启动并说明原因
```

## 5. Nginx + HTTPS

```bash
cd /opt/guangxue
sudo sed 's/@DOMAIN@/你的域名/g' deploy/nginx/guangxue.conf.template \
  | sudo tee /etc/nginx/sites-available/guangxue.conf > /dev/null
sudo ln -sf /etc/nginx/sites-available/guangxue.conf /etc/nginx/sites-enabled/
sudo rm -f /etc/nginx/sites-enabled/default      # 删掉默认站点，否则域名会被它抢走
```

**⚠️ 先别 reload**：证书文件还不存在，`nginx -t` 会因为找不到 `fullchain.pem` 而报错。
顺序是「先拿证书（走 HTTP-01，临时用 http 配置），再启用这份带证书的配置」。

```bash
# ① 用一个只监听 80 的最小配置把证书拿到手
sudo tee /etc/nginx/sites-available/guangxue-http-only.conf > /dev/null <<'EOF'
server {
    listen 80;
    server_name @DOMAIN@;
    root /var/www/certbot;
    location ^~ /.well-known/acme-challenge/ { root /var/www/certbot; }
    location / { return 200 "ok\n"; }
}
EOF
sudo sed -i 's/@DOMAIN@/你的域名/g' /etc/nginx/sites-available/guangxue-http-only.conf
sudo ln -sf /etc/nginx/sites-available/guangxue-http-only.conf /etc/nginx/sites-enabled/guangxue-http-only.conf
sudo rm -f /etc/nginx/sites-enabled/guangxue.conf     # 临时摘掉那份要证书的
sudo mkdir -p /var/www/certbot
sudo nginx -t && sudo systemctl reload nginx

# ② 申请证书（certbot 需要能访问 80 端口）
sudo certbot certonly --webroot -w /var/www/certbot -d 你的域名

# ③ 换回正式配置
sudo ln -sf /etc/nginx/sites-available/guangxue.conf /etc/nginx/sites-enabled/guangxue.conf
sudo rm -f /etc/nginx/sites-enabled/guangxue-http-only.conf
sudo nginx -t && sudo systemctl reload nginx

# ④ 续期演练（**必须做**：证书 90 天过期，全自动续期失败是上线最常见的翻车点）
sudo certbot renew --dry-run
```

**验证**：

```bash
curl -sS -o /dev/null -w '%{http_code}\n' https://你的域名/healthz          # 200
curl -sS -o /dev/null -w '%{http_code}\n' https://你的域名/api/health       # 200
curl -sS -o /dev/null -w '%{http_code}\n' https://你的域名/api/auth/health  # 200
curl -sSI https://你的域名/ | grep -i strict-transport                     # 有 HSTS 头
curl -sS -o /dev/null -w '%{http_code}\n' http://你的域名/                   # 301 → https
```

> ⚠️ 如果 `/api/auth/health` 是 404 而 `/api/health` 是 200，就是 `proxy_pass` 末尾
> 多写了一个 `/`（会把 `/api/auth/health` 改写成 `/health`）。
> 如果反过来，则是两个 location 的顺序写反了。

## 6. 部署静态文件

**不要整仓库 rsync**：仓库根目录里有 `.env`、`备份/`、`.db` 快照、`pages.zip`。
用白名单只推前端要用的东西：

```bash
sudo mkdir -p /var/www/guangxue
sudo chown -R guangxue:guangxue /var/www/guangxue

cd /opt/guangxue
sudo -u guangxue rsync -a --delete \
  --include='index.html' --include='main.js' --include='main.css' \
  --include='modules/***' --include='account/***' --include='admin/***' \
  --include='image/***' --include='assets/***' --include='fonts/***' \
  --exclude='*' \
  ./ /var/www/guangxue/
```

**验证**：

```bash
curl -sS -o /dev/null -w '%{http_code}\n' https://你的域名/            # 200
curl -sS -o /dev/null -w '%{http_code}\n' https://你的域名/account/    # 200（不能是 301）
curl -sS -o /dev/null -w '%{http_code}\n' https://你的域名/admin/      # 200
curl -sS -o /dev/null -w '%{http_code}\n' https://你的域名/.env        # 403 或 404（**绝不能是 200**）
```

> ⚠️ `/account/` 如果返回 301 到 `/account/index.html`，说明 Nginx 用的是 `index` 指令
> 而不是 `try_files $uri $uri/index.html`。那次跳转会让页面里的相对路径全部失效。

更新时重复这一条命令即可（`--delete` 会清掉旧文件）。

## 7. 每日备份

```bash
sudo mkdir -p /srv/guangxue-backups && sudo chown guangxue:guangxue /srv/guangxue-backups
sudo cp /opt/guangxue/deploy/systemd/guangxue-backup.{service,timer} /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now guangxue-backup.timer

# 立刻手动跑一次，确认能出产物
sudo systemctl start guangxue-backup.service
journalctl -u guangxue-backup -n 30
ls -lh /srv/guangxue-backups/*/            # 应当有两个 .db，且**体积不是 0**
```

**验证备份可用**（这一步别省：备份没恢复过就等于没有备份）：

```bash
sqlite3 /srv/guangxue-backups/<时间戳>/auth.db 'PRAGMA integrity_check;'   # 期望 ok
sqlite3 /srv/guangxue-backups/<时间戳>/auth.db 'SELECT count(*) FROM users;'  # 数字应当对得上
```

**怎么确认它真的每天在跑**（定时器不会补跑，静默失败很常见）：

```bash
systemctl list-timers guangxue-backup.timer    # NEXT 应当是明天 03:00 左右
systemctl --failed                              # 有失败会列在这里
```

> 想把备份再往远处放一层：`rclone` 同步到对象存储，或者 `rsync` 到另一台机器。
> **备份和数据库放同一块盘是假的备份**——盘坏了两个一起没。
>
> ⚠️ `/srv/guangxue-backups` 与数据库不在同一个 systemd 沙箱里：
> `guangxue-backup.service` 的 `ReadWritePaths` 只放开了这个目录，
> 数据库目录它是**只读**的——备份工具没有写库的能力，这也是有意的。

### 恢复流程（演练一次再上线）

```bash
sudo systemctl stop guangxue-auth guangxue-api
sudo cp /opt/guangxue/backend-rust/auth.db /opt/guangxue/backend-rust/auth.db.broken   # 留证据
sudo -u guangxue cp /srv/guangxue-backups/<时间戳>/auth.db /opt/guangxue/backend-rust/auth.db
# ⚠️ 恢复后必须删掉 -wal / -shm：旧库的 WAL 和新库文件对不上，SQLite 会拒绝打开
sudo rm -f /opt/guangxue/backend-rust/auth.db-wal /opt/guangxue/backend-rust/auth.db-shm
sudo systemctl start guangxue-auth guangxue-api
curl -sS http://127.0.0.1:8081/api/auth/health
```

## 8. 日志与隐私复核

```bash
# ① 日志里**不能出现**验证码明文与密码
sudo journalctl -u guangxue-auth --since '1 hour ago' | grep -iE '验证码|password|code=' | head
sudo grep -iE 'password|验证码' /var/log/nginx/guangxue.access.log | head

# ② 确认请求体没被记进日志（Nginx 默认的 combined 格式只记 URL，不记 body）
sudo tail -5 /var/log/nginx/guangxue.access.log

# ③ gin 不该再刷调试日志（APP_ENV=production → ReleaseMode）
sudo journalctl -u guangxue-api -n 20 | grep -c '\[GIN\]'    # 期望 0
```

## 9. 上线前的最后一份清单

- [ ] `https://域名/api/health` 与 `/api/auth/health` 都 200
- [ ] 浏览器打开站点 → 注册（要邀请码）→ 登录 → 复习 → 统计，全链路通
- [ ] 刷新页面后登录态还在（Cookie `Secure` + `SameSite` 正确）
- [ ] 用**手机 4G**（不是家里的 WiFi）走一遍：确认没有依赖内网地址
- [ ] `curl https://域名/.env` 不是 200
- [ ] `certbot renew --dry-run` 通过
- [ ] 备份产物非空、`integrity_check` = ok、恢复演练过一次
- [ ] `systemctl --failed` 为空
- [ ] `sudo reboot` 后两个服务与定时器自动起来（**这条一定要真做一次**）
- [ ] `sysctl -w net.ipv4.tcp_syncookies=1`、开 ufw 只放 22/80/443（其余一律不开）

## 10. 这份手册里「本机做不到」的部分

| 做不到的事 | 原因 | 谁来补 |
|---|---|---|
| 真机验收（浏览器全链路、手机 4G） | 本机没有服务器也没有域名 | 你，在第 9 步清单里 |
| `certbot` 拿证书与续期演练 | 需要域名与 DNS 权限 | 你，第 5 步 |
| systemd 服务真实拉起 | Windows 上没有 systemd | 你，第 4 步 |
| SPF / DKIM / DMARC | 需要域名 DNS 控制台 | 见 [P0-3](launch-plan.md#p0-3-打通真发邮件) 第 4 步 |
| 生产限流调参（`AUTH_RL_*`） | 要按真实流量看 | 放人之后，见 P2 |

**代码侧已经做完并验证过的**：生产开关（dev 接口强制关、默认口令拒启、Cookie Secure）、
Gin 模式切换、Go 侧启动告警、Nginx / systemd / 备份模板、以及把这些行为钉住的测试
（`backend-go/routes/routes_test.go`、`backend-go/config/config_test.go`、
`backend-rust/src/config.rs` 的生产开关用例）。
