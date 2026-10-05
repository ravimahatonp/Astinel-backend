use redis::{aio::ConnectionManager, RedisResult};

#[derive(Clone)]
pub struct RedisPool {
    pub con: ConnectionManager,
}

impl RedisPool {
    pub async fn new(url: &str) -> RedisResult<Self> {
        let client = redis::Client::open(url)?;
        let con = ConnectionManager::new(client).await?;
        Ok(Self { con })
    }
}

pub struct SessionStore {
    redis: RedisPool,
}

impl SessionStore {
    pub fn new(redis: RedisPool) -> Self {
        Self { redis }
    }

    pub async fn store_refresh_token(&self, jti: &str, ttl_secs: i64) -> RedisResult<()> {
        let key = format!("session:refresh:{}", jti);
        let mut con = self.redis.con.clone();
        redis::cmd("SET")
            .arg(&key)
            .arg("1")
            .arg("EX")
            .arg(ttl_secs)
            .query_async(&mut con)
            .await
    }

    pub async fn is_refresh_token_valid(&self, jti: &str) -> RedisResult<bool> {
        let key = format!("session:refresh:{}", jti);
        let mut con = self.redis.con.clone();
        redis::cmd("EXISTS").arg(&key).query_async(&mut con).await
    }

    pub async fn invalidate_refresh_token(&self, jti: &str) -> RedisResult<()> {
        let key = format!("session:refresh:{}", jti);
        let mut con = self.redis.con.clone();
        redis::cmd("DEL").arg(&key).query_async(&mut con).await
    }
}

pub struct RateLimiter {
    redis: RedisPool,
}

impl RateLimiter {
    pub fn new(redis: RedisPool) -> Self {
        Self { redis }
    }

    pub async fn check_rate_limit(
        &self,
        key: &str,
        max_requests: u64,
        window_secs: u64,
    ) -> RedisResult<bool> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let window_start = now - window_secs as i64;
        let redis_key = format!("ratelimit:{}", key);
        let mut con = self.redis.con.clone();

        let count: u64 = redis::cmd("ZCOUNT")
            .arg(&redis_key)
            .arg(window_start)
            .arg(now)
            .query_async(&mut con)
            .await?;

        if count >= max_requests {
            return Ok(false);
        }

        redis::cmd("ZADD")
            .arg(&redis_key)
            .arg(now)
            .arg(format!("{}:{}", key, now))
            .query_async::<()>(&mut con)
            .await?;

        redis::cmd("EXPIRE")
            .arg(&redis_key)
            .arg(window_secs + 1)
            .query_async::<()>(&mut con)
            .await?;

        Ok(true)
    }
}

pub struct WebhookDedup {
    redis: RedisPool,
}

impl WebhookDedup {
    pub fn new(redis: RedisPool) -> Self {
        Self { redis }
    }

    pub async fn is_duplicate(&self, event_id: &str) -> RedisResult<bool> {
        let key = format!("webhook:dedup:{}", event_id);
        let mut con = self.redis.con.clone();
        let result: Option<String> = redis::cmd("SET")
            .arg(&key)
            .arg("1")
            .arg("NX")
            .arg("EX")
            .arg(3600)
            .query_async(&mut con)
            .await?;
        Ok(result.is_none())
    }
}

#[derive(Clone)]
pub struct ScanStatusCache {
    redis: RedisPool,
}

impl ScanStatusCache {
    pub fn new(redis: RedisPool) -> Self {
        Self { redis }
    }

    pub async fn set_progress(&self, scan_id: &str, progress: u8, phase: &str) -> RedisResult<()> {
        let key = format!("scan:{}:progress", scan_id);
        let mut con = self.redis.con.clone();
        let value = serde_json::json!({"p": progress, "ph": phase}).to_string();
        redis::cmd("SETEX")
            .arg(&key)
            .arg(86400)
            .arg(&value)
            .query_async(&mut con)
            .await
    }

    pub async fn get_progress(&self, scan_id: &str) -> RedisResult<(u8, String)> {
        let key = format!("scan:{}:progress", scan_id);
        let mut con = self.redis.con.clone();
        let value: Option<String> = redis::cmd("GET").arg(&key).query_async(&mut con).await?;
        match value {
            Some(json_str) => {
                let v: serde_json::Value = serde_json::from_str(&json_str)
                    .unwrap_or(serde_json::json!({"p": 0, "ph": "unknown"}));
                let progress = v["p"].as_u64().unwrap_or(0) as u8;
                let phase = v["ph"].as_str().unwrap_or("unknown").to_string();
                Ok((progress, phase))
            }
            None => Ok((0, "unknown".to_string())),
        }
    }

    pub async fn mark_cancelled(&self, scan_id: &str) -> RedisResult<()> {
        let key = format!("scan:{}:cancelled", scan_id);
        let mut con = self.redis.con.clone();
        redis::cmd("SET")
            .arg(&key)
            .arg("1")
            .arg("EX")
            .arg(86400)
            .query_async(&mut con)
            .await
    }

    pub async fn is_cancelled(&self, scan_id: &str) -> RedisResult<bool> {
        let key = format!("scan:{}:cancelled", scan_id);
        let mut con = self.redis.con.clone();
        redis::cmd("EXISTS").arg(&key).query_async(&mut con).await
    }
}
