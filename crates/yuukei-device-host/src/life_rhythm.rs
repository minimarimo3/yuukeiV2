use super::*;

#[derive(Clone, Debug, Default)]
pub(crate) struct PresenceState {
    pub(crate) startup_emitted: bool,
    pub(crate) last_time_period: Option<String>,
    next_life_tick_at: Option<DateTime<Utc>>,
    pub(crate) last_user_activity_at: Option<DateTime<Utc>>,
    pub(crate) talk_interval_minutes: Option<u64>,
    pub(crate) next_talk_impulse_at: Option<DateTime<Utc>>,
    pub(crate) talk_rng_state: u64,
    idle_active: bool,
    last_idle_elapsed_seconds: Option<f64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocalTimePeriod {
    Morning,
    Day,
    Evening,
    LateNight,
}

impl LocalTimePeriod {
    pub fn as_daihon_value(self) -> &'static str {
        match self {
            Self::Morning => "朝",
            Self::Day => "昼",
            Self::Evening => "夜",
            Self::LateNight => "深夜",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TalkImpulseDecision {
    Disabled,
    Waiting,
    SkippedRecentActivity,
    Emit,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum IdlePresenceDecision {
    Start {
        threshold_seconds: u64,
    },
    End {
        idle_seconds: u64,
        idle_minutes: u64,
    },
}

pub(crate) fn evaluate_life_tick(now: DateTime<Utc>, state: &mut PresenceState) -> bool {
    let next = state
        .next_life_tick_at
        .get_or_insert_with(|| add_duration(now, PRESENCE_LIFE_TICK_INTERVAL).unwrap_or(now));
    if now < *next {
        return false;
    }
    state.next_life_tick_at = Some(add_duration(now, PRESENCE_LIFE_TICK_INTERVAL).unwrap_or(now));
    true
}

pub(crate) fn evaluate_talk_impulse(
    now: DateTime<Utc>,
    interval_minutes: u64,
    random_permyriad: u16,
    state: &mut PresenceState,
) -> TalkImpulseDecision {
    if interval_minutes == 0 {
        state.talk_interval_minutes = Some(0);
        state.next_talk_impulse_at = None;
        return TalkImpulseDecision::Disabled;
    }

    if state.talk_interval_minutes != Some(interval_minutes) || state.next_talk_impulse_at.is_none()
    {
        state.talk_interval_minutes = Some(interval_minutes);
        state.next_talk_impulse_at = Some(schedule_next_talk_impulse(
            now,
            interval_minutes,
            random_permyriad,
        ));
        return TalkImpulseDecision::Waiting;
    }

    let Some(next_due) = state.next_talk_impulse_at else {
        return TalkImpulseDecision::Waiting;
    };
    if now < next_due {
        return TalkImpulseDecision::Waiting;
    }

    state.next_talk_impulse_at = Some(schedule_next_talk_impulse(
        now,
        interval_minutes,
        random_permyriad,
    ));
    if recently_active(now, state.last_user_activity_at) {
        return TalkImpulseDecision::SkippedRecentActivity;
    }
    TalkImpulseDecision::Emit
}

pub(crate) fn evaluate_idle_presence(
    idle_seconds_since_last_input: Option<f64>,
    state: &mut PresenceState,
) -> Option<IdlePresenceDecision> {
    let idle_seconds = idle_seconds_since_last_input?;
    if !idle_seconds.is_finite() || idle_seconds < 0.0 {
        return None;
    }

    let threshold_seconds = PRESENCE_IDLE_THRESHOLD.as_secs();
    if idle_seconds >= threshold_seconds as f64 {
        state.last_idle_elapsed_seconds = Some(
            state
                .last_idle_elapsed_seconds
                .map_or(idle_seconds, |previous| previous.max(idle_seconds)),
        );
        if state.idle_active {
            return None;
        }
        state.idle_active = true;
        return Some(IdlePresenceDecision::Start { threshold_seconds });
    }

    if !state.idle_active {
        state.last_idle_elapsed_seconds = None;
        return None;
    }

    state.idle_active = false;
    let idle_seconds = state
        .last_idle_elapsed_seconds
        .take()
        .unwrap_or(threshold_seconds as f64)
        .floor()
        .max(0.0) as u64;
    Some(IdlePresenceDecision::End {
        idle_seconds,
        idle_minutes: idle_seconds / 60,
    })
}

fn recently_active(now: DateTime<Utc>, last_user_activity_at: Option<DateTime<Utc>>) -> bool {
    let Some(last) = last_user_activity_at else {
        return false;
    };
    match now.signed_duration_since(last).to_std() {
        Ok(elapsed) => elapsed < TALK_IMPULSE_RECENT_ACTIVITY_SUPPRESSION,
        Err(_) => true,
    }
}

fn schedule_next_talk_impulse(
    now: DateTime<Utc>,
    interval_minutes: u64,
    random_permyriad: u16,
) -> DateTime<Utc> {
    let duration = jittered_talk_interval(interval_minutes, random_permyriad);
    add_duration(now, duration).unwrap_or(now)
}

pub(crate) fn jittered_talk_interval(interval_minutes: u64, random_permyriad: u16) -> Duration {
    let base_secs = interval_minutes.saturating_mul(60).max(1);
    let min_secs = base_secs.saturating_mul(80) / 100;
    let spread_secs = (base_secs.saturating_mul(40) / 100).max(1);
    let random = u64::from(random_permyriad.min(9_999));
    Duration::from_secs(min_secs + (spread_secs * random / 9_999))
}

fn add_duration(at: DateTime<Utc>, duration: Duration) -> Option<DateTime<Utc>> {
    chrono::Duration::from_std(duration)
        .ok()
        .and_then(|duration| at.checked_add_signed(duration))
}

pub(crate) fn next_rng_permyriad(state: &mut u64, now: DateTime<Utc>) -> u16 {
    if *state == 0 {
        *state = (now.timestamp_nanos_opt().unwrap_or_default() as u64) ^ 0xA5A5_5A5A_D3C1_B2E0;
    }
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    (x % 10_000) as u16
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PresenceSnapshot {
    local_hour: u32,
    local_minute: u32,
    pub(crate) time_period: &'static str,
}

impl PresenceSnapshot {
    pub(crate) fn into_payload(self) -> JsonMap {
        JsonMap::from([
            ("localHour".to_string(), json!(self.local_hour)),
            ("localMinute".to_string(), json!(self.local_minute)),
            ("timePeriod".to_string(), json!(self.time_period)),
        ])
    }
}

pub(crate) fn current_presence_payload() -> JsonMap {
    current_presence_snapshot().into_payload()
}

pub(crate) fn current_presence_snapshot() -> PresenceSnapshot {
    presence_snapshot_at(Local::now())
}

pub(crate) fn presence_snapshot_at(now: DateTime<Local>) -> PresenceSnapshot {
    let local_hour = now.hour();
    PresenceSnapshot {
        local_hour,
        local_minute: now.minute(),
        time_period: time_period_for_hour(local_hour).as_daihon_value(),
    }
}

pub fn time_period_for_hour(hour: u32) -> LocalTimePeriod {
    match hour {
        5..=9 => LocalTimePeriod::Morning,
        10..=16 => LocalTimePeriod::Day,
        17..=21 => LocalTimePeriod::Evening,
        _ => LocalTimePeriod::LateNight,
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn talk_impulse_jitter_stays_within_twenty_percent() {
        assert_eq!(jittered_talk_interval(5, 0), Duration::from_secs(240));
        assert_eq!(jittered_talk_interval(5, 9_999), Duration::from_secs(360));
    }

    #[test]
    fn talk_impulse_evaluation_disables_schedules_skips_and_emits() {
        let now = DateTime::parse_from_rfc3339("2026-07-06T12:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let mut state = PresenceState::default();

        assert_eq!(
            evaluate_talk_impulse(now, 0, 0, &mut state),
            TalkImpulseDecision::Disabled
        );
        assert_eq!(state.next_talk_impulse_at, None);

        assert_eq!(
            evaluate_talk_impulse(now, 5, 0, &mut state),
            TalkImpulseDecision::Waiting
        );
        assert_eq!(
            state.next_talk_impulse_at,
            Some(now + chrono::Duration::seconds(240))
        );

        state.last_user_activity_at = Some(now + chrono::Duration::seconds(200));
        assert_eq!(
            evaluate_talk_impulse(now + chrono::Duration::seconds(240), 5, 9_999, &mut state),
            TalkImpulseDecision::SkippedRecentActivity
        );
        assert_eq!(
            state.next_talk_impulse_at,
            Some(now + chrono::Duration::seconds(240 + 360))
        );

        state.last_user_activity_at = Some(now);
        assert_eq!(
            evaluate_talk_impulse(now + chrono::Duration::seconds(600), 5, 0, &mut state),
            TalkImpulseDecision::Emit
        );
    }

    #[test]
    fn talk_impulse_setting_change_reschedules_without_emitting() {
        let now = DateTime::parse_from_rfc3339("2026-07-06T12:00:00+00:00")
            .unwrap()
            .with_timezone(&Utc);
        let mut state = PresenceState {
            talk_interval_minutes: Some(5),
            next_talk_impulse_at: Some(now - chrono::Duration::seconds(1)),
            ..PresenceState::default()
        };

        assert_eq!(
            evaluate_talk_impulse(now, 10, 0, &mut state),
            TalkImpulseDecision::Waiting
        );
        assert_eq!(state.talk_interval_minutes, Some(10));
        assert_eq!(
            state.next_talk_impulse_at,
            Some(now + chrono::Duration::seconds(480))
        );
    }

    #[test]
    fn idle_presence_evaluation_emits_start_and_end_once() {
        let mut state = PresenceState::default();

        assert_eq!(evaluate_idle_presence(Some(299.0), &mut state), None);
        assert_eq!(
            evaluate_idle_presence(Some(300.0), &mut state),
            Some(IdlePresenceDecision::Start {
                threshold_seconds: 300
            })
        );
        assert_eq!(evaluate_idle_presence(Some(301.9), &mut state), None);
        assert_eq!(
            evaluate_idle_presence(Some(1.0), &mut state),
            Some(IdlePresenceDecision::End {
                idle_seconds: 301,
                idle_minutes: 5
            })
        );
        assert_eq!(evaluate_idle_presence(Some(0.5), &mut state), None);
    }

    #[test]
    fn idle_presence_evaluation_ignores_unavailable_input() {
        let mut state = PresenceState::default();

        assert_eq!(evaluate_idle_presence(None, &mut state), None);
        assert_eq!(evaluate_idle_presence(Some(f64::NAN), &mut state), None);
        assert_eq!(evaluate_idle_presence(Some(-1.0), &mut state), None);
        assert_eq!(
            evaluate_idle_presence(Some(300.0), &mut state),
            Some(IdlePresenceDecision::Start {
                threshold_seconds: 300
            })
        );
        assert_eq!(evaluate_idle_presence(None, &mut state), None);
        assert_eq!(evaluate_idle_presence(Some(300.5), &mut state), None);
    }

    #[test]
    fn idle_presence_evaluation_reemits_after_returning_active() {
        let mut state = PresenceState::default();

        assert_eq!(
            evaluate_idle_presence(Some(320.2), &mut state),
            Some(IdlePresenceDecision::Start {
                threshold_seconds: 300
            })
        );
        assert_eq!(
            evaluate_idle_presence(Some(10.0), &mut state),
            Some(IdlePresenceDecision::End {
                idle_seconds: 320,
                idle_minutes: 5
            })
        );
        assert_eq!(
            evaluate_idle_presence(Some(600.0), &mut state),
            Some(IdlePresenceDecision::Start {
                threshold_seconds: 300
            })
        );
    }

    #[test]
    fn time_period_uses_four_life_periods() {
        assert_eq!(time_period_for_hour(5).as_daihon_value(), "朝");
        assert_eq!(time_period_for_hour(9).as_daihon_value(), "朝");
        assert_eq!(time_period_for_hour(10).as_daihon_value(), "昼");
        assert_eq!(time_period_for_hour(16).as_daihon_value(), "昼");
        assert_eq!(time_period_for_hour(17).as_daihon_value(), "夜");
        assert_eq!(time_period_for_hour(21).as_daihon_value(), "夜");
        assert_eq!(time_period_for_hour(22).as_daihon_value(), "深夜");
        assert_eq!(time_period_for_hour(4).as_daihon_value(), "深夜");
    }
}
