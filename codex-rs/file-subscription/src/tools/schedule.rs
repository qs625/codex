use std::time::Duration;

use chrono::DateTime;
use chrono::Datelike;
use chrono::Days;
use chrono::LocalResult;
use chrono::NaiveTime;
use chrono::TimeZone;
use chrono::Timelike;
use chrono::Utc;
use chrono_tz::Tz;
use codex_protocol::subscriptions::ScheduleSpec;
use codex_protocol::subscriptions::ScheduleWeekday;

#[derive(Clone, Debug)]
pub(crate) enum CompiledSchedule {
    OnceAt(DateTime<Utc>),
    EveryInterval(Duration),
    EveryDayAt {
        time: NaiveTime,
        timezone: Tz,
    },
    EveryWeekAt {
        weekdays: Vec<ScheduleWeekday>,
        time: NaiveTime,
        timezone: Tz,
    },
}

impl CompiledSchedule {
    pub(crate) fn compile(spec: ScheduleSpec) -> Result<Self, String> {
        match spec {
            ScheduleSpec::OnceAfter { delay_ms } => {
                if delay_ms == 0 {
                    return Err("delay_ms must be greater than zero".to_string());
                }
                let delay = chrono::Duration::from_std(Duration::from_millis(delay_ms))
                    .map_err(|_| "delay_ms is too large".to_string())?;
                Ok(Self::OnceAt(Utc::now() + delay))
            }
            ScheduleSpec::OnceAt { run_at } => {
                let run_at = DateTime::parse_from_rfc3339(&run_at)
                    .map_err(|err| format!("run_at must be RFC 3339: {err}"))?
                    .with_timezone(&Utc);
                if run_at <= Utc::now() {
                    return Err("run_at must be in the future".to_string());
                }
                Ok(Self::OnceAt(run_at))
            }
            ScheduleSpec::EveryInterval { interval_ms } => {
                if interval_ms == 0 {
                    return Err("interval_ms must be greater than zero".to_string());
                }
                Ok(Self::EveryInterval(Duration::from_millis(interval_ms)))
            }
            ScheduleSpec::EveryDayAt { time, timezone } => Ok(Self::EveryDayAt {
                time: parse_time(&time)?,
                timezone: parse_timezone(&timezone)?,
            }),
            ScheduleSpec::EveryWeekAt {
                weekdays,
                time,
                timezone,
            } => {
                if weekdays.is_empty() {
                    return Err("weekdays must not be empty".to_string());
                }
                Ok(Self::EveryWeekAt {
                    weekdays,
                    time: parse_time(&time)?,
                    timezone: parse_timezone(&timezone)?,
                })
            }
        }
    }

    pub(crate) fn summary(&self) -> String {
        match self {
            Self::OnceAt(run_at) => format!("once at {}", run_at.to_rfc3339()),
            Self::EveryInterval(interval) => {
                format!("every {} ms", interval.as_millis())
            }
            Self::EveryDayAt { time, timezone } => {
                format!(
                    "every day at {} {}",
                    time.format("%H:%M:%S"),
                    timezone.name()
                )
            }
            Self::EveryWeekAt {
                weekdays,
                time,
                timezone,
            } => {
                let weekdays = weekdays
                    .iter()
                    .map(weekday_name)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "every week on {weekdays} at {} {}",
                    time.format("%H:%M:%S"),
                    timezone.name()
                )
            }
        }
    }

    pub(crate) fn next_fire_at(&self, after: DateTime<Utc>) -> Result<DateTime<Utc>, String> {
        match self {
            Self::OnceAt(run_at) => Ok(*run_at),
            Self::EveryInterval(interval) => {
                let delay = chrono::Duration::from_std(*interval)
                    .map_err(|_| "interval is too large".to_string())?;
                Ok(after + delay)
            }
            Self::EveryDayAt { time, timezone } => next_daily_fire(after, *time, *timezone),
            Self::EveryWeekAt {
                weekdays,
                time,
                timezone,
            } => next_weekly_fire(after, weekdays, *time, *timezone),
        }
    }

    pub(crate) fn is_one_shot(&self) -> bool {
        matches!(self, Self::OnceAt(_))
    }
}

fn parse_time(value: &str) -> Result<NaiveTime, String> {
    NaiveTime::parse_from_str(value, "%H:%M:%S")
        .or_else(|_| NaiveTime::parse_from_str(value, "%H:%M"))
        .map_err(|err| format!("time must be HH:MM or HH:MM:SS: {err}"))
}

fn parse_timezone(value: &str) -> Result<Tz, String> {
    value
        .parse::<Tz>()
        .map_err(|_| format!("timezone must be a valid IANA timezone: {value}"))
}

fn next_daily_fire(
    after: DateTime<Utc>,
    time: NaiveTime,
    timezone: Tz,
) -> Result<DateTime<Utc>, String> {
    let after_local = after.with_timezone(&timezone);
    for offset_days in 0..=366 {
        let date = after_local
            .date_naive()
            .checked_add_days(Days::new(offset_days))
            .ok_or_else(|| "failed to compute next daily schedule date".to_string())?;
        if let Some(candidate) = resolve_local_datetime(timezone, date, time)
            && candidate > after
        {
            return Ok(candidate);
        }
    }
    Err("failed to find the next daily schedule occurrence".to_string())
}

fn next_weekly_fire(
    after: DateTime<Utc>,
    weekdays: &[ScheduleWeekday],
    time: NaiveTime,
    timezone: Tz,
) -> Result<DateTime<Utc>, String> {
    let after_local = after.with_timezone(&timezone);
    for offset_days in 0..=14 {
        let date = after_local
            .date_naive()
            .checked_add_days(Days::new(offset_days))
            .ok_or_else(|| "failed to compute next weekly schedule date".to_string())?;
        let weekday_number = date.weekday().number_from_monday();
        if !weekdays
            .iter()
            .any(|weekday| weekday_number_from_monday(weekday) == weekday_number)
        {
            continue;
        }
        if let Some(candidate) = resolve_local_datetime(timezone, date, time)
            && candidate > after
        {
            return Ok(candidate);
        }
    }
    Err("failed to find the next weekly schedule occurrence".to_string())
}

fn resolve_local_datetime(
    timezone: Tz,
    date: chrono::NaiveDate,
    time: NaiveTime,
) -> Option<DateTime<Utc>> {
    match timezone.with_ymd_and_hms(
        date.year(),
        date.month(),
        date.day(),
        time.hour(),
        time.minute(),
        time.second(),
    ) {
        LocalResult::Single(dt) => Some(dt.with_timezone(&Utc)),
        LocalResult::Ambiguous(first, second) => Some(first.min(second).with_timezone(&Utc)),
        LocalResult::None => None,
    }
}

fn weekday_name(weekday: &ScheduleWeekday) -> &'static str {
    match weekday {
        ScheduleWeekday::Mon => "monday",
        ScheduleWeekday::Tue => "tuesday",
        ScheduleWeekday::Wed => "wednesday",
        ScheduleWeekday::Thu => "thursday",
        ScheduleWeekday::Fri => "friday",
        ScheduleWeekday::Sat => "saturday",
        ScheduleWeekday::Sun => "sunday",
    }
}

fn weekday_number_from_monday(weekday: &ScheduleWeekday) -> u32 {
    match weekday {
        ScheduleWeekday::Mon => 1,
        ScheduleWeekday::Tue => 2,
        ScheduleWeekday::Wed => 3,
        ScheduleWeekday::Thu => 4,
        ScheduleWeekday::Fri => 5,
        ScheduleWeekday::Sat => 6,
        ScheduleWeekday::Sun => 7,
    }
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use chrono::Utc;
    use codex_protocol::subscriptions::ScheduleSpec;
    use codex_protocol::subscriptions::ScheduleWeekday;
    use pretty_assertions::assert_eq;

    use super::CompiledSchedule;

    #[test]
    fn compiles_every_week_at_and_computes_next_fire() {
        let compiled = CompiledSchedule::compile(ScheduleSpec::EveryWeekAt {
            weekdays: vec![ScheduleWeekday::Tue, ScheduleWeekday::Thu],
            time: "09:30".to_string(),
            timezone: "Asia/Shanghai".to_string(),
        })
        .expect("schedule should compile");

        let after = DateTime::parse_from_rfc3339("2026-05-27T00:00:00Z")
            .expect("valid timestamp")
            .with_timezone(&Utc);

        let next_fire = compiled
            .next_fire_at(after)
            .expect("next fire should exist");
        assert_eq!(next_fire.to_rfc3339(), "2026-05-28T01:30:00+00:00");
    }

    #[test]
    fn rejects_empty_weekdays() {
        let error = CompiledSchedule::compile(ScheduleSpec::EveryWeekAt {
            weekdays: vec![],
            time: "09:30".to_string(),
            timezone: "Asia/Shanghai".to_string(),
        })
        .expect_err("empty weekday list should fail");

        assert_eq!(error, "weekdays must not be empty");
    }
}
