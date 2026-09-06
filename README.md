# 广学

## ⚡ 我的爱发电主页

如果这个项目对你有帮助，欢迎在爱发电支持我！

[👉 在爱发电支持我](https://afdian.com/a/hr_redsun)

---

一个学科内容展示平台，支持多科目内容加载与本地缓存。

---

## 项目架构

```
┌─────────────────────────────────────────────────────────────────┐
│                         用户浏览器                                │
│                    (HTTPS: test.lovezmx.com)                     │
└────────────────────────────┬────────────────────────────────────┘
                             │
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│                        Nginx 反向代理                             │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │  location /        → 前端静态文件 (frontend/)              │  │
│  │  location /api/    → Go后端服务 (localhost:8080)           │  │
│  └───────────────────────────────────────────────────────────┘  │
────────────────────────────┬────────────────────────────────────┘
                             │
              ┌──────────────┴──────────────┐
              ▼                              ▼
┌─────────────────────────┐    ┌─────────────────────────┐
│       前端 (Frontend)    │    │      后端 (Backend)      │
│                         │    │                         │
│  index.html             │    │  Go (Gin框架)           │
│  main.css               │    │                         │
│  main.js                │    │  ├── main.go            │
│  image/                 │    │  ├── config/            │
│  pages/                 │    │  ├── routes/            │
│                         │    │  ├── handlers/          │
│  特性:                   │    │  ├── models/           │
│  - 学科导航              │    │  └── utils/            │
│  - localStorage缓存      │    │                         │
│  - 30天自动清理          │    │  特性:                   │
│  - 默认显示英语          │    │  - RESTful API          │
│                         │    │  - 健康检查              │
│                         │    │  - 用户管理              │
└─────────────────────────    │  - 数据管理              │
                               └─────────────────────────┘
```

---

## 目录结构

```
e:\porject\4/
├── frontend/                    # 前端静态文件
│   ├── index.html               # 主页面入口
│   ├── main.css                 # 全局样式
│   ├── main.js                  # 交互逻辑与缓存管理
│   ├── image/                   # 图片资源
│   │   └── avatar.png           # 头像图片
│   ── pages/                   # 学科内容页面
│       ├── english.html         # 英语（已实现）
│       ├── chinese.html         # 语文（敬请期待）
│       ├── math.html            # 数学（敬请期待）
│       ├── physics.html         # 物理（敬请期待）
│       ├── chemistry.html       # 化学（敬请期待）
│       ├── biology.html         # 生物（敬请期待）
│       ├── history.html         # 历史（敬请期待）
│       ├── politics.html        # 政治（敬请期待）
│       └── geography.html       # 地理（敬请期待）
│
├── backend-go/                  # Go后端服务
│   ├── main.go                  # 程序入口
│   ├── go.mod                   # Go模块定义
│   ├── config/
│   │   └── config.go            # 配置管理
│   ├── database/
│   │   └── database.go          # SQLite 连接与自动迁移
│   ├── routes/
│   │   └── routes.go            # 路由定义
│   ├── handlers/
│   │   ├── handlers.go          # 请求处理器
│   │   └── review_handlers.go   # 词汇复习（FSRS）处理器
│   ├── models/
│   │   └── models.go            # 数据模型（含 Word/WordReview/ReviewLog）
│   ├── utils/
│   │   └── utils.go             # 工具函数
│   └── README.md                # 后端说明文档
│
├── frontend-rust/               # 前端 Rust/WASM 模块（间隔复习引擎）
│   ├── Cargo.toml               # cdylib + wasm-bindgen + fsrs
│   ├── src/
│   │   ├── lib.rs               # 模块导出
│   │   ├── fsrs_engine.rs       # FSRS 调度引擎
│   │   └── randomizer.rs        # 随机器（洗牌/抽样）
│   └── README.md                # 引擎说明文档
│
└── README.md                    # 项目总说明（本文件）
```

---

## 技术栈

### 前端
| 技术 | 用途 |
|------|------|
| HTML5 | 页面结构 |
| CSS3 | 样式与布局 |
| JavaScript (ES5) | 交互逻辑 |
| localStorage | 本地缓存（30天过期） |
| fetch API | HTTP请求 |

### 后端
| 技术 | 用途 |
|------|------|
| Go 1.21 | 后端语言 |
| Gin | Web框架 |
| GORM + SQLite (glebarez) | 数据持久化（纯 Go，无 CGO） |
| Nginx | 反向代理与静态文件服务 |

### 复习引擎
| 技术 | 用途 |
|------|------|
| Rust + wasm-bindgen | 前端 WASM 模块（在浏览器中运行） |
| fsrs v6 (fsrs-rs) | FSRS 间隔复习调度算法 |
| serde / serde_json | 引擎 JSON 数据交换 |

---

## 请求流程

```
用户点击学科按钮
        │
        ▼
┌──────────────────┐
│ 检查localStorage  │
│ 是否有缓存？       │
└─────────────────┘
         │
    ────┴────┐
    │         │
   有         无
    │         │
    ▼         ▼
──────┐  ┌──────────┐
│直接  │  │fetch请求 │
│显示  │  │服务器    │
└──────┘  └────┬─────┘
               │
               ▼
         ┌──────────┐
         │保存到    │
         │localStorage│
         └────┬─────┘
              │
              ▼
         ┌──────┐
         │显示  │
         │内容  │
         └──────
```

---

## API接口

### 后端API（Go）

| 方法 | 路径 | 说明 | 状态 |
|------|------|------|------|
| GET | `/api/health` | 健康检查 | ✅ 可用 |
| GET | `/api/hello` | 欢迎信息 | ✅ 可用 |
| GET | `/api/user/info` | 获取用户信息 | 🔄 待实现 |
| POST | `/api/user/update` | 更新用户信息 | 🔄 待实现 |
| GET | `/api/data/list` | 获取数据列表 | 🔄 待实现 |
| POST | `/api/data/submit` | 提交数据 | 🔄 待实现 |
| GET | `/api/words` | 单词列表 | ✅ 可用 |
| POST | `/api/words` | 批量添加单词 | ✅ 可用 |
| GET | `/api/reviews/due` | 到期复习卡列表 | ✅ 可用 |
| GET | `/api/reviews/new` | 未加入复习的新词 | ✅ 可用 |
| POST | `/api/reviews/submit` | 提交复习结果（持久化 FSRS 状态） | ✅ 可用 |
| GET | `/api/reviews/stats` | 复习统计 | ✅ 可用 |

---

## 缓存机制

### localStorage缓存策略

```
┌─────────────────────────────────────────────────────┐
│                   缓存生命周期                        │
│                                                     │
│  首次访问 → 请求服务器 → 保存到localStorage          │
│       │                                             │
│       ▼                                             │
│  再次访问 → 直接读取localStorage（零带宽）            │
│       │                                             │
│       ▼                                             │
│  30天未访问 → 自动清理过期缓存                       │
│                                                     │
│  存储空间不足 → 自动清理过期缓存后重试                │
└─────────────────────────────────────────────────────┘
```

### 缓存数据结构

| 键名 | 用途 |
|------|------|
| `pageCache_pages/xxx.html` | 存储页面HTML内容 |
| `pageCache_meta` | 存储各页面最后访问时间 |

---

## 部署说明

### 服务器目录结构

```
/var/www/test.lovezmx.com/
├── frontend/          # 前端文件
├── backend-go/        # 后端代码
── nginx.conf         # Nginx配置
```

### Nginx配置示例

```nginx
server {
    listen 443 ssl;
    server_name test.lovezmx.com;

    # SSL证书配置
    ssl_certificate /path/to/cert.pem;
    ssl_certificate_key /path/to/key.pem;

    # 前端静态文件
    location / {
        root /var/www/test.lovezmx.com/frontend;
        index index.html;
        try_files $uri $uri/ /index.html;
    }

    # Go后端API
    location /api/ {
        proxy_pass http://localhost:8080/;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    }
}
```

### 启动后端服务

```bash
# 安装依赖
cd backend-go
go mod tidy

# 开发模式运行
go run main.go

# 生产模式构建
go build -o server main.go
./server
```

### 环境变量

| 变量 | 说明 | 默认值 |
|------|------|--------|
| `SERVER_HOST` | 监听地址 | `0.0.0.0` |
| `SERVER_PORT` | 端口 | `8080` |
| `APP_ENV` | 运行环境 | `development` |

---

## 开发计划

- [x] 前端基础架构
- [x] 学科导航功能
- [x] localStorage缓存机制
- [x] 30天自动清理
- [x] Go后端基础框架
- [x] 词汇间隔复习引擎（Rust/WASM：FSRS 调度 + 随机器）
- [x] 数据库集成（SQLite + GORM，自动迁移完成）
- [ ] 用户认证系统
- [ ] 学科内容完善
- [ ] 响应式优化
- [x] 复习页面 UI（简单版：pages/english.html + main.js 集成 WASM 引擎）
- [ ] 复习页面 UI（进阶：完整词义卡交互、发音、统计图表等）
- [ ] FSRS 参数优化（基于 review_logs 的 compute_parameters）

---

## 支持作者

如果这个项目对你有帮助，欢迎在爱发电支持我！

[⚡ 在爱发电支持我](https://afdian.com/a/hr_redsun)

---

## 许可证

MIT License