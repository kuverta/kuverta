//! What the e-paper display says, and when it is redrawn — without a panel.

use std::sync::mpsc;

use anyhow::Result;
use scannerd::display::{
    drive, render, Facts, Frame, Panel, Screen, FULL_EVERY, HEIGHT, ROW_BYTES, WIDTH,
};
use scannerd::folders::{Filings, Folder, Outcome};
use scannerd::quality::{Problem, Quality};
use scannerd::run::Photographed;

fn facts(state: &str) -> Facts<'_> {
    Facts {
        now: 1_000,
        state,
        finishing: false,
        collecting: true,
        open_pages: 0,
        waiting: 0,
        last: None,
        filing: None,
        touch: false,
        scanning: true,
        live: None,
        notice: None,
        finishing_in: None,
        can_undo_letter: false,
        can_refile: false,
        confirming: None,
        queue: &[],
        showing_queue: false,
    }
}

/// A letter sent at 900, and what Paperless made of it at 950.
fn filed(outcome: Outcome) -> Filings {
    let mut filings = Filings::default();
    filings.sent("letter.pdf", "task", 900);
    if outcome != Outcome::Reading {
        filings.decide("task", outcome, 950);
    }
    filings
}

fn photographed(at: u64, problems: Vec<Problem>) -> Photographed {
    Photographed {
        at,
        page: Some(2),
        quality: Some(Quality {
            ink: 30,
            paper: 220,
            sharpness: Some(0.9),
            problems,
        }),
        thumbnail: None,
        envelope: false,
        kept: true,
    }
}

#[test]
fn an_empty_table_is_ready() {
    let screen = Screen::for_facts(&facts("waiting"));
    assert_eq!(screen.headline, "Ready");
    assert_eq!(screen.detail, "Put a page down");
    assert_eq!(screen.verdict, None);
    assert_eq!(screen.footer, "All sent");
}

#[test]
fn a_good_photograph_says_so() {
    let last = photographed(990, vec![]);
    let screen = Screen::for_facts(&Facts {
        last: Some(&last),
        open_pages: 2,
        ..facts("photographed")
    });
    assert!(screen.buttons.is_empty(), "no touch, no buttons");
    assert_eq!(screen.headline, "Scanned");
    assert_eq!(screen.detail, "Next page, or take it away");
    assert_eq!(screen.verdict, Some((true, "Page 2: good to read".into())));
    assert_eq!(screen.footer, "Letter: 2 pages  ·  All sent");
}

#[test]
fn a_bad_photograph_says_what_is_wrong() {
    let last = photographed(990, vec![Problem::WashedOut]);
    let screen = Screen::for_facts(&Facts {
        last: Some(&last),
        ..facts("photographed")
    });
    assert_eq!(screen.headline, "Check scan");
    assert_eq!(screen.verdict, Some((false, "Page 2: too bright".into())));
}

#[test]
fn the_verdict_stays_up_while_the_next_page_is_awaited_but_not_for_ever() {
    let last = photographed(990, vec![]);
    let ready = Screen::for_facts(&Facts {
        last: Some(&last),
        ..facts("waiting")
    });
    assert_eq!(ready.headline, "Ready");
    assert!(ready.verdict.is_some());

    let later = Screen::for_facts(&Facts {
        now: 990 + 3_600,
        last: Some(&last),
        ..facts("waiting")
    });
    assert_eq!(later.verdict, None);
}

#[test]
fn a_photograph_that_could_not_be_judged_has_no_verdict() {
    let last = Photographed {
        at: 990,
        page: None,
        quality: None,
        thumbnail: None,
        envelope: false,
        kept: true,
    };
    let screen = Screen::for_facts(&Facts {
        last: Some(&last),
        ..facts("photographed")
    });
    assert_eq!(screen.headline, "Scanned");
    assert_eq!(screen.verdict, None);
}

#[test]
fn the_camera_and_the_queue_are_reported() {
    let screen = Screen::for_facts(&Facts {
        waiting: 3,
        ..facts("no-camera")
    });
    assert_eq!(screen.headline, "No camera");
    assert_eq!(screen.footer, "3 waiting to send");

    let finishing = Screen::for_facts(&Facts {
        finishing: true,
        ..facts("settling")
    });
    assert_eq!(finishing.headline, "Sending letter");
}

#[test]
fn between_letters_the_screen_says_which_folder() {
    let filings = filed(Outcome::Folders(vec![Folder::named("Car")]));
    let screen = Screen::for_facts(&Facts {
        filing: filings.latest(),
        ..facts("waiting")
    });
    assert_eq!(screen.headline, "Car");
    assert_eq!(screen.detail, "Put the letter in this folder");
    assert_eq!(screen.tone, scannerd::display::Tone::Folder);
    assert_eq!(screen.footer, "Ready for the next letter  ·  All sent");
}

#[test]
fn the_screen_says_who_the_letter_is_for_as_well_as_where_it_goes() {
    // A household where more than one person gets post: the folder alone does
    // not say whose pile the letter joins.
    let filings = filed(Outcome::Folders(vec![
        Folder::named("Taxes"),
        Folder::person("Erika Mustermann"),
    ]));
    let screen = Screen::for_facts(&Facts {
        filing: filings.latest(),
        ..facts("waiting")
    });
    assert_eq!(screen.headline, "Taxes", "the folder, not the person");
    assert_eq!(screen.detail, "For Erika Mustermann");

    // Post for two — an estate, a couple's insurer — names both.
    let both = filed(Outcome::Folders(vec![
        Folder::named("Taxes"),
        Folder::person("Erika Mustermann"),
        Folder::person("Max Mustermann"),
    ]));
    let screen = Screen::for_facts(&Facts {
        filing: both.latest(),
        ..facts("waiting")
    });
    assert_eq!(screen.headline, "Taxes");
    assert_eq!(screen.detail, "For Erika Mustermann and Max Mustermann");

    // A letter for somebody that fits no folder still says whose it is.
    let who_only = filed(Outcome::Folders(vec![Folder::person("Max Mustermann")]));
    let screen = Screen::for_facts(&Facts {
        filing: who_only.latest(),
        ..facts("waiting")
    });
    assert_eq!(screen.headline, "Which folder?");
    assert_eq!(screen.detail, "For Max Mustermann");
}

#[test]
fn a_letter_just_sent_can_be_filed_by_hand_from_the_display() {
    // Paperless cannot know that a letter is worth keeping but belongs on no
    // shelf, or that the paper can go: whoever is holding it does.
    let filings = filed(Outcome::Folders(vec![Folder::named("Car")]));
    let screen = Screen::for_facts(&Facts {
        filing: filings.latest(),
        can_refile: true,
        touch: true,
        scanning: false,
        ..facts("waiting")
    });
    let labels: Vec<String> = screen.buttons.iter().map(|b| b.label()).collect();
    assert!(labels.contains(&"No folder".to_string()), "{labels:?}");
    assert!(labels.contains(&"Throw away".to_string()), "{labels:?}");

    // While scanning, the two answers are there instead of "undo": the
    // question in front of whoever is standing there is where this one goes.
    let scanning = Screen::for_facts(&Facts {
        filing: filings.latest(),
        can_refile: true,
        can_undo_letter: true,
        scanning: true,
        touch: true,
        ..facts("waiting")
    });
    let labels: Vec<String> = scanning.buttons.iter().map(|b| b.label()).collect();
    assert!(labels.contains(&"No folder".to_string()), "{labels:?}");
    assert!(
        !labels.contains(&"Undo last letter".to_string()),
        "{labels:?}"
    );
    // Five at most, or they do not fit above the picture.
    assert!(scanning.buttons.len() <= 5, "{labels:?}");
}

#[test]
fn a_letter_filed_in_no_folder_says_so_rather_than_asking_which() {
    let filings = filed(Outcome::Nowhere);
    let screen = Screen::for_facts(&Facts {
        filing: filings.latest(),
        ..facts("waiting")
    });
    assert_eq!(screen.headline, "No folder");
    assert_eq!(screen.detail, "Kept in Paperless, on no shelf");
    assert_eq!(screen.tone, scannerd::display::Tone::Done);
}

#[test]
fn the_bin_says_throw_away_whatever_else_matched() {
    let bin = Folder {
        discard: true,
        ..Folder::named("Bin")
    };
    let filings = filed(Outcome::Folders(vec![Folder::named("Car"), bin]));
    let screen = Screen::for_facts(&Facts {
        filing: filings.latest(),
        ..facts("waiting")
    });
    assert_eq!(screen.headline, "Throw away");
    assert_eq!(screen.tone, scannerd::display::Tone::Bin);
}

#[test]
fn a_letter_still_being_read_says_so_and_no_folder_says_that() {
    let reading = filed(Outcome::Reading);
    let screen = Screen::for_facts(&Facts {
        filing: reading.latest(),
        ..facts("waiting")
    });
    assert_eq!(screen.headline, "Sorting...");

    let none = filed(Outcome::Folders(vec![]));
    let screen = Screen::for_facts(&Facts {
        filing: none.latest(),
        ..facts("waiting")
    });
    assert_eq!(screen.headline, "Which folder?");
}

#[test]
fn the_next_letter_takes_the_screen_and_the_folder_moves_to_the_footer() {
    let filings = filed(Outcome::Folders(vec![Folder::named("Taxes")]));
    let screen = Screen::for_facts(&Facts {
        filing: filings.latest(),
        open_pages: 1,
        ..facts("waiting")
    });
    assert_eq!(screen.headline, "Ready");
    assert_eq!(screen.footer, "Letter: 1 page  ·  Last: Taxes  ·  All sent");

    let settling = Screen::for_facts(&Facts {
        filing: filings.latest(),
        ..facts("settling")
    });
    assert_eq!(settling.headline, "Hold still");
}

#[test]
fn a_folder_is_not_shown_for_ever() {
    let filings = filed(Outcome::Folders(vec![Folder::named("Car")]));
    let screen = Screen::for_facts(&Facts {
        now: 950 + 3_600,
        filing: filings.latest(),
        ..facts("waiting")
    });
    assert_eq!(screen.headline, "Ready");
    assert_eq!(screen.footer, "All sent");
}

fn black_in(frame: &Frame, x: std::ops::Range<u32>, y: std::ops::Range<u32>) -> usize {
    y.flat_map(|y| x.clone().map(move |x| (x, y)))
        .filter(|&(x, y)| frame.is_black(x, y))
        .count()
}

#[test]
fn a_frame_is_the_panels_size_and_mostly_white() {
    let frame = render(&Screen::for_facts(&facts("waiting")), false);
    assert_eq!(frame.bytes.len(), ROW_BYTES * WIDTH as usize);
    let black = black_in(&frame, 0..WIDTH, 0..HEIGHT);
    assert!(black > 200, "the words are drawn: {black}");
    assert!(black < (WIDTH * HEIGHT / 4) as usize, "and not much else");
    // The headline sits top left.
    assert!(black_in(&frame, 0..120, 0..34) > 100);
}

#[test]
fn bad_news_is_white_on_black() {
    let last = photographed(990, vec![Problem::Blurry]);
    let bad = render(
        &Screen::for_facts(&Facts {
            last: Some(&last),
            ..facts("photographed")
        }),
        false,
    );
    let bar = black_in(&bad, 0..WIDTH, 60..90);
    assert!(bar > (WIDTH * 30 * 2 / 3) as usize, "{bar}");

    let last = photographed(990, vec![]);
    let good = render(
        &Screen::for_facts(&Facts {
            last: Some(&last),
            ..facts("photographed")
        }),
        false,
    );
    assert!(black_in(&good, 0..WIDTH, 60..90) < (WIDTH * 30 / 4) as usize);
}

#[test]
fn flipping_turns_the_picture_round() {
    let screen = Screen::for_facts(&facts("waiting"));
    let (upright, flipped) = (render(&screen, false), render(&screen, true));
    assert_ne!(upright.bytes, flipped.bytes);
    for (x, y) in [(10, 10), (40, 20), (100, 50), (200, 110)] {
        assert_eq!(upright.is_black(x, y), flipped.is_black(x, y));
    }
}

#[test]
fn text_too_long_for_the_panel_is_shortened_not_spilled() {
    let screen = Screen {
        headline: "A headline far too long for a panel of this size".into(),
        verdict: Some((
            false,
            "no page / too dark, too bright, blurry, no text found".into(),
        )),
        ..Screen::default()
    };
    let frame = render(&screen, false);
    // The last few columns stay as the layout leaves them: white above the
    // bar, black in it.
    assert_eq!(black_in(&frame, WIDTH - 4..WIDTH, 0..58), 0);
    assert_eq!(black_in(&frame, WIDTH - 4..WIDTH, 60..90), 4 * 30);
}

/// A panel that says what it was asked to do, as it does it.
struct Recorder(mpsc::Sender<bool>);

impl Panel for Recorder {
    fn show(&mut self, _frame: &Frame, full: bool) -> Result<()> {
        self.0.send(full).unwrap();
        Ok(())
    }
}

#[test]
fn the_panel_is_only_touched_when_the_picture_changes() {
    let (refreshed, refreshes) = mpsc::channel();
    let (screens, receiver) = mpsc::channel();
    let ready = Screen::for_facts(&facts("waiting"));
    let hold = Screen::for_facts(&facts("settling"));
    let driver = std::thread::spawn(move || drive(Box::new(Recorder(refreshed)), receiver, false));

    screens.send(ready.clone()).unwrap();
    assert!(
        refreshes.recv().unwrap(),
        "the first picture is a full refresh"
    );
    screens.send(ready.clone()).unwrap();
    screens.send(hold.clone()).unwrap();
    assert!(!refreshes.recv().unwrap(), "a change is a partial one");
    screens.send(hold).unwrap();
    drop(screens);
    driver.join().unwrap();
    assert_eq!(refreshes.try_iter().count(), 0, "repeats touch nothing");
}

#[test]
fn ghosts_are_cleared_with_a_full_refresh_now_and_then() {
    let (refreshed, refreshes) = mpsc::channel();
    let (screens, receiver) = mpsc::channel();
    let driver = std::thread::spawn(move || drive(Box::new(Recorder(refreshed)), receiver, false));
    let mut fulls = Vec::new();
    for n in 0..(FULL_EVERY + 3) {
        screens
            .send(Screen {
                headline: format!("{n}"),
                ..Screen::default()
            })
            .unwrap();
        if refreshes.recv().unwrap() {
            fulls.push(n);
        }
    }
    drop(screens);
    driver.join().unwrap();
    assert_eq!(fulls, vec![0, FULL_EVERY + 1]);
}

// -- a display you can touch ---------------------------------------------------------

use scannerd::display::Action;
use scannerd::picture::Picture;

#[test]
fn stopped_the_screen_offers_to_start_and_to_learn_the_table() {
    let screen = Screen::for_facts(&Facts {
        touch: true,
        scanning: false,
        ..facts("paused")
    });
    assert_eq!(screen.headline, "Ready to scan");
    assert_eq!(
        screen.buttons,
        vec![Action::StartScanning, Action::LearnEmpty]
    );
    assert!(screen.scanning.is_none());
}

#[test]
fn stopped_after_a_letter_the_screen_says_its_folder_above_the_buttons() {
    let filings = filed(Outcome::Folders(vec![Folder::named("Car")]));
    let screen = Screen::for_facts(&Facts {
        touch: true,
        scanning: false,
        filing: filings.latest(),
        ..facts("paused")
    });
    assert_eq!(screen.headline, "Car");
    assert_eq!(
        screen.buttons,
        vec![Action::StartScanning, Action::LearnEmpty]
    );
}

#[test]
fn scanning_shows_the_camera_and_the_last_page_and_offers_to_finish() {
    let live = Picture::scaled(&[128; 16], 4, 4, 4, 4);
    let mut last = photographed(990, vec![]);
    last.thumbnail = Some(Picture::scaled(&[200; 6], 2, 3, 2, 3));
    let screen = Screen::for_facts(&Facts {
        touch: true,
        scanning: true,
        open_pages: 2,
        live: Some(live.clone()),
        last: Some(&last),
        ..facts("photographed")
    });
    assert_eq!(screen.headline, "Scanned - next page?");
    assert_eq!(
        screen.buttons,
        vec![
            Action::FinishLetter(2),
            Action::UndoPage,
            Action::CancelLetter(2)
        ]
    );
    let panes = screen.scanning.expect("panes");
    assert_eq!(panes.live, Some(live));
    assert_eq!(panes.last, last.thumbnail);
    assert_eq!(screen.verdict, Some((true, "Page 2: good to read".into())));
}

#[test]
fn scanning_with_nothing_photographed_yet_only_offers_to_stop() {
    let screen = Screen::for_facts(&Facts {
        touch: true,
        scanning: true,
        ..facts("waiting")
    });
    assert_eq!(screen.headline, "Put a page down");
    assert_eq!(screen.buttons, vec![Action::StopScanning]);
    assert_eq!(Action::FinishLetter(1).label(), "Finish letter (1 page)");
}

#[test]
fn a_button_that_worked_or_did_not_says_so_on_the_home_screen() {
    let screen = Screen::for_facts(&Facts {
        touch: true,
        scanning: false,
        notice: Some((false, "Could not see the table - no camera?")),
        ..facts("paused")
    });
    assert_eq!(screen.detail, "Could not see the table - no camera?");
    assert_eq!(screen.tone, scannerd::display::Tone::Problem);
}

#[test]
fn scanning_the_top_line_says_what_came_of_a_button_for_a_moment() {
    let screen = Screen::for_facts(&Facts {
        touch: true,
        scanning: true,
        notice: Some((true, "Learnt the empty table")),
        ..facts("waiting")
    });
    assert_eq!(screen.headline, "Learnt the empty table");
}

#[test]
fn an_envelope_says_so_and_the_last_letter_stays_in_sight() {
    let mut last = photographed(990, vec![Problem::NoText]);
    last.envelope = true;
    let filings = filed(Outcome::Folders(vec![Folder::named("Taxes")]));
    let screen = Screen::for_facts(&Facts {
        touch: true,
        scanning: true,
        open_pages: 1,
        last: Some(&last),
        filing: filings.latest(),
        ..facts("photographed")
    });
    assert_eq!(screen.headline, "Envelope - now the letter in it");
    assert_eq!(
        screen.verdict,
        Some((true, "Envelope: a new letter".into()))
    );
    assert_eq!(
        screen.last_letter,
        Some((scannerd::display::Tone::Folder, "Last letter: Taxes".into()))
    );
}

#[test]
fn a_letter_still_being_read_is_sorting_in_the_strip() {
    let filings = filed(Outcome::Reading);
    let screen = Screen::for_facts(&Facts {
        touch: true,
        scanning: true,
        filing: filings.latest(),
        ..facts("waiting")
    });
    assert_eq!(
        screen.last_letter,
        Some((
            scannerd::display::Tone::Busy,
            "Last letter: sorting...".into()
        ))
    );
}

#[test]
fn cancelling_a_letter_asks_for_a_second_tap() {
    use scannerd::display::Confirming;
    let screen = Screen::for_facts(&Facts {
        touch: true,
        scanning: true,
        open_pages: 3,
        confirming: Some(Confirming::Cancel),
        ..facts("waiting")
    });
    assert_eq!(
        screen.buttons,
        vec![
            Action::FinishLetter(3),
            Action::UndoPage,
            Action::ConfirmCancel(3)
        ]
    );
    assert_eq!(
        Action::ConfirmCancel(3).label(),
        "Tap again: throw 3 pages away"
    );
}

#[test]
fn a_letter_just_finished_can_be_taken_back_between_letters() {
    use scannerd::display::Confirming;
    let scanning = Screen::for_facts(&Facts {
        touch: true,
        scanning: true,
        can_undo_letter: true,
        ..facts("waiting")
    });
    assert_eq!(
        scanning.buttons,
        vec![Action::UndoLetter, Action::StopScanning]
    );
    let stopped = Screen::for_facts(&Facts {
        touch: true,
        scanning: false,
        can_undo_letter: true,
        confirming: Some(Confirming::UndoLetter),
        ..facts("paused")
    });
    assert_eq!(
        stopped.buttons,
        vec![
            Action::StartScanning,
            Action::LearnEmpty,
            Action::ConfirmUndoLetter
        ]
    );
}

// -- what is waiting to be sent ----------------------------------------------------------

fn waiting(letters: &[(u64, u32)]) -> Vec<scannerd::hub::Queued> {
    letters
        .iter()
        .enumerate()
        .map(|(i, (at, attempts))| scannerd::hub::Queued {
            name: format!("{at}-{i}.pdf"),
            attempts: *attempts,
            at: *at,
        })
        .collect()
}

#[test]
fn what_is_waiting_can_be_opened_from_the_screen() {
    let queue = waiting(&[(1_790_000_000, 0)]);
    let screen = Screen::for_facts(&Facts {
        touch: true,
        scanning: true,
        queue: &queue,
        ..facts("waiting")
    });
    assert_eq!(
        screen.buttons,
        vec![Action::OpenQueue(1), Action::StopScanning]
    );
    assert_eq!(Action::OpenQueue(1).label(), "1 waiting to send");
    assert_eq!(Action::OpenQueue(4).label(), "4 waiting to send");
}

#[test]
fn the_list_has_a_row_a_letter_and_a_way_back() {
    // Six waiting: four fit the panel.
    let queue = waiting(&[
        (1_790_000_000, 0),
        (1_790_000_100, 1),
        (1_790_000_200, 4),
        (1_790_000_300, 0),
        (1_790_000_400, 0),
        (1_790_000_500, 0),
    ]);
    let screen = Screen::for_facts(&Facts {
        touch: true,
        scanning: true,
        showing_queue: true,
        queue: &queue,
        ..facts("waiting")
    });
    assert_eq!(screen.headline, "6 waiting to send");
    assert_eq!(screen.detail, "Tap a letter to throw it away; 2 more below");
    assert_eq!(
        screen.buttons,
        vec![
            Action::QueuedLetter(0),
            Action::QueuedLetter(1),
            Action::QueuedLetter(2),
            Action::QueuedLetter(3),
            Action::Back,
        ]
    );
    assert_eq!(screen.queue.len(), 4);
    assert!(
        screen.queue[0].ends_with("not tried yet"),
        "{:?}",
        screen.queue[0]
    );
    assert!(
        screen.queue[1].ends_with("tried once"),
        "{:?}",
        screen.queue[1]
    );
    assert!(
        screen.queue[2].ends_with("tried 4×"),
        "{:?}",
        screen.queue[2]
    );
}

#[test]
fn throwing_one_away_asks_for_a_second_tap() {
    use scannerd::display::Confirming;
    let queue = waiting(&[(1_790_000_000, 0), (1_790_000_100, 0)]);
    let screen = Screen::for_facts(&Facts {
        touch: true,
        scanning: true,
        showing_queue: true,
        confirming: Some(Confirming::Queued(1)),
        queue: &queue,
        ..facts("waiting")
    });
    assert_eq!(
        screen.buttons,
        vec![
            Action::QueuedLetter(0),
            Action::ConfirmQueued(1),
            Action::Back
        ]
    );
}

#[test]
fn an_empty_queue_has_no_list_to_show() {
    let screen = Screen::for_facts(&Facts {
        touch: true,
        scanning: true,
        showing_queue: true,
        ..facts("waiting")
    });
    assert_eq!(screen.buttons, vec![Action::StopScanning]);
}

#[test]
fn the_list_says_the_time_of_day_where_the_pi_is() {
    use chrono::TimeZone;
    // Noon in Hamburg in summer is 10:00 UTC; the display says noon.
    let at = chrono::Utc
        .with_ymd_and_hms(2026, 9, 23, 10, 0, 0)
        .single()
        .unwrap()
        .timestamp() as u64;
    let queue = waiting(&[(at, 0)]);
    let screen = Screen::for_facts(&Facts {
        touch: true,
        scanning: true,
        showing_queue: true,
        queue: &queue,
        ..facts("waiting")
    });
    let expected = chrono::Local
        .timestamp_opt(at as i64, 0)
        .single()
        .unwrap()
        .format("%H:%M")
        .to_string();
    assert!(
        screen.queue[0].starts_with(&expected),
        "{:?}",
        screen.queue[0]
    );
}

#[test]
fn a_photograph_of_the_table_says_it_was_not_kept() {
    let mut last = photographed(990, vec![Problem::NoText]);
    last.kept = false;
    let screen = Screen::for_facts(&Facts {
        touch: true,
        scanning: true,
        last: Some(&last),
        ..facts("photographed")
    });
    assert_eq!(screen.headline, "That was the table - not kept");
    assert_eq!(
        screen.verdict,
        Some((false, "Not kept: nothing on it".into()))
    );
}
