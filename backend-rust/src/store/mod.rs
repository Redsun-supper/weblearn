// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
//! 存储层：SQLite 连接 + 事务边界 + SQL 语句
//!
//! 为什么是「一个连接 + 互斥锁」而不是连接池：
//!   - SQLite 的写操作本来就是串行的，池化在这里并不能提高写并发；
//!   - 服务本身的并发量（个人站点的注册/登录）远低于锁竞争的门槛；
//!   - 少一层池依赖，行为更好推理。
//! 所有阻塞的数据库调用都放进 `spawn_blocking`，不会卡住 tokio 的异步调度。
//!
//! ⚠️ **不要在持有数据库锁的时候做昂贵计算**（Argon2 哈希、发信、网络请求）：
//! 连接是全局串行的，一个 50ms 的哈希会把所有请求一起堵住。服务层因此把
//! 「算哈希」放在事务之外。
//!
//! `read` / `write` 的错误类型是泛型的（`E: From<StoreError>`），这样服务层可以
//! 在事务闭包里直接返回 `AuthError`，业务错误与数据库错误用同一个 `?` 往上抛。

pub mod sql;

use std::sync::{Arc, Mutex};

use rusqlite::{Connection, Transaction};

use crate::error::StoreError;

pub type StoreResult<T> = std::result::Result<T, StoreError>;

#[derive(Clone)]
pub struct SqliteStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteStore {
    /// 打开（或创建）数据库并自动应用迁移
    pub fn open(path: &str) -> StoreResult<Self> {
        let mut conn = crate::db::open_connection(path)?;
        crate::db::migrate(&mut conn)?;
        Ok(Self { conn: Arc::new(Mutex::new(conn)) })
    }

    /// 只读查询
    pub async fn read<T, E, F>(&self, f: F) -> std::result::Result<T, E>
    where
        F: FnOnce(&Connection) -> std::result::Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<StoreError> + Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> std::result::Result<T, E> {
            let guard = conn
                .lock()
                .map_err(|_| E::from(StoreError::Internal("数据库连接锁被毒化".into())))?;
            f(&guard)
        })
        .await
        .map_err(|e| E::from(StoreError::Task(e.to_string())))?
    }

    /// 事务写：闭包返回 `Ok` 才提交，返回 `Err` 自动回滚
    pub async fn write<T, E, F>(&self, f: F) -> std::result::Result<T, E>
    where
        F: FnOnce(&Connection) -> std::result::Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: From<StoreError> + Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> std::result::Result<T, E> {
            let mut guard = conn
                .lock()
                .map_err(|_| E::from(StoreError::Internal("数据库连接锁被毒化".into())))?;
            let tx: Transaction<'_> = guard.transaction().map_err(StoreError::from).map_err(E::from)?;
            let out = f(&tx)?;
            tx.commit().map_err(StoreError::from).map_err(E::from)?;
            Ok(out)
        })
        .await
        .map_err(|e| E::from(StoreError::Task(e.to_string())))?
    }
}

impl std::fmt::Debug for SqliteStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteStore").finish_non_exhaustive()
    }
}
