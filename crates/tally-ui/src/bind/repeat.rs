use tally_app::Repeat;
use tally_core::{RepeatFrom, RepeatRule, RepeatUnit};

use crate::{strings, ui};

const UNITS: [RepeatUnit; 4] = [
    RepeatUnit::Day,
    RepeatUnit::Week,
    RepeatUnit::Month,
    RepeatUnit::Year,
];

pub(super) fn draft(repeat: Option<Repeat>) -> ui::RepeatDraft {
    let Some(repeat) = repeat else {
        return ui::RepeatDraft {
            interval: "1".into(),
            unit: 0,
            mode: 0,
        };
    };
    ui::RepeatDraft {
        interval: repeat.rule.interval.to_string().into(),
        unit: UNITS
            .iter()
            .position(|unit| *unit == repeat.rule.unit)
            .unwrap_or(0) as i32,
        mode: i32::from(repeat.from == RepeatFrom::Completion),
    }
}

pub(super) fn parse(draft: &ui::RepeatDraft) -> Result<Repeat, &'static str> {
    let interval = draft
        .interval
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|&n| n > 0)
        .ok_or(strings::REPEAT_ERROR)?;
    let unit = usize::try_from(draft.unit)
        .ok()
        .and_then(|i| UNITS.get(i))
        .copied()
        .ok_or(strings::REPEAT_ERROR)?;
    let from = if draft.mode == 1 {
        RepeatFrom::Completion
    } else {
        RepeatFrom::Date
    };
    Ok(Repeat {
        rule: RepeatRule { interval, unit },
        from,
    })
}

pub(super) fn caption(repeat: Option<Repeat>) -> String {
    let Some(Repeat { rule, .. }) = repeat else {
        return strings::REPEAT.to_owned();
    };
    let unit = match rule.unit {
        RepeatUnit::Day => "дн.",
        RepeatUnit::Week => "нед.",
        RepeatUnit::Month => "мес.",
        RepeatUnit::Year => "г.",
    };
    format!("{} · {} {unit}", strings::REPEAT, rule.interval)
}
