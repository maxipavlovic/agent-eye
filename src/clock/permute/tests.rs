use super::*;

/// 24 hours of `per_hour` prompts each, every prompt corrected with the chance `rate(h)`.
fn week(rng: &mut Rng, per_hour: i64, rate: impl Fn(usize) -> f64) -> Vec<Hour> {
    (0..24)
        .map(|h| Hour {
            h,
            prompts: per_hour,
            corrections: (0..per_hour).filter(|_| rng.unit() < rate(h)).count() as i64,
            interrupts: 0,
            tool_calls: 0,
            tool_errors: 0,
            think: None,
            think_n: 0,
        })
        .collect()
}

#[test]
fn the_generator_is_seeded_and_stays_in_range() {
    let (mut a, mut b) = (Rng::new(7), Rng::new(7));
    let drawn: Vec<usize> = (0..50).map(|_| a.below(10)).collect();
    assert_eq!(drawn, (0..50).map(|_| b.below(10)).collect::<Vec<_>>());
    assert!(drawn.iter().all(|d| *d < 10) && drawn.iter().any(|d| *d != drawn[0]));
    assert!((0..100).all(|_| (0.0..1.0).contains(&a.unit())));
}

fn passes(t: Option<BandTest>) -> bool {
    t.is_some_and(|t| t.p < bands::MAX_P)
}

/// The bar a gate faces in a report that holds three (`holm`): the report's rate over a few gates,
/// so a gate on its own at this bar names noise in at most one world in ten, with room to spare.
const IN_A_FAMILY_OF_THREE: f64 = bands::MAX_P / 3.0;

#[test]
fn an_evening_that_runs_twice_as_bad_passes() {
    let mut rng = Rng::new(1);
    let hours = week(&mut rng, 150, |h| {
        if (18..21).contains(&h) {
            0.16
        } else {
            0.08
        }
    });
    let t = band_test(&hours, SEED).expect("a real effect");
    assert!((1.5..3.0).contains(&t.ratio), "{}", t.ratio);
    assert!(t.p < bands::MAX_P, "{}", t.p);
}

#[test]
fn hours_that_are_all_alike_give_nothing() {
    let flat: Vec<Hour> = week(&mut Rng::new(1), 60, |_| 0.0)
        .into_iter()
        .map(|h| Hour {
            corrections: 5,
            ..h
        })
        .collect();
    assert_eq!(band_test(&flat, SEED), None);
    assert_eq!(band_test(&[], SEED), None);
}

#[test]
fn the_same_data_gives_the_same_answer_every_time() {
    let hours = week(&mut Rng::new(3), 60, |h| if h == 19 { 0.2 } else { 0.08 });
    assert_eq!(band_test(&hours, SEED), band_test(&hours, SEED));
}

/// The test the whole file is for: with no hour worse than another, the hours must not be named
/// more than one time in ten, though they are picked because they look worst. (Testing the picked
/// hours with a plain two-proportion test, as before, named them in more than half.)
#[test]
fn with_no_effect_hours_are_named_in_one_week_of_ten_at_most() {
    let mut rng = Rng::new(20_261_126);
    let trials = 300;
    let named = (0..trials)
        .filter(|_| {
            band_test(&week(&mut rng, 60, |_| 0.08), SEED)
                .is_some_and(|t| t.p < IN_A_FAMILY_OF_THREE)
        })
        .count();
    assert!(named * 100 <= trials * 10, "{named} of {trials}");
}

#[test]
fn with_a_real_double_effect_it_is_found() {
    let mut rng = Rng::new(20_261_127);
    let trials = 100;
    let found = (0..trials)
        .filter(|_| {
            let hours = week(&mut rng, 150, |h| {
                if (19..22).contains(&h) {
                    0.16
                } else {
                    0.08
                }
            });
            passes(band_test(&hours, SEED))
        })
        .count();
    assert!(found * 100 >= trials * 90, "{found} of {trials}");
}

fn thoughts(rng: &mut Rng, n: usize, scale: f64) -> Vec<f64> {
    // skewed like real thinking sizes: a few long thoughts among many short ones
    (0..n)
        .map(|_| scale * (1.0 + 4.0 * rng.unit().powi(3)))
        .collect()
}

#[test]
fn thinking_that_does_not_move_is_rarely_called_moved() {
    let mut rng = Rng::new(11);
    let trials = 300;
    let moved = (0..trials)
        .filter(|_| {
            let (a, b) = (thoughts(&mut rng, 40, 1.0), thoughts(&mut rng, 80, 1.0));
            median_shift_p(&a, &b, SEED).is_some_and(|p| p < IN_A_FAMILY_OF_THREE)
        })
        .count();
    assert!(moved * 100 <= trials * 10, "{moved} of {trials}");
}

#[test]
fn thinking_that_moved_a_third_is_found() {
    let mut rng = Rng::new(12);
    let (a, b) = (thoughts(&mut rng, 150, 0.65), thoughts(&mut rng, 300, 1.0));
    assert!(median_shift_p(&a, &b, SEED).unwrap() < bands::MAX_P);
    assert_eq!(median_shift_p(&[], &b, SEED), None);
    assert_eq!(median_shift_p(&a, &[0.0; 4], SEED), None);
}

#[test]
fn failures_in_bursts_are_dealt_out_by_the_turn_not_the_call() {
    // 10 calls a turn; one turn in six fails all ten in the hours, one in ten elsewhere. Counted
    // call by call that is 17% against 10% on 240 and 4,000 calls: a pattern. As turns it is 4 of
    // 24 against 40 of 400, which chance deals out all the time.
    let burst = (10, 10);
    let fine = (0, 10);
    let inside: Vec<_> = (0..24).map(|i| if i < 4 { burst } else { fine }).collect();
    let outside: Vec<_> = (0..400)
        .map(|i| if i < 40 { burst } else { fine })
        .collect();
    let sum = |t: &[(i64, i64)]| t.iter().fold((0, 0), |a, x| (a.0 + x.0, a.1 + x.1));
    let by_call = bands::proportion_p(sum(&inside), sum(&outside)).unwrap();
    assert!(by_call < 0.01, "{by_call}");
    assert_eq!(errors_p(&inside, &outside, SEED), Some(1.0));
}

#[test]
fn a_higher_failure_rate_in_every_turn_of_the_hours_is_a_pattern() {
    let inside = vec![(3, 10); 40];
    let outside = vec![(1, 10); 400];
    let p = errors_p(&inside, &outside, SEED).unwrap();
    assert!(p < 0.01, "{p}");
}

#[test]
fn failure_rates_that_cannot_be_compared_give_no_p() {
    assert_eq!(errors_p(&[(0, 10)], &[(0, 10)], SEED), None);
    assert_eq!(errors_p(&[], &[(1, 10)], SEED), None);
}
