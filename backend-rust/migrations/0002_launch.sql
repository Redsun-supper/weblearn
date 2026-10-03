-- 0002_launch.sql —— P0-5：三级角色与邀请码分级
--
-- 用户已定案的 13 条决策里，与本次迁移有关的两条：
--   ① 邀请码只能给 user / admin 两级（**不设超管码**：一张码就能再造一个能封你号的人）；
--   ② 库里的**存量邀请码全部停用**（它们没有等级，留着就等于一批身份不明的凭证）。
--
-- ⚠️ 第二条会让手头已有的明文码当场失效 —— 这是有意为之：
--    迁移后请用 `cargo run --bin invite -- create --grant-role admin` 重新发。
--    代码保留在库里（只是 disabled=1），所以还能从列表里看出「这批码是谁什么时候发的」。

ALTER TABLE invite_codes ADD COLUMN grant_role TEXT NOT NULL DEFAULT 'user';
ALTER TABLE invite_codes ADD COLUMN batch_id   TEXT NOT NULL DEFAULT '';

-- 存量码没有等级信息，一律停用（`WHERE disabled = 0` 让本行可重复执行而不改变语义）
UPDATE invite_codes SET disabled = 1 WHERE disabled = 0;

-- 按等级 / 批次筛选是后台列表的常用查询
CREATE INDEX IF NOT EXISTS idx_invite_codes_batch ON invite_codes(batch_id);
CREATE INDEX IF NOT EXISTS idx_invite_codes_role  ON invite_codes(grant_role);
