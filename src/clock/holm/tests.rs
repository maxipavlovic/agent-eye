use super::*;

#[test]
fn a_lone_test_needs_the_plain_threshold() {
    assert_eq!(survivors(&[Some(0.09)]), [true]);
    assert_eq!(survivors(&[Some(0.1)]), [false]);
    assert_eq!(survivors(&[]), Vec::<bool>::new());
}

#[test]
fn the_smallest_p_must_beat_the_threshold_over_the_family() {
    // four tests: 0.1 / 4 = 0.025 for the smallest
    assert_eq!(
        survivors(&[Some(0.03), Some(0.5), None, Some(0.9), Some(0.6)]),
        [false; 5]
    );
    assert_eq!(
        survivors(&[Some(0.02), Some(0.5), None, Some(0.9), Some(0.6)]),
        [true, false, false, false, false]
    );
}

#[test]
fn it_steps_down_and_stops_at_the_first_failure() {
    // 0.001 beats 0.1/3, 0.04 beats 0.1/2 and 0.06 beats 0.1/1
    assert_eq!(
        survivors(&[Some(0.06), Some(0.001), Some(0.04)]),
        [true, true, true]
    );
    // 0.04 fails 0.1/3 when it is the smallest, so the larger ones fail too
    assert_eq!(
        survivors(&[Some(0.04), Some(0.05), Some(0.06)]),
        [false, false, false]
    );
}

#[test]
fn tests_that_could_not_run_do_not_count() {
    assert_eq!(survivors(&[None, Some(0.04), None]), [false, true, false]);
}
