use super::*;

const MIN: i64 = 60_000;
const DAY: i64 = 86_400_000;
const NOW: i64 = 20_800 * DAY;

/// `n` prompts at `ts`, the first `c` of them corrections.
fn prompts(ts: i64, n: usize, c: usize, pending: bool) -> Vec<Prompt> {
    (0..n)
        .map(|i| Prompt {
            ts: ts + i as i64,
            agent: "claude".into(),
            correction: i < c,
            pending,
        })
        .collect()
}

fn data(ps: Vec<Vec<Prompt>>) -> Data {
    Data {
        prompts: ps.into_iter().flatten().collect(),
        ..Data::default()
    }
}

fn utc(_: i64) -> i64 {
    0
}

fn facts_of(d: &Data, weeks: i64) -> Vec<String> {
    facts(&Inputs {
        data: d,
        judged: true,
        breaks: &[],
        now: NOW,
        weeks,
        off: &utc,
        on: &|_| true,
    })
}

fn starting<'a>(out: &'a [String], prefix: &str) -> Option<&'a str> {
    out.iter()
        .find(|l| l.starts_with(prefix))
        .map(String::as_str)
}

#[test]
fn the_average_reads_as_one_in_n_and_needs_a_hundred_prompts() {
    let d = data(vec![prompts(NOW - DAY, 120, 20, false)]);
    assert_eq!(
        starting(&facts_of(&d, 4), "You correct"),
        Some("You correct about 1 in 6 prompts: 17 per 100 (20 of 120).")
    );
    let few = data(vec![prompts(NOW - DAY, 99, 20, false)]);
    assert!(facts_of(&few, 4).is_empty());
    let none = data(vec![prompts(NOW - DAY, 100, 0, false)]);
    assert_eq!(
        facts_of(&none, 4),
        ["None of your 100 prompts was a correction."]
    );
    let most = data(vec![prompts(NOW - DAY, 100, 70, false)]);
    assert_eq!(
        facts_of(&most, 4),
        ["70 of every 100 prompts are corrections (70 of 100)."]
    );
}

#[test]
fn prompts_waiting_for_the_pass_are_left_out_of_every_rate() {
    // the earlier fortnight is judged at 20 per 100; the last one has 100 judged at 20 per 100 and
    // 300 still waiting, which counted as no corrections would read as a fall to 5
    let d = data(vec![
        prompts(NOW - 20 * DAY, 200, 40, false),
        prompts(NOW - 5 * DAY, 100, 20, false),
        prompts(NOW - DAY, 300, 0, true),
    ]);
    let out = facts_of(&d, 4);
    assert_eq!(
        starting(&out, "You correct"),
        Some("You correct about 1 in 5 prompts: 20 per 100 (60 of 300).")
    );
    assert_eq!(
        starting(&out, "Last"),
        Some("Last 2 weeks: 20 per 100, the same as in the 2 weeks before.")
    );
}

#[test]
fn a_day_the_pass_left_half_done_is_left_out_whole() {
    // the pass marks what its word filter drops as no correction before it asks about the rest; when
    // the asking fails, the day holds 80 such prompts and 20 possible corrections still waiting
    let d = data(vec![
        prompts(NOW - 20 * DAY, 100, 20, false),
        prompts(NOW - 5 * DAY, 100, 20, false),
        prompts(NOW - DAY, 80, 0, false),
        prompts(NOW - DAY + 1_000, 20, 0, true),
    ]);
    assert_eq!(
        starting(&facts_of(&d, 4), "You correct"),
        Some("You correct about 1 in 5 prompts: 20 per 100 (40 of 200).")
    );
    // a prompt of an agent that is off waits for good: its day counts with what was judged
    let off = facts(&Inputs {
        data: &d,
        judged: true,
        breaks: &[],
        now: NOW,
        weeks: 4,
        off: &utc,
        on: &|a| a != "claude",
    });
    assert_eq!(
        starting(&off, "You correct"),
        Some("You correct about 1 in 7 prompts: 14 per 100 (40 of 280).")
    );
}

#[test]
fn the_later_half_of_the_window_is_set_against_the_earlier_half() {
    let at = |recent: usize| {
        data(vec![
            prompts(NOW - 20 * DAY, 200, 40, false),
            prompts(NOW - 3 * DAY, 200, recent, false),
        ])
    };
    assert_eq!(
        starting(&facts_of(&at(20), 4), "Last"),
        Some("Last 2 weeks: 10 per 100, down from 20 in the 2 weeks before.")
    );
    assert_eq!(
        starting(&facts_of(&at(60), 4), "Last"),
        Some("Last 2 weeks: 30 per 100, up from 20 in the 2 weeks before.")
    );
    // three weeks: the last one against the two before it; one week has no halves
    assert_eq!(
        starting(&facts_of(&at(20), 3), "Last"),
        Some("Last week: 10 per 100, down from 20 in the 2 weeks before.")
    );
    assert_eq!(starting(&facts_of(&at(20), 1), "Last"), None);
    // a gap that chance makes often is not called a change: 9 against 14 in 100 each
    let noise = data(vec![
        prompts(NOW - 20 * DAY, 100, 14, false),
        prompts(NOW - 3 * DAY, 100, 9, false),
    ]);
    assert_eq!(
        starting(&facts_of(&noise, 4), "Last"),
        Some("Last 2 weeks: 9 per 100, against 14 in the 2 weeks before: not a clear change yet.")
    );
    // a half under a hundred prompts says nothing
    let thin = data(vec![
        prompts(NOW - 20 * DAY, 99, 20, false),
        prompts(NOW - 3 * DAY, 200, 20, false),
    ]);
    assert_eq!(starting(&facts_of(&thin, 4), "Last"), None);
}

#[test]
fn breaks_are_said_to_help_only_when_the_difference_is_more_than_chance() {
    let brk = NOW - 3 * DAY;
    let with = |later: usize| {
        let d = data(vec![
            prompts(brk + 10 * MIN, 200, 20, false),
            prompts(brk + 90 * MIN, 200, later, false),
        ]);
        facts(&Inputs {
            data: &d,
            judged: true,
            breaks: &[brk],
            now: NOW,
            weeks: 4,
            off: &utc,
            on: &|_| true,
        })
    };
    assert_eq!(
        starting(&with(60), "Breaks"),
        Some("Breaks help: an hour or more after one, you correct 30 per 100, against 10 in the first hour.")
    );
    assert_eq!(
        starting(&with(24), "An hour"),
        Some("An hour or more after a break you correct 12 per 100, against 10 in the first hour: not a clear difference yet.")
    );
    // a clear fall after the first hour is said as one, not as no difference
    assert_eq!(
        starting(&with(2), "An hour"),
        Some("An hour or more after a break you correct 1 per 100, fewer than the 10 of the first hour after one.")
    );
}

#[test]
fn with_corrections_not_judged_only_the_stops_are_given() {
    let mut d = data(vec![prompts(NOW - DAY, 300, 30, false)]);
    d.interrupts = (0..10).map(|i| NOW - DAY + i).collect();
    let out = facts(&Inputs {
        data: &d,
        judged: false,
        breaks: &[],
        now: NOW,
        weeks: 4,
        off: &utc,
        on: &|_| true,
    });
    assert_eq!(
        out,
        ["You stopped the agent mid-answer 10 times, about once every 30 prompts."]
    );
    d.interrupts.truncate(1);
    assert_eq!(
        starting(&facts_of(&d, 4), "You stopped"),
        Some("You stopped the agent mid-answer once in 300 prompts.")
    );
    d.interrupts.clear();
    assert_eq!(starting(&facts_of(&d, 4), "You stopped"), None);
}
