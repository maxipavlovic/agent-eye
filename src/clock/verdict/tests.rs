use super::*;
use crate::clock::signals::Think;

const H: i64 = 3_600_000;

fn utc(_: i64) -> i64 {
    0
}
fn evening() -> Vec<bool> {
    (0..24).map(|h| (18..24).contains(&h)).collect()
}
const EVENING: [Range; 1] = [Range {
    from_h: 18,
    to_h: 0,
}];
const FLAT: ModelSide = ModelSide {
    think_ratio: Some(0.95),
    tool_error_ratio: Some(1.0),
    think_moved: false,
    tool_errors_moved: false,
};
const SHALLOW: ModelSide = ModelSide {
    think_ratio: Some(0.8),
    tool_error_ratio: Some(1.0),
    think_moved: true,
    tool_errors_moved: false,
};
const BLIND: ModelSide = ModelSide {
    think_ratio: None,
    tool_error_ratio: None,
    think_moved: false,
    tool_errors_moved: false,
};

fn kind(prompts: i64, corrections: i64, you: bool, band: Option<ModelSide>) -> Verdict {
    decide(&Findings {
        prompts,
        corrections,
        not_judged: None,
        you: you.then_some((1.8, 105)),
        band: band.as_ref().map(|m| (&EVENING[..], *m)),
    })
}

fn think(h: i64, agent: &str, n: i64) -> Think {
    Think {
        ts: h * H,
        agent: agent.into(),
        n,
    }
}

/// A call in hour `h`, of turn `turn`.
fn call(h: i64, agent: &str, turn: i64) -> Call {
    Call {
        ts: h * H,
        agent: agent.into(),
        turn,
    }
}

fn model_of(d: &Data) -> ModelSide {
    test_band(d, &utc, &evening(), None).model
}

/// `n` thoughts of `value` in hour `h`.
fn thoughts(h: i64, agent: &str, value: i64, n: usize) -> Vec<Think> {
    (0..n).map(|_| think(h, agent, value)).collect()
}

/// `calls` tool calls of `agent` in hour `h`, `errors` of them failed, each in a turn of its own.
fn tools(data: &mut Data, h: i64, agent: &str, calls: usize, errors: usize) {
    let first = data.tools.len() as i64;
    data.tools
        .extend((0..calls as i64).map(|i| call(h, agent, first + i)));
    data.tool_errors
        .extend((0..errors as i64).map(|i| call(h, agent, first + i)));
}

#[test]
fn the_model_side_compares_each_agent_with_itself() {
    let mut d = Data::default();
    // the claude values are large and the codex ones small: only the ratio counts
    for (agent, evening, day) in [("claude", 800, 1000), ("codex", 8, 10)] {
        d.thinks.extend(thoughts(19, agent, evening, 40));
        d.thinks.extend(thoughts(10, agent, day, 40));
        d.thinks.extend(thoughts(11, agent, day, 40));
    }
    let m = model_of(&d);
    assert!((m.think_ratio.unwrap() - 0.8).abs() < 1e-9);
    assert!(m.moved() && m.measured());
    let none = model_of(&Data::default());
    assert_eq!(
        (
            none.think_ratio,
            none.tool_error_ratio,
            none.moved(),
            none.measured()
        ),
        (None, None, false, false)
    );
}

#[test]
fn thinking_needs_enough_thoughts_on_both_sides() {
    let mut d = Data::default();
    d.thinks
        .extend(thoughts(19, "claude", 500, MIN_THINK_N - 1));
    d.thinks.extend(thoughts(10, "claude", 1000, 200));
    assert_eq!(model_of(&d).think_ratio, None);
    d.thinks.extend(thoughts(19, "claude", 500, 1));
    assert!(model_of(&d).think_ratio.is_some());
    let mut few_outside = Data::default();
    few_outside.thinks.extend(thoughts(19, "claude", 500, 200));
    few_outside.thinks.extend(thoughts(10, "claude", 1000, 5));
    assert_eq!(model_of(&few_outside).think_ratio, None);
}

#[test]
fn a_thinking_ratio_that_chance_explains_is_not_the_model_moving() {
    // 30 thoughts a side, 16 long ones inside and 14 outside: the medians are 30% apart, which
    // shuffling the same thoughts over the hours does about as well as
    let mut d = Data::default();
    d.thinks.extend(thoughts(19, "claude", 1300, 16));
    d.thinks.extend(thoughts(19, "claude", 1000, 14));
    d.thinks.extend(thoughts(10, "claude", 1300, 14));
    d.thinks.extend(thoughts(10, "claude", 1000, 16));
    let m = model_of(&d);
    assert!((m.think_ratio.unwrap() - 1.3).abs() < 1e-9);
    assert!(!m.think_moved && !m.moved());
    let v = decide(&Findings {
        prompts: 900,
        corrections: 90,
        not_judged: None,
        you: Some((1.8, 105)),
        band: Some((&EVENING[..], m)),
    });
    assert!(v.text.contains("Model thinking not clearly different."));
    assert_eq!(v.kind, Kind::You);
}

#[test]
fn a_handful_of_errors_is_not_the_model_moving() {
    // 3 errors in 40 calls against 100 in 4000: ratio 3.0, but not enough events to say
    let mut d = Data::default();
    tools(&mut d, 19, "claude", 40, 3);
    tools(&mut d, 10, "claude", 4000, 100);
    let m = model_of(&d);
    assert!(m.tool_error_ratio.unwrap() > 2.9);
    assert!(!m.tool_errors_moved && !m.moved());
    // the same rates on ten times the data are
    let mut big = Data::default();
    tools(&mut big, 19, "claude", 400, 30);
    tools(&mut big, 10, "claude", 4000, 100);
    assert!(model_of(&big).tool_errors_moved);
}

#[test]
fn a_big_difference_between_agents_is_not_the_hours() {
    // codex runs only in the evening and fails 10%, claude runs all day and fails 2%: within
    // each agent the evening is the same as the rest, so nothing moved
    let mut d = Data::default();
    tools(&mut d, 19, "codex", 400, 40);
    tools(&mut d, 19, "claude", 400, 8);
    tools(&mut d, 10, "claude", 4000, 80);
    let m = model_of(&d);
    assert!(!m.tool_errors_moved);
    let codex = test_band(&d, &utc, &evening(), None)
        .sides
        .into_iter()
        .find(|s| s.agent == "codex")
        .unwrap();
    // codex has no calls outside the hours: no ratio of its own
    assert_eq!(codex.tool_error_ratio, None);
    assert!((m.tool_error_ratio.unwrap() - 1.0).abs() < 1e-9);
}

#[test]
fn the_agent_whose_errors_moved_speaks_for_the_band() {
    let mut d = Data::default();
    tools(&mut d, 19, "claude", 400, 40);
    tools(&mut d, 10, "claude", 4000, 100);
    tools(&mut d, 19, "codex", 100, 5);
    tools(&mut d, 10, "codex", 1000, 50);
    let m = model_of(&d);
    assert!(m.tool_errors_moved);
    assert!((m.tool_error_ratio.unwrap() - 4.0).abs() < 1e-9);
    let sides = test_band(&d, &utc, &evening(), None).sides;
    let flags: Vec<_> = sides
        .iter()
        .map(|s| (s.agent.as_str(), s.tool_errors_moved))
        .collect();
    assert_eq!(flags, [("claude", true), ("codex", false)]);
}

#[test]
fn every_kind_of_verdict() {
    assert_eq!(kind(299, 100, true, Some(FLAT)).kind, Kind::TooFew);
    assert_eq!(kind(900, 29, true, Some(FLAT)).kind, Kind::TooFew);
    assert_eq!(kind(900, 90, true, Some(FLAT)).kind, Kind::You);
    assert_eq!(kind(900, 90, false, Some(SHALLOW)).kind, Kind::Model);
    assert_eq!(kind(900, 90, true, Some(SHALLOW)).kind, Kind::Both);
    assert_eq!(kind(900, 90, false, Some(FLAT)).kind, Kind::None);
    assert_eq!(kind(900, 90, false, None).kind, Kind::None);
    assert_eq!(kind(900, 90, true, Some(BLIND)).kind, Kind::Unmeasured);
    assert_eq!(kind(900, 90, false, Some(BLIND)).kind, Kind::None);
    assert!(Kind::Unmeasured.names_hours() && !Kind::None.names_hours());
}

#[test]
fn the_text_has_the_counts_in_plain_words() {
    assert_eq!(
        kind(2231, 205, true, Some(ModelSide { think_ratio: Some(0.92), tool_error_ratio: Some(0.91), think_moved: false, tool_errors_moved: false })).text,
        "Evenings: 1.8x more corrections (105). Model thinking \u{2212}8%. Tool errors flat. Mostly you."
    );
    let t = kind(
        900,
        90,
        false,
        Some(ModelSide {
            think_ratio: Some(1.0),
            tool_error_ratio: Some(1.5),
            think_moved: false,
            tool_errors_moved: true,
        }),
    )
    .text;
    assert_eq!(
        t,
        "Evenings: your corrections not clearly different. Model thinking unchanged. Tool errors +50%. Mostly the model."
    );
    assert!(kind(900, 90, true, Some(SHALLOW))
        .text
        .ends_with("Both moved."));
    assert_eq!(
        kind(900, 90, false, None).text,
        "No clear pattern in 900 prompts and 90 corrections: no hours stand out clearly."
    );
    assert_eq!(
        kind(120, 12, false, None).text,
        "Too few to tell yet: 120 prompts and 12 corrections (it needs 300 and 30)."
    );
}

#[test]
fn nothing_measured_is_said_and_never_called_flat() {
    let t = kind(900, 90, true, Some(BLIND)).text;
    assert_eq!(
        t,
        "Evenings: 1.8x more corrections (105). No model-side data, so it cannot be said whether it was you or the model."
    );
    assert!(!t.contains("flat"));
    let one = ModelSide {
        think_ratio: None,
        tool_error_ratio: Some(1.0),
        think_moved: false,
        tool_errors_moved: false,
    };
    assert_eq!(
        kind(900, 90, true, Some(one)).text,
        "Evenings: 1.8x more corrections (105). No thinking data. Tool errors flat. Mostly you."
    );
    assert!(kind(900, 90, false, Some(BLIND))
        .text
        .ends_with("the model's side has no data."));
    // a ratio out of the flat range that chance explains is not called flat either
    let loose = ModelSide {
        think_ratio: Some(1.0),
        tool_error_ratio: Some(3.0),
        think_moved: false,
        tool_errors_moved: false,
    };
    assert!(kind(900, 90, true, Some(loose))
        .text
        .contains("Tool errors not clearly different."));
}

#[test]
fn with_judging_off_the_text_says_so() {
    let v = decide(&Findings {
        prompts: 900,
        corrections: 0,
        not_judged: Some("[clock] llm is off"),
        you: None,
        band: None,
    });
    assert_eq!(v.kind, Kind::TooFew);
    assert!(v.text.contains("llm is off"));
}

#[test]
fn too_few_names_the_real_reason_judging_is_off() {
    let text = |why: Option<&str>| {
        decide(&Findings {
            prompts: 900,
            corrections: 0,
            not_judged: why,
            you: None,
            band: None,
        })
        .text
    };
    assert_eq!(
        text(Some("[summary] enabled is off, and corrections follow it")),
        "Corrections are not being judged: [summary] enabled is off, and corrections follow it, so there is nothing to compare."
    );
    // never blamed on `[clock] llm` when something else is the reason
    assert!(!text(Some("the steering clock is off ([clock] enabled)")).contains("llm"));
    assert!(text(None).starts_with("Too few to tell yet"));
}

#[test]
fn hours_are_named_by_one_rule() {
    let names: Vec<_> = [0, 4, 5, 11, 12, 16, 17, 21, 22, 23]
        .map(part_of_day)
        .into();
    assert_eq!(
        names,
        [
            "night",
            "night",
            "morning",
            "morning",
            "afternoon",
            "afternoon",
            "evening",
            "evening",
            "night",
            "night"
        ]
    );
    assert_eq!(
        range_label(&Range {
            from_h: 18,
            to_h: 2
        }),
        "evening"
    );
    assert_eq!(
        range_label(&Range {
            from_h: 23,
            to_h: 3
        }),
        "night"
    );
}

#[test]
fn several_runs_are_named_by_their_hours() {
    let two = [
        Range { from_h: 2, to_h: 4 },
        Range {
            from_h: 22,
            to_h: 1,
        },
    ];
    assert_eq!(label(&two), "Hours 02:00-04:00 and 22:00-01:00");
    assert_eq!(
        label(&[Range {
            from_h: 9,
            to_h: 12
        }]),
        "Mornings"
    );
    assert_eq!(
        label(&[Range {
            from_h: 13,
            to_h: 15
        }]),
        "Afternoons"
    );
    assert_eq!(label(&[Range { from_h: 1, to_h: 5 }]), "Nights");
}

/// Thinking that is the same in and out of the hours, `n` thoughts each.
fn flat_thinking(d: &mut Data, n: usize) {
    d.thinks.extend(thoughts(19, "claude", 1000, n));
    d.thinks.extend(thoughts(10, "claude", 1000, n));
}

#[test]
fn every_gate_that_exists_is_in_the_family_whatever_the_data_shows() {
    // your p of 0.06 beats 0.1 alone, but not 0.1 over two gates (yours and the thinking, which
    // is flat: counted with a p of 1, not left out)
    let mut d = Data::default();
    flat_thinking(&mut d, 60);
    assert!(!test_band(&d, &utc, &evening(), Some(0.06)).you);
    assert!(test_band(&d, &utc, &evening(), Some(0.04)).you);
    // the same with no thoughts at all: the gate is still there, with a p of 1
    assert!(!test_band(&Data::default(), &utc, &evening(), Some(0.06)).you);
    // an agent that makes tool calls adds its own gate, flat or not: 0.04 needs 0.1 / 3
    tools(&mut d, 19, "claude", 40, 2);
    tools(&mut d, 10, "claude", 400, 20);
    assert!(!test_band(&d, &utc, &evening(), Some(0.04)).you);
    assert!(test_band(&d, &utc, &evening(), Some(0.03)).you);
    // a band that was not tested for your corrections has no p: it counts as 1, not as absent
    assert!(!test_band(&d, &utc, &evening(), None).you);
}
