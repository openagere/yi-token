use super::{Services, UsageQueryInput, UsageReportView};
use crate::error::{CoreError, CoreResult};
use crate::storage::usage::{self, UsageQuery};
use rusqlite::Connection;
use std::sync::Arc;

const DEFAULT_DAYS: u32 = 7;

/// UTC milliseconds of local midnight of `date` (`YYYY-MM-DD`) shifted by
/// `plus_days`, computed by SQLite so it honours the local time zone (and DST)
/// exactly like the report's day buckets.
fn local_midnight_ms(conn: &Connection, date: &str, plus_days: i64) -> CoreResult<i64> {
    let ms: Option<i64> = conn.query_row(
        "SELECT CAST(strftime('%s', date(?1, ?2), 'utc') AS INTEGER) * 1000",
        rusqlite::params![date, format!("{plus_days:+} days")],
        |r| r.get(0),
    )?;
    ms.ok_or_else(|| CoreError::Validation(format!("日期格式无效：{date}，应为 YYYY-MM-DD")))
}

fn today(conn: &Connection) -> CoreResult<String> {
    Ok(conn.query_row("SELECT date('now', 'localtime')", [], |r| r.get(0))?)
}

fn window(conn: &Connection, input: &UsageQueryInput) -> CoreResult<(i64, i64)> {
    match (&input.start_date, &input.end_date) {
        (Some(start), Some(end)) => {
            let start_ms = local_midnight_ms(conn, start, 0)?;
            let end_ms = local_midnight_ms(conn, end, 1)?;
            if end_ms <= start_ms {
                return Err(CoreError::Validation("结束日期不能早于开始日期".into()));
            }
            Ok((start_ms, end_ms))
        }
        (None, None) => {
            let days = i64::from(input.days.unwrap_or(DEFAULT_DAYS).max(1));
            let today = today(conn)?;
            Ok((
                local_midnight_ms(conn, &today, 1 - days)?,
                local_midnight_ms(conn, &today, 1)?,
            ))
        }
        _ => Err(CoreError::Validation("请同时选择开始日期和结束日期".into())),
    }
}

impl Services {
    pub async fn get_usage(
        self: &Arc<Self>,
        input: UsageQueryInput,
    ) -> CoreResult<UsageReportView> {
        self.run(move |s| {
            s.db.with_conn(|c| {
                let (start_ms, end_ms) = window(c, &input)?;
                let model = input.model.filter(|m| !m.trim().is_empty());
                let query = UsageQuery {
                    start_ms,
                    end_ms,
                    model,
                    client: input.client,
                };
                let report = usage::query(c, query.clone())?;
                let totals = usage::totals(c, &query)?;
                let groups = usage::groups(c, &query, input.group_by.unwrap_or_default())?;
                Ok(UsageReportView {
                    report,
                    totals,
                    groups,
                })
            })
        })
        .await
    }
}
