//! `time_now`：返回当前时间，演示无参数工具最小实现。

use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::AppResult;
use crate::tool::Tool;

pub struct TimeNow;

#[async_trait]
impl Tool for TimeNow {
    fn name(&self) -> &str {
        "time_now"
    }

    fn description(&self) -> &str {
        "Get the current UTC time as Unix timestamp (seconds) and a human-readable UTC string. No parameters required."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn invoke(&self, _args: Value) -> AppResult<String> {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let (y, mo, d, h, mi, s) = unix_to_utc_ymdhms(secs as i64);
        let pretty = format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            y, mo, d, h, mi, s
        );
        Ok(json!({ "unix": secs, "utc": pretty }).to_string())
    }
}

/// 将 Unix 秒数转换为 UTC 年月日时分秒（民用日历，1970~2099 范围内可用）。
fn unix_to_utc_ymdhms(secs: i64) -> (i32, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400) as u32;
    let h = rem / 3600;
    let mi = (rem % 3600) / 60;
    let s = rem % 60;

    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i32 + era as i32 * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    (y, mo, d, h, mi, s)
}
