use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn parse_github_timestamp_to_unix_seconds(timestamp: &str) -> Option<i64> {
    let (date_part, time_part) = timestamp.split_once('T')?;
    let (year, month, day) = parse_date_parts(date_part)?;
    let (hour, minute, second, tz_offset_seconds) = parse_time_and_offset(time_part)?;

    let days = days_from_civil(year, month, day)?;
    let day_seconds = i64::from(hour) * 3600 + i64::from(minute) * 60 + i64::from(second);
    days.saturating_mul(86_400)
        .checked_add(day_seconds)
        .and_then(|local_seconds| local_seconds.checked_sub(i64::from(tz_offset_seconds)))
}

fn parse_date_parts(date_part: &str) -> Option<(i32, u32, u32)> {
    let mut parts = date_part.split('-');
    let year = parts.next()?.parse::<i32>().ok()?;
    let month = parts.next()?.parse::<u32>().ok()?;
    let day = parts.next()?.parse::<u32>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year, month, day))
}

fn parse_time_and_offset(time_part: &str) -> Option<(u32, u32, u32, i32)> {
    if let Some(clock) = time_part.strip_suffix('Z') {
        let (hour, minute, second) = parse_hms(clock)?;
        return Some((hour, minute, second, 0));
    }

    let tz_pos = time_part.rfind(['+', '-'])?;
    let (clock, offset_part) = time_part.split_at(tz_pos);
    let sign = if offset_part.starts_with('-') { -1 } else { 1 };
    let offset = &offset_part[1..];
    let (offset_hour, offset_minute) = parse_hm(offset)?;
    let tz_offset_seconds = sign * (offset_hour * 3600 + offset_minute * 60);
    let (hour, minute, second) = parse_hms(clock)?;
    Some((hour, minute, second, tz_offset_seconds))
}

fn parse_hms(clock: &str) -> Option<(u32, u32, u32)> {
    let mut parts = clock.split(':');
    let hour = parts.next()?.parse::<u32>().ok()?;
    let minute = parts.next()?.parse::<u32>().ok()?;
    let second_raw = parts.next()?;
    if parts.next().is_some() {
        return None;
    }

    let second_text = second_raw
        .split_once('.')
        .map_or(second_raw, |(sec, _)| sec);
    let second = second_text.parse::<u32>().ok()?;
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    Some((hour, minute, second))
}

fn parse_hm(clock: &str) -> Option<(i32, i32)> {
    let mut parts = clock.split(':');
    let hour = parts.next()?.parse::<i32>().ok()?;
    let minute = parts.next()?.parse::<i32>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    if !(0..=23).contains(&hour) || !(0..=59).contains(&minute) {
        return None;
    }
    Some((hour, minute))
}

fn days_from_civil(year: i32, month: u32, day: u32) -> Option<i64> {
    let adjusted_year = year - i32::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year / 400
    } else {
        (adjusted_year - 399) / 400
    };
    let yoe = adjusted_year - era * 400;
    let month_i32 = i32::try_from(month).ok()?;
    let day_i32 = i32::try_from(day).ok()?;
    let m = month_i32 + if month > 2 { -3 } else { 9 };
    let doy = (153 * m + 2) / 5 + day_i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(i64::from(era) * 146_097 + i64::from(doe) - 719_468)
}

pub(super) fn unix_ts() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}
