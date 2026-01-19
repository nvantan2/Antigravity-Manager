use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Token usage statistics record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsageRecord {
    pub id: i64,
    pub timestamp: i64,
    pub account_email: String,
    pub model: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
}

/// Aggregated token statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenStatsAggregated {
    pub period: String,           // e.g., "2024-01-15 14:00" for hourly, "2024-01-15" for daily
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tokens: u64,
    pub request_count: u64,
}

/// Per-account token statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountTokenStats {
    pub account_email: String,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tokens: u64,
    pub request_count: u64,
}

/// Summary statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenStatsSummary {
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub total_tokens: u64,
    pub total_requests: u64,
    pub unique_accounts: u64,
}

fn get_db_path() -> Result<PathBuf, String> {
    let data_dir = crate::modules::account::get_data_dir()?;
    Ok(data_dir.join("token_stats.db"))
}

fn connect_db() -> Result<Connection, String> {
    let db_path = get_db_path()?;
    let conn = Connection::open(db_path).map_err(|e| e.to_string())?;
    
    // Enable WAL mode for better concurrency
    conn.pragma_update(None, "journal_mode", "WAL").map_err(|e| e.to_string())?;
    conn.pragma_update(None, "busy_timeout", 5000).map_err(|e| e.to_string())?;
    conn.pragma_update(None, "synchronous", "NORMAL").map_err(|e| e.to_string())?;
    
    Ok(conn)
}

/// Initialize the token stats database
pub fn init_db() -> Result<(), String> {
    let conn = connect_db()?;
    
    // Create main usage table
    conn.execute(
        "CREATE TABLE IF NOT EXISTS token_usage (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp INTEGER NOT NULL,
            account_email TEXT NOT NULL,
            model TEXT NOT NULL,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0
        )",
        [],
    ).map_err(|e| e.to_string())?;
    
    // Create indexes for efficient queries
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_token_timestamp ON token_usage (timestamp DESC)",
        [],
    ).map_err(|e| e.to_string())?;
    
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_token_account ON token_usage (account_email)",
        [],
    ).map_err(|e| e.to_string())?;
    
    // Create hourly aggregation table for fast queries
    conn.execute(
        "CREATE TABLE IF NOT EXISTS token_stats_hourly (
            hour_bucket TEXT NOT NULL,
            account_email TEXT NOT NULL,
            total_input_tokens INTEGER NOT NULL DEFAULT 0,
            total_output_tokens INTEGER NOT NULL DEFAULT 0,
            total_tokens INTEGER NOT NULL DEFAULT 0,
            request_count INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (hour_bucket, account_email)
        )",
        [],
    ).map_err(|e| e.to_string())?;
    
    Ok(())
}

/// Record token usage from a request
pub fn record_usage(
    account_email: &str,
    model: &str,
    input_tokens: u32,
    output_tokens: u32,
) -> Result<(), String> {
    let conn = connect_db()?;
    let timestamp = chrono::Utc::now().timestamp();
    let total_tokens = input_tokens + output_tokens;
    
    // Insert into raw usage table
    conn.execute(
        "INSERT INTO token_usage (timestamp, account_email, model, input_tokens, output_tokens, total_tokens)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![timestamp, account_email, model, input_tokens, output_tokens, total_tokens],
    ).map_err(|e| e.to_string())?;
    

    let hour_bucket = chrono::Utc::now().format("%Y-%m-%d %H:00").to_string();
    conn.execute(
        "INSERT INTO token_stats_hourly (hour_bucket, account_email, total_input_tokens, total_output_tokens, total_tokens, request_count)
         VALUES (?1, ?2, ?3, ?4, ?5, 1)
         ON CONFLICT(hour_bucket, account_email) DO UPDATE SET
            total_input_tokens = total_input_tokens + ?3,
            total_output_tokens = total_output_tokens + ?4,
            total_tokens = total_tokens + ?5,
            request_count = request_count + 1",
        params![hour_bucket, account_email, input_tokens, output_tokens, total_tokens],
    ).map_err(|e| e.to_string())?;
    
    Ok(())
}

/// Get hourly aggregated stats for a time range
pub fn get_hourly_stats(hours: i64) -> Result<Vec<TokenStatsAggregated>, String> {
    let conn = connect_db()?;
    let cutoff = chrono::Utc::now() - chrono::Duration::hours(hours);
    let cutoff_bucket = cutoff.format("%Y-%m-%d %H:00").to_string();
    
    let mut stmt = conn.prepare(
        "SELECT hour_bucket, 
                SUM(total_input_tokens) as input, 
                SUM(total_output_tokens) as output,
                SUM(total_tokens) as total,
                SUM(request_count) as count
         FROM token_stats_hourly 
         WHERE hour_bucket >= ?1
         GROUP BY hour_bucket
         ORDER BY hour_bucket ASC"
    ).map_err(|e| e.to_string())?;
    
    let rows = stmt.query_map([cutoff_bucket], |row| {
        Ok(TokenStatsAggregated {
            period: row.get(0)?,
            total_input_tokens: row.get(1)?,
            total_output_tokens: row.get(2)?,
            total_tokens: row.get(3)?,
            request_count: row.get(4)?,
        })
    }).map_err(|e| e.to_string())?;
    
    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| e.to_string())?);
    }
    Ok(result)
}

/// Get daily aggregated stats for a time range
pub fn get_daily_stats(days: i64) -> Result<Vec<TokenStatsAggregated>, String> {
    let conn = connect_db()?;
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days);
    let cutoff_bucket = cutoff.format("%Y-%m-%d").to_string();
    
    let mut stmt = conn.prepare(
        "SELECT substr(hour_bucket, 1, 10) as day_bucket, 
                SUM(total_input_tokens) as input, 
                SUM(total_output_tokens) as output,
                SUM(total_tokens) as total,
                SUM(request_count) as count
         FROM token_stats_hourly 
         WHERE substr(hour_bucket, 1, 10) >= ?1
         GROUP BY day_bucket
         ORDER BY day_bucket ASC"
    ).map_err(|e| e.to_string())?;
    
    let rows = stmt.query_map([cutoff_bucket], |row| {
        Ok(TokenStatsAggregated {
            period: row.get(0)?,
            total_input_tokens: row.get(1)?,
            total_output_tokens: row.get(2)?,
            total_tokens: row.get(3)?,
            request_count: row.get(4)?,
        })
    }).map_err(|e| e.to_string())?;
    
    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| e.to_string())?);
    }
    Ok(result)
}

/// Get weekly aggregated stats
pub fn get_weekly_stats(weeks: i64) -> Result<Vec<TokenStatsAggregated>, String> {
    let conn = connect_db()?;
    let cutoff = chrono::Utc::now() - chrono::Duration::weeks(weeks);
    let cutoff_timestamp = cutoff.timestamp();
    
    let mut stmt = conn.prepare(
        "SELECT strftime('%Y-W%W', datetime(timestamp, 'unixepoch')) as week_bucket,
                SUM(input_tokens) as input, 
                SUM(output_tokens) as output,
                SUM(total_tokens) as total,
                COUNT(*) as count
         FROM token_usage 
         WHERE timestamp >= ?1
         GROUP BY week_bucket
         ORDER BY week_bucket ASC"
    ).map_err(|e| e.to_string())?;
    
    let rows = stmt.query_map([cutoff_timestamp], |row| {
        Ok(TokenStatsAggregated {
            period: row.get(0)?,
            total_input_tokens: row.get(1)?,
            total_output_tokens: row.get(2)?,
            total_tokens: row.get(3)?,
            request_count: row.get(4)?,
        })
    }).map_err(|e| e.to_string())?;
    
    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| e.to_string())?);
    }
    Ok(result)
}

/// Get per-account statistics for a time range
pub fn get_account_stats(hours: i64) -> Result<Vec<AccountTokenStats>, String> {
    let conn = connect_db()?;
    let cutoff = chrono::Utc::now() - chrono::Duration::hours(hours);
    let cutoff_bucket = cutoff.format("%Y-%m-%d %H:00").to_string();
    
    let mut stmt = conn.prepare(
        "SELECT account_email,
                SUM(total_input_tokens) as input, 
                SUM(total_output_tokens) as output,
                SUM(total_tokens) as total,
                SUM(request_count) as count
         FROM token_stats_hourly 
         WHERE hour_bucket >= ?1
         GROUP BY account_email
         ORDER BY total DESC"
    ).map_err(|e| e.to_string())?;
    
    let rows = stmt.query_map([cutoff_bucket], |row| {
        Ok(AccountTokenStats {
            account_email: row.get(0)?,
            total_input_tokens: row.get(1)?,
            total_output_tokens: row.get(2)?,
            total_tokens: row.get(3)?,
            request_count: row.get(4)?,
        })
    }).map_err(|e| e.to_string())?;
    
    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|e| e.to_string())?);
    }
    Ok(result)
}

/// Get summary statistics for a time range
pub fn get_summary_stats(hours: i64) -> Result<TokenStatsSummary, String> {
    let conn = connect_db()?;
    let cutoff = chrono::Utc::now() - chrono::Duration::hours(hours);
    let cutoff_bucket = cutoff.format("%Y-%m-%d %H:00").to_string();
    
    let (total_input, total_output, total, requests): (u64, u64, u64, u64) = conn.query_row(
        "SELECT COALESCE(SUM(total_input_tokens), 0),
                COALESCE(SUM(total_output_tokens), 0),
                COALESCE(SUM(total_tokens), 0),
                COALESCE(SUM(request_count), 0)
         FROM token_stats_hourly 
         WHERE hour_bucket >= ?1",
        [&cutoff_bucket],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    ).map_err(|e| e.to_string())?;
    
    let unique_accounts: u64 = conn.query_row(
        "SELECT COUNT(DISTINCT account_email) FROM token_stats_hourly WHERE hour_bucket >= ?1",
        [&cutoff_bucket],
        |row| row.get(0),
    ).map_err(|e| e.to_string())?;
    
    Ok(TokenStatsSummary {
        total_input_tokens: total_input,
        total_output_tokens: total_output,
        total_tokens: total,
        total_requests: requests,
        unique_accounts,
    })
}

/// Clean up old data (keep last N days of raw data)
pub fn cleanup_old_data(days: i64) -> Result<usize, String> {
    let conn = connect_db()?;
    let cutoff = chrono::Utc::now().timestamp() - (days * 24 * 3600);
    
    let deleted = conn.execute(
        "DELETE FROM token_usage WHERE timestamp < ?1",
        [cutoff],
    ).map_err(|e| e.to_string())?;
    
    // Keep hourly aggregations for longer (90 days)
    let cutoff_bucket = (chrono::Utc::now() - chrono::Duration::days(90))
        .format("%Y-%m-%d %H:00")
        .to_string();
    conn.execute(
        "DELETE FROM token_stats_hourly WHERE hour_bucket < ?1",
        [cutoff_bucket],
    ).map_err(|e| e.to_string())?;
    
    conn.execute("VACUUM", []).map_err(|e| e.to_string())?;
    
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_record_and_query() {
        // This would need a test database setup
        // For now, just verify the module compiles
        assert!(true);
    }
}
