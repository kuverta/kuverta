//! Every combination of the fragments that make this parser interesting.
//!
//! A fuzzer has to discover structure; this starts from it. The pieces are
//! the ones the reader has special cases for — quotes inside a tag, an
//! unterminated comment, a link round a block, a multi-byte character where
//! an index might land — and every ordering of a few of them is tried.

use core_rpc::html;

const PIECES: &[&str] = &[
    "<a href=",
    "<img alt=",
    "<p>",
    "</p>",
    "</a>",
    "<script",
    "</script>",
    "<!--",
    "-->",
    "<!",
    "<?",
    ">",
    "<",
    "\"",
    "'",
    "/",
    "&",
    ";",
    "#",
    "x",
    "é",
    "Ľ",
    "\u{0}",
    " ",
    "\n",
    "</h2>",
    "javascript:",
    "=",
];

fn main() {
    let deep: usize = std::env::args()
        .nth(1)
        .and_then(|n| n.parse().ok())
        .unwrap_or(4);
    let mut tried = 0u64;
    let mut index = vec![0usize; deep];
    loop {
        let mut input = String::new();
        for &at in &index {
            input.push_str(PIECES[at]);
        }
        // Panics take the process down, which is the point: the input is
        // printed first so the one that did it is on the screen.
        let _ = std::panic::catch_unwind(|| html::to_text(&input))
            .unwrap_or_else(|_| panic!("PANICKED ON {input:?}"));
        tried += 1;

        let mut place = deep;
        loop {
            if place == 0 {
                println!("{tried} combinations of {deep} pieces, no panic");
                return;
            }
            place -= 1;
            index[place] += 1;
            if index[place] < PIECES.len() {
                break;
            }
            index[place] = 0;
        }
    }
}
