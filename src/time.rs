use std::time::{SystemTime, UNIX_EPOCH};

/// 本地时区相对 UTC 的偏移秒数（东八区 = +8h）。换时区只改这里。
pub const TZ_OFFSET_SECS: i64 = 8 * 3600;

fn epoch_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

/// 本地时区的当前秒数
pub fn local_secs() -> i64 {
    epoch_secs() + TZ_OFFSET_SECS
}

pub fn sys_now() -> i64 {
    local_secs() / 86_400
}

pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}
