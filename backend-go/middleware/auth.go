// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  HR_RedSun (大冬呱)
// Package middleware 提供两类横切处理：
//   - 登录态校验：RequireUser / RequireAdmin —— 用账号服务签发的 gx_access Cookie 本地验签；
//   - CSRF 防护：CSRFGuard —— 与账号服务 backend-rust 的同名逻辑保持同一口径。
//
// 为什么 Go 自己验签、不回头问账号服务：两边共享同一把 AUTH_JWT_SECRET，
// Go 用 HS256 重算签名就能判断令牌真伪，不必查库、不必跨进程调用
// （方案与理由见 docs/roadmap.md 第 2 节、docs/backend-auth.md）。
// 代价是**不感知会话撤销**：退出登录 / 踢掉设备后，那张 access 令牌在过期前仍能通过验签。
// 这一点与账号服务自身的口径一致（它同样只校验签名与 exp），所以 access 令牌的有效期要短。
package middleware

import (
	"log"
	"net/http"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/gin-gonic/gin"
	"github.com/golang-jwt/jwt/v5"
)

const (
	// AccessCookieName 登录令牌所在的 Cookie 名，
	// 与账号服务 backend-rust/src/http/mod.rs:16 的 ACCESS_COOKIE 对齐
	AccessCookieName = "gx_access"

	// tokenLeeway 允许的时钟偏差，
	// 与账号服务 backend-rust/src/core/token.rs:39 的 LEEWAY_SECONDS = 60 对齐。
	// ⚠️ 两边必须一致：Go 这边更严就会出现「账号服务认为有效、Go 判过期」的 401 抖动。
	tokenLeeway = 60 * time.Second

	// RoleAdmin / RoleSuperAdmin 角色值。
	// ⚠️ 这是**跨服务的契约**：字面量必须与账号服务 backend-rust/src/models.rs 的
	// ROLE_ADMIN / ROLE_SUPER_ADMIN 逐字相同，否则症状是「后台能进、接口 403」，
	// 而且两边各自的单元测试都不会红（各自 mock 自己的字符串）。两侧都有测试钉住字面量。
	RoleAdmin = "admin"
	// RoleSuperAdmin 超级管理员：治理角色（发码、调权限、封号、查审计）
	RoleSuperAdmin = "super_admin"
)

// gin.Context 里的登录态键名：后续处理器要取「当前用户是谁」时统一用这几个常量，
// 不要各写各的字符串字面量（写错了编译期发现不了）
const (
	CtxUserID    = "auth.user_id"    // int64：users.id
	CtxSessionID = "auth.session_id" // int64：登录会话（sessions.id）
	CtxRole      = "auth.role"       // string：user / admin / super_admin
)

// AccessClaims 与账号服务 backend-rust/src/core/token.rs:25-36 的 AccessClaims 一一对应。
//
// ⚠️ sub / sid 在账号服务里是 i64（JSON 数字），不能改成 string：
// 数字放进字符串字段会让解析直接失败，表现为「明明登录了却一直 401」。
type AccessClaims struct {
	Sub  int64  `json:"sub"`  // 用户 id
	Sid  int64  `json:"sid"`  // 会话 id
	Role string `json:"role"` // 角色：user / admin / super_admin
	Iat  int64  `json:"iat"`  // 签发时间（Unix 秒）
	Exp  int64  `json:"exp"`  // 过期时间（Unix 秒）
	Jti  string `json:"jti"`  // 令牌唯一 id
}

// 以下六个方法是为了满足 jwt/v5 的 Claims 接口（校验器靠它们取时间与主体）。
// 账号服务的令牌里没有 iss / aud / nbf，这里如实返回「没有」，不做任何编造。

func (c *AccessClaims) GetExpirationTime() (*jwt.NumericDate, error) {
	return jwt.NewNumericDate(time.Unix(c.Exp, 0)), nil
}

func (c *AccessClaims) GetIssuedAt() (*jwt.NumericDate, error) {
	return jwt.NewNumericDate(time.Unix(c.Iat, 0)), nil
}

func (c *AccessClaims) GetNotBefore() (*jwt.NumericDate, error) { return nil, nil }

func (c *AccessClaims) GetIssuer() (string, error) { return "", nil }

// GetSubject 返回字符串形式的用户 id（jwt 规范里 sub 是字符串，账号服务发的是数字）
func (c *AccessClaims) GetSubject() (string, error) { return strconv.FormatInt(c.Sub, 10), nil }

func (c *AccessClaims) GetAudience() (jwt.ClaimStrings, error) { return nil, nil }

// RequireUser 要求请求带一张有效的登录令牌，否则 401 并中断。
// 通过后把用户 id / 会话 id / 角色放进 gin.Context 供处理器使用。
func RequireUser(secret string) gin.HandlerFunc {
	return func(c *gin.Context) {
		claims, ok := verifyRequestToken(c, secret)
		if !ok {
			abortUnauthorized(c)
			return
		}
		setAuthContext(c, claims)
		c.Next()
	}
}

// CanEnterAdmin 能不能进后台做内容/运营：管理员与超管都可以。
//
// 收口成一个函数是为了避免「放行规则」散落在各处 —— 少改一处的症状是
// 超管被自己的后台挡在门外（403），而日志里只留下一句「非管理员访问」。
func CanEnterAdmin(role string) bool {
	return role == RoleAdmin || role == RoleSuperAdmin
}

// RequireAdmin 在 RequireUser 之上再要求「能进后台」（admin 或 super_admin）：
// 没登录 → 401；登录了但两条都不是 → 403（与账号服务 backend-rust/src/http/extract.rs 同口径）。
func RequireAdmin(secret string) gin.HandlerFunc {
	return func(c *gin.Context) {
		claims, ok := verifyRequestToken(c, secret)
		if !ok {
			abortUnauthorized(c)
			return
		}
		if !CanEnterAdmin(claims.Role) {
			abortForbidden(c)
			return
		}
		setAuthContext(c, claims)
		c.Next()
	}
}

// RequireSuperAdmin 在 RequireUser 之上再要求 role == "super_admin"：
// **比 RequireAdmin 严格更窄** —— 普通管理员做不了治理动作（发码、改他人角色、封禁）。
//
// ⚠️ 目前 Go 侧还没有治理类接口（发码在账号服务里），这条中间件是为「以后要在 Go 上加
// 管理接口」预留的，并顺手把口径与 Rust 侧对齐；有测试直接调用它，避免它成为死代码。
func RequireSuperAdmin(secret string) gin.HandlerFunc {
	return func(c *gin.Context) {
		claims, ok := verifyRequestToken(c, secret)
		if !ok {
			abortUnauthorized(c)
			return
		}
		if claims.Role != RoleSuperAdmin {
			abortForbidden(c)
			return
		}
		setAuthContext(c, claims)
		c.Next()
	}
}

// CSRFGuard 防跨站请求伪造，逻辑照抄账号服务的 csrf_guard
// （backend-rust/src/http/middleware.rs:98-127）：两道闸门，任一满足即可放行。
//
//  1. 带 Origin 时必须命中白名单 —— 浏览器发起的请求都会带 Origin（同源请求也带）；
//  2. 不带 Origin 时要求 Content-Type 以 application/json 开头
//     —— 跨站表单只能发 application/x-www-form-urlencoded 或 multipart/form-data，发不出 JSON。
//
// 只读方法（GET / HEAD / OPTIONS）直接放行：它们不该改变状态。
// 注意这不是防「同源内的 XSS」，那是 HttpOnly + 输入转义的职责。
func CSRFGuard(allowedOrigins []string) gin.HandlerFunc {
	return func(c *gin.Context) {
		switch c.Request.Method {
		case http.MethodGet, http.MethodHead, http.MethodOptions:
			c.Next()
			return
		}

		if origin := c.GetHeader("Origin"); origin != "" {
			if !slices.Contains(allowedOrigins, origin) {
				log.Printf("Origin 不在白名单，拒绝该请求: %s", origin)
				abortForbidden(c)
				return
			}
		} else if ct := c.GetHeader("Content-Type"); !strings.HasPrefix(ct, "application/json") {
			log.Printf("缺少 Origin 且不是 JSON 请求，拒绝该请求: Content-Type=%q", ct)
			abortForbidden(c)
			return
		}
		c.Next()
	}
}

// verifyRequestToken 校验请求里的登录令牌：验签 + 过期时间 + 算法钉死。
//
// 任何失败都只返回 false，调用方一律按「未登录」处理 —— 不回显具体原因，
// 免得把「签名错 / 已过期 / 角色不对」变成给攻击者用的探针。
func verifyRequestToken(c *gin.Context, secret string) (*AccessClaims, bool) {
	// 密钥没配（本机没写 .env）时一律判不通过：
	// 绝不能让「用空密钥签出来的令牌」蒙混过关
	if secret == "" {
		return nil, false
	}

	raw, err := c.Cookie(AccessCookieName)
	if err != nil || raw == "" {
		return nil, false
	}

	claims := &AccessClaims{}
	token, err := jwt.ParseWithClaims(raw, claims, func(*jwt.Token) (interface{}, error) {
		return []byte(secret), nil
	},
		// 钉死 HS256：否则攻击者把 header 的 alg 改成 none 或 RS256 就可能绕过验签
		jwt.WithValidMethods([]string{jwt.SigningMethodHS256.Alg()}),
		// 令牌必须带 exp，且过期判定与账号服务一致（now > exp + 60 秒才算过期）
		jwt.WithExpirationRequired(),
		jwt.WithLeeway(tokenLeeway),
	)
	if err != nil || token == nil || !token.Valid {
		return nil, false
	}
	return claims, true
}

func setAuthContext(c *gin.Context, claims *AccessClaims) {
	c.Set(CtxUserID, claims.Sub)
	c.Set(CtxSessionID, claims.Sid)
	c.Set(CtxRole, claims.Role)
}

// CurrentUserID 取本次请求的登录用户 id（账号服务 auth.db 里的 users.id）。
//
// ⚠️ 只在挂了 RequireUser / RequireAdmin 的路由上调用：没挂时返回 (0, false)，
// 调用方必须按「未登录」处理（回 401），**绝不能**把 0 当成一个真实用户——user_id = 0
// 是 P0-1 迁移前历史数据的占位值，一旦当成某个用户写进库，那些行对谁都不再可见。
func CurrentUserID(c *gin.Context) (uint, bool) {
	v, ok := c.Get(CtxUserID)
	if !ok {
		return 0, false
	}
	id, ok := v.(int64)
	if !ok || id <= 0 {
		return 0, false
	}
	return uint(id), true
}

// 错误响应体沿用统一信封 {code, message, error}，与账号服务 backend-rust/src/error.rs:100-110 对齐：
// 前端按 error 字段（unauthenticated / forbidden）做分支即可，不要去匹配中文文案。
func abortUnauthorized(c *gin.Context) {
	c.AbortWithStatusJSON(http.StatusUnauthorized, gin.H{
		"code":    http.StatusUnauthorized,
		"message": "请先登录",
		"error":   "unauthenticated",
	})
}

func abortForbidden(c *gin.Context) {
	c.AbortWithStatusJSON(http.StatusForbidden, gin.H{
		"code":    http.StatusForbidden,
		"message": "没有权限",
		"error":   "forbidden",
	})
}
