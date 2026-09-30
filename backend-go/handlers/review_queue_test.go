package handlers

import (
	"testing"
	"time"

	"gorm.io/gorm"

	"backend-go/models"
)

// 本文件覆盖复习取数四个接口的「取数条件」：
//   - GET /api/reviews/new    没有 word_reviews 行的词（= 尚未加入复习的新词），按 words.id 升序
//   - GET /api/reviews/due    已学且 due_at <= now 的卡，按 due_at 升序
//   - GET /api/reviews/queue  所有已学词（含未到期）按 due_at 升序，带 total 分页
//   - GET /api/reviews/probes 已学词按 due_at 倒序（越轮不到复习的越靠前）
//
// 关于「每日 5 新词 + 5 抽查」的配额：配额是**客户端**按 localStorage 记账的
// （docs/review-engine.md 有说明），服务端只提供候选池。所以这里测的是服务端这一侧的
// 口径——把 limit 收到 5 时能拿到「最该出现的那 5 个」候选，以及 limit 的边界钳制。
// 服务端一旦哪天开始按天限流，这些用例就是现成的回归锚点。

// cardWordIDs 取出 dueCard 列表里的 word_id（响应里的字段名是 id，见 dueCard 结构）
func cardWordIDs(t *testing.T, data map[string]interface{}) []uint {
	t.Helper()
	list := items(t, data)
	out := make([]uint, 0, len(list))
	for _, item := range list {
		raw, ok := item["id"]
		if !ok {
			t.Fatalf("到期卡缺少 id 字段，实际字段=%v", mapKeys(item))
		}
		f, ok := raw.(float64)
		if !ok {
			t.Fatalf("到期卡 id 期望数字，实际类型=%T", raw)
		}
		out = append(out, uint(f))
	}
	return out
}

// wordIDs 取出完整 Word 列表里的 id（/api/reviews/new 返回的是 models.Word）
func wordIDs(t *testing.T, data map[string]interface{}) []uint {
	t.Helper()
	list := items(t, data)
	out := make([]uint, 0, len(list))
	for _, item := range list {
		raw, ok := item["id"]
		if !ok {
			t.Fatalf("词条缺少 id 字段，实际字段=%v", mapKeys(item))
		}
		f, ok := raw.(float64)
		if !ok {
			t.Fatalf("词条 id 期望数字，实际类型=%T", raw)
		}
		out = append(out, uint(f))
	}
	return out
}

// assertIDOrder 断言 id 序列与期望完全一致（顺序也要求一致）
func assertIDOrder(t *testing.T, got, want []uint, label string) {
	t.Helper()
	if len(got) != len(want) {
		t.Fatalf("%s 长度期望 %d（%v），实际 %d（%v）", label, len(want), want, len(got), got)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("%s 第 %d 位期望 %d，实际 %d（完整序列 %v）", label, i+1, want[i], got[i], got)
		}
	}
}

// reviewsFixture 一套覆盖四种状态的词库 + 记忆状态：
//
//	newOnly      没有复习行          → 只进 new
//	dueOld       到期最久（-3h）      → due / queue 最前，probes 最后
//	dueRecent    刚到期的（-10m）     → due / queue 中间
//	futureFar    远期未到期（+30d）   → 不进 due，queue 最后，probes 最前
//	futureNear   近期未到期（+1h）    → 不进 due，queue 中间
type reviewsFixture struct {
	newOnly    uint
	dueOld     uint
	dueRecent  uint
	futureNear uint
	futureFar  uint
}

// seedReviewsFixture 造出 reviewsFixture 描述的局面，返回各词的 id
func seedReviewsFixture(t *testing.T, db *gorm.DB) reviewsFixture {
	t.Helper()
	f := reviewsFixture{
		newOnly:    seedWord(t, db, "obsure"),
		dueOld:     seedWord(t, db, "pursue"),
		dueRecent:  seedWord(t, db, "quaint"),
		futureNear: seedWord(t, db, "relent"),
		futureFar:  seedWord(t, db, "soothe"),
	}
	now := time.Now()
	old := now.Add(-3 * time.Hour)
	recent := now.Add(-10 * time.Minute)
	near := now.Add(time.Hour)
	far := now.Add(30 * 24 * time.Hour)

	// stability 用序号做区分，便于按字段核对顺序
	seedReview(t, db, f.dueOld, 1, 5, &old, 1, 0)
	seedReview(t, db, f.dueRecent, 2, 5, &recent, 2, 0)
	seedReview(t, db, f.futureNear, 3, 5, &near, 3, 0)
	seedReview(t, db, f.futureFar, 4, 5, &far, 4, 0)
	return f
}

// TestReviewsNewOnlyUnlearned new 池只放没有 word_reviews 行的词，且按 id 升序
func TestReviewsNewOnlyUnlearned(t *testing.T) {
	router, db := setupTestRouter(t)
	f := seedReviewsFixture(t, db)

	data, code := decodeData(t, doJSON(t, router, "GET", "/api/reviews/new", nil))
	if code != 200 {
		t.Fatalf("new 的 code 期望 200，实际 %d", code)
	}
	assertIDOrder(t, wordIDs(t, data), []uint{f.newOnly}, "new 池")

	// 新词走完后（给它建一张复习行）它就离开 new 池，这是「懒创建」的另一面
	submitReview(t, router, newReviewRequest(f.newOnly, 3, 1, 5, 1))
	data2, _ := decodeData(t, doJSON(t, router, "GET", "/api/reviews/new", nil))
	assertIDOrder(t, wordIDs(t, data2), nil, "提交复习后的 new 池")
	if len(wordIDs(t, data2)) != 0 {
		t.Fatalf("已学词不应再出现在 new 池")
	}
}

// TestReviewsNewOrderAndLimit new 池按 words.id 升序，limit 生效
func TestReviewsNewOrderAndLimit(t *testing.T) {
	router, db := setupTestRouter(t)
	first := seedWord(t, db, "thrive")
	second := seedWord(t, db, "uphold")
	third := seedWord(t, db, "vanish")

	data, _ := decodeData(t, doJSON(t, router, "GET", "/api/reviews/new?limit=2", nil))
	assertIDOrder(t, wordIDs(t, data), []uint{first, second}, "new limit=2")

	dataAll, _ := decodeData(t, doJSON(t, router, "GET", "/api/reviews/new?limit=100", nil))
	assertIDOrder(t, wordIDs(t, dataAll), []uint{first, second, third}, "new limit=100")
}

// TestReviewsDueOnlyExpired due 池只放已到期的卡，按 due_at 升序（最旧的排最前）
func TestReviewsDueOnlyExpired(t *testing.T) {
	router, db := setupTestRouter(t)
	f := seedReviewsFixture(t, db)

	data, code := decodeData(t, doJSON(t, router, "GET", "/api/reviews/due", nil))
	if code != 200 {
		t.Fatalf("due 的 code 期望 200，实际 %d", code)
	}
	assertIDOrder(t, cardWordIDs(t, data), []uint{f.dueOld, f.dueRecent}, "due 池")

	// 未到期的两个不能出现
	for _, id := range cardWordIDs(t, data) {
		if id == f.futureNear || id == f.futureFar {
			t.Fatalf("未到期的词 %d 不应出现在 due 池", id)
		}
	}
	// 没有复习行的新词也不能出现
	for _, id := range cardWordIDs(t, data) {
		if id == f.newOnly {
			t.Fatalf("新词不应出现在 due 池")
		}
	}
	// 响应里带 now（毫秒），前端用它做本地判定
	if _, ok := data["now"]; !ok {
		t.Fatalf("due 响应缺少 now 字段，实际字段=%v", mapKeys(data))
	}
}

// TestReviewsDueNowOverride due 的 now 参数可覆盖「现在」：
// 传一个更早的时刻，原本已到期的卡就不算到期了（客户端与服务端时间对齐用的口子）。
func TestReviewsDueNowOverride(t *testing.T) {
	router, db := setupTestRouter(t)
	wordID := seedWord(t, db, "wander")
	due := time.Now().Add(30 * time.Minute)
	seedReview(t, db, wordID, 2, 5, &due, 1, 0)

	// 不传 now：尚未到期，due 池为空
	data, _ := decodeData(t, doJSON(t, router, "GET", "/api/reviews/due", nil))
	assertIDOrder(t, cardWordIDs(t, data), nil, "未到期时的 due 池")

	// now 传 1 小时后：这张卡算到期
	futureMs := time.Now().Add(time.Hour).UnixMilli()
	data2, _ := decodeData(t, doJSON(t, router, "GET", "/api/reviews/due?now="+itoa(futureMs), nil))
	assertIDOrder(t, cardWordIDs(t, data2), []uint{wordID}, "now 覆盖后的 due 池")
	assertInt(t, data2, "now", futureMs)
}

// TestReviewsQueueIncludesNotDue queue 给「整库已学词」——含未到期的，按 due_at 升序
func TestReviewsQueueIncludesNotDue(t *testing.T) {
	router, db := setupTestRouter(t)
	f := seedReviewsFixture(t, db)

	data, code := decodeData(t, doJSON(t, router, "GET", "/api/reviews/queue", nil))
	if code != 200 {
		t.Fatalf("queue 的 code 期望 200，实际 %d", code)
	}
	assertIDOrder(t, cardWordIDs(t, data),
		[]uint{f.dueOld, f.dueRecent, f.futureNear, f.futureFar}, "queue")
	// total = 已学词总数（不含新词）
	assertInt(t, data, "total", 4)
	assertInt(t, data, "limit", 100)
	assertInt(t, data, "offset", 0)
	if _, ok := data["now"]; !ok {
		t.Fatalf("queue 响应缺少 now 字段，实际字段=%v", mapKeys(data))
	}
}

// TestReviewsQueuePagination queue 的分页：total 是整库已学词数，翻页只改 items
func TestReviewsQueuePagination(t *testing.T) {
	router, db := setupTestRouter(t)
	f := seedReviewsFixture(t, db)

	page1, _ := decodeData(t, doJSON(t, router, "GET", "/api/reviews/queue?limit=2&offset=0", nil))
	assertIDOrder(t, cardWordIDs(t, page1), []uint{f.dueOld, f.dueRecent}, "queue 第 1 页")
	assertInt(t, page1, "total", 4)
	assertInt(t, page1, "limit", 2)
	assertInt(t, page1, "offset", 0)

	page2, _ := decodeData(t, doJSON(t, router, "GET", "/api/reviews/queue?limit=2&offset=2", nil))
	assertIDOrder(t, cardWordIDs(t, page2), []uint{f.futureNear, f.futureFar}, "queue 第 2 页")
	assertInt(t, page2, "total", 4) // total 不随翻页变
	assertInt(t, page2, "offset", 2)

	// 翻到底之外返回空列表（前端据此判断「整库过了一遍」）
	page3, _ := decodeData(t, doJSON(t, router, "GET", "/api/reviews/queue?limit=2&offset=4", nil))
	assertIDOrder(t, cardWordIDs(t, page3), nil, "queue 越界页")
	assertInt(t, page3, "total", 4)
}

// TestReviewsProbesFarthestFirst probes 是 queue 的另一端：按 due_at 倒序，越轮不到复习的越靠前
func TestReviewsProbesFarthestFirst(t *testing.T) {
	router, db := setupTestRouter(t)
	f := seedReviewsFixture(t, db)

	data, code := decodeData(t, doJSON(t, router, "GET", "/api/reviews/probes", nil))
	if code != 200 {
		t.Fatalf("probes 的 code 期望 200，实际 %d", code)
	}
	assertIDOrder(t, cardWordIDs(t, data),
		[]uint{f.futureFar, f.futureNear, f.dueRecent, f.dueOld}, "probes")

	// probes 与 queue 是同一份数据的两端：集合相同、顺序相反
	queueData, _ := decodeData(t, doJSON(t, router, "GET", "/api/reviews/queue", nil))
	queueIDs := cardWordIDs(t, queueData)
	probeIDs := cardWordIDs(t, data)
	if len(queueIDs) != len(probeIDs) {
		t.Fatalf("probes 与 queue 应当是同一份数据，长度 %d vs %d", len(probeIDs), len(queueIDs))
	}
	for i := range queueIDs {
		if queueIDs[i] != probeIDs[len(probeIDs)-1-i] {
			t.Fatalf("probes 应当是 queue 的倒序，第 %d 位 %d vs %d（queue=%v probes=%v）",
				i+1, queueIDs[i], probeIDs[len(probeIDs)-1-i], queueIDs, probeIDs)
		}
	}
}

// TestReviewsDailyQuotaCandidates 每日配额的取数侧：把 limit 收到 5，两个池各自给出
// 「最该出现的 5 个」——new 池按 id 升序取前 5（前端再随机抽），probes 取 due_at 最远的 5 个。
// 服务端不做每日记账（那是客户端 localStorage 的事），这里钉住的是候选池的取数与上限。
func TestReviewsDailyQuotaCandidates(t *testing.T) {
	router, db := setupTestRouter(t)
	now := time.Now()

	// 7 个新词
	newIDs := make([]uint, 0, 7)
	for i := 0; i < 7; i++ {
		newIDs = append(newIDs, seedWord(t, db, "newbie"+itoa(int64(i))))
	}
	// 7 个已学词，到期时间从近到远
	dueIDs := make([]uint, 0, 7)
	for i := 0; i < 7; i++ {
		id := seedWord(t, db, "veteran"+itoa(int64(i)))
		due := now.Add(time.Duration(i) * 24 * time.Hour)
		seedReview(t, db, id, float64(i+1), 5, &due, 1, 0)
		dueIDs = append(dueIDs, id)
	}

	newData, _ := decodeData(t, doJSON(t, router, "GET", "/api/reviews/new?limit=5", nil))
	assertIDOrder(t, wordIDs(t, newData), newIDs[:5], "new limit=5")

	probeData, _ := decodeData(t, doJSON(t, router, "GET", "/api/reviews/probes?limit=5", nil))
	// due_at 倒序 → 最远的 5 个（dueIDs 是升序，末尾 5 个反过来）
	wantProbes := []uint{dueIDs[6], dueIDs[5], dueIDs[4], dueIDs[3], dueIDs[2]}
	assertIDOrder(t, cardWordIDs(t, probeData), wantProbes, "probes limit=5")

	// 客户端「多给候选」的用法：limit 放大后能看到更多
	bigProbe, _ := decodeData(t, doJSON(t, router, "GET", "/api/reviews/probes?limit=20", nil))
	if n := len(cardWordIDs(t, bigProbe)); n != 7 {
		t.Fatalf("limit=20 时 probes 应给全部 7 张已学卡，实际 %d", n)
	}
}

// TestReviewsLimitClamping limit 的边界钳制：0 / 负数 / 超大值都退回默认，防止一次拉全库。
func TestReviewsLimitClamping(t *testing.T) {
	cases := []struct {
		name      string
		path      string
		limitQ    string
		wantLimit int64
	}{
		{name: "queue 默认", path: "/api/reviews/queue", limitQ: "", wantLimit: 100},
		{name: "queue limit=0 退回默认", path: "/api/reviews/queue", limitQ: "0", wantLimit: 100},
		{name: "queue 负数退回默认", path: "/api/reviews/queue", limitQ: "-5", wantLimit: 100},
		{name: "queue 非数字退回默认", path: "/api/reviews/queue", limitQ: "abc", wantLimit: 100},
		{name: "queue 超上限退回默认", path: "/api/reviews/queue", limitQ: "501", wantLimit: 100},
		{name: "queue 上限内保留", path: "/api/reviews/queue", limitQ: "50", wantLimit: 50},
		{name: "queue 正好上限保留", path: "/api/reviews/queue", limitQ: "500", wantLimit: 500},
		{name: "due 默认", path: "/api/reviews/due", limitQ: "", wantLimit: 0}, // due 不回 limit 字段
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			router, db := setupTestRouter(t)
			seedWord(t, db, "yonder")

			path := tc.path
			if tc.limitQ != "" {
				path += "?limit=" + tc.limitQ
			}
			data, code := decodeData(t, doJSON(t, router, "GET", path, nil))
			if code != 200 {
				t.Fatalf("%s 的 code 期望 200，实际 %d", path, code)
			}
			if tc.wantLimit > 0 {
				assertInt(t, data, "limit", tc.wantLimit)
			}
		})
	}
}

// TestReviewsQueueOffsetNegative offset 传负数按 0 处理（不能变成 SQL 的负偏移）
func TestReviewsQueueOffsetNegative(t *testing.T) {
	router, db := setupTestRouter(t)
	first := seedWord(t, db, "zealot")
	second := seedWord(t, db, "abbey")
	now := time.Now()
	seedReview(t, db, first, 1, 5, &now, 1, 0)
	seedReview(t, db, second, 2, 5, &now, 2, 0)

	data, _ := decodeData(t, doJSON(t, router, "GET", "/api/reviews/queue?offset=-3", nil))
	assertInt(t, data, "offset", 0)
	if n := len(cardWordIDs(t, data)); n != 2 {
		t.Fatalf("负数 offset 应按 0 处理并返回全部 2 条，实际 %d 条", n)
	}
}

// TestReviewsQueueAcceptsDueAtBeyondNow queue 的到期与否只影响顺序、不影响能否出现：
// 全部卡都远未到期时，queue 仍要给出它们（这是「想多学就能一直往下翻」的前提）。
func TestReviewsQueueAcceptsDueAtBeyondNow(t *testing.T) {
	router, db := setupTestRouter(t)
	wordID := seedWord(t, db, "brisk")
	far := time.Now().Add(365 * 24 * time.Hour)
	seedReview(t, db, wordID, 30, 5, &far, 10, 0)

	queueData, _ := decodeData(t, doJSON(t, router, "GET", "/api/reviews/queue", nil))
	assertIDOrder(t, cardWordIDs(t, queueData), []uint{wordID}, "远未到期的 queue")

	dueData, _ := decodeData(t, doJSON(t, router, "GET", "/api/reviews/due", nil))
	assertIDOrder(t, cardWordIDs(t, dueData), nil, "远未到期的 due 池")
}

// TestReviewsCardFields 取数三个接口复用同一套 dueCard 字段，客户端解析代码不用区分。
// 字段名一旦变更会同时打断 queue / probes / due 三处前端代码，所以逐个钉住。
func TestReviewsCardFields(t *testing.T) {
	router, db := setupTestRouter(t)
	wordID := seedWordWith(t, db, modelsWord())
	now := time.Now().Add(-time.Minute)
	seedReview(t, db, wordID, 4.5, 6.5, &now, 7, 1)

	for _, path := range []string{"/api/reviews/queue", "/api/reviews/probes", "/api/reviews/due"} {
		data, _ := decodeData(t, doJSON(t, router, "GET", path, nil))
		list := items(t, data)
		if len(list) != 1 {
			t.Fatalf("%s 期望 1 张卡，实际 %d 张", path, len(list))
		}
		card := list[0]
		wantKeys := []string{
			"id", "word", "phonetic", "meaning", "example", "example_translation",
			"senses", "subject", "stability", "difficulty", "due_at", "last_review_at", "reps",
		}
		for _, key := range wantKeys {
			if _, ok := card[key]; !ok {
				t.Fatalf("%s 的卡片缺少字段 %q，实际字段=%v", path, key, mapKeys(card))
			}
		}
		if card["word"] != "kernel" {
			t.Fatalf("%s 的卡片 word 期望 kernel，实际 %v", path, card["word"])
		}
		if toFloat(t, card["stability"]) != 4.5 || toFloat(t, card["difficulty"]) != 6.5 {
			t.Fatalf("%s 的记忆状态字段不对: stability=%v difficulty=%v", path, card["stability"], card["difficulty"])
		}
		if toInt64(t, card["reps"]) != 7 {
			t.Fatalf("%s 的 reps 期望 7，实际 %v", path, card["reps"])
		}
		// senses 为空时必须序列化成 []（不是 null）——前端 Rust 引擎按数组解析，遇 null 会报错
		senses, ok := card["senses"].([]interface{})
		if !ok {
			t.Fatalf("%s 的 senses 期望数组（空也必须是 []），实际 %T（值=%v）", path, card["senses"], card["senses"])
		}
		if len(senses) != 0 {
			t.Fatalf("%s 的 senses 期望空数组，实际 %v", path, senses)
		}
	}
}

// modelsWord 造一个字段齐全的词条（cardFields 用例用），避免测试里出现超长字面量
func modelsWord() models.Word {
	return models.Word{
		Word:               "kernel",
		Phonetic:           "/ˈkɜː.nəl/",
		Meaning:            "n. 核心；内核",
		Example:            "The kernel handles interrupts.",
		ExampleTranslation: "内核负责处理中断。",
		Book:               "必修一",
		Unit:               "Unit 1",
	}
}
