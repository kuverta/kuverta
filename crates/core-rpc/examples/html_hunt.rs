//! The fragments that make this parser interesting, in every order.
//!
//! A fuzzer has to discover structure; this starts from it. The pieces are
//! the ones the reader has special cases for — quotes inside a tag, an
//! unterminated comment, a link round a block, an element whose content is
//! skipped unread, a multi-byte character where an index might land.
//!
//! `html_hunt <n>` tries every ordering of `n` pieces, which is exhaustive
//! and short. `html_hunt random <longest> <tries>` builds longer ones, which
//! is where a crash of a hundred-odd bytes lives.

use core_rpc::html;

const PIECES: &[&str] = &[
    "<a href=",
    "<img alt=",
    "<p>",
    "</p>",
    "</a>",
    "<script",
    "</script>",
    "<style",
    "</style>",
    "<title",
    "</title>",
    "<head",
    "</head>",
    "<noscript",
    "</noscript>",
    "<template",
    "</template>",
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
    "\r",
    "\t",
    "</h2>",
    "javascript:",
    "=",
    "&#",
    "&#x",
    "&lt",
    "&amp",
    "<td>",
    "<li>",
    "<br>",
    "<h2>",
    "<table>",
    "alt=",
    "href=",
    "[",
    "]",
];

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("random") => {
            let longest: usize = args.next().and_then(|n| n.parse().ok()).unwrap_or(24);
            let tries: u64 = args
                .next()
                .and_then(|n| n.parse().ok())
                .unwrap_or(5_000_000);
            random(longest, tries);
        }
        other => {
            let deep: usize = other.and_then(|n| n.parse().ok()).unwrap_or(4);
            exhaustive(deep);
        }
    }
}

fn try_one(input: &str) {
    let _ = std::panic::catch_unwind(|| html::to_text(input))
        .unwrap_or_else(|_| panic!("PANICKED ON {input:?}"));
}

fn exhaustive(deep: usize) {
    let mut tried = 0u64;
    let mut index = vec![0usize; deep];
    loop {
        let mut input = String::new();
        for &at in &index {
            input.push_str(PIECES[at]);
        }
        try_one(&input);
        tried += 1;

        let mut place = deep;
        loop {
            if place == 0 {
                println!("{tried} orderings of {deep} pieces, no panic");
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

/// xorshift, so the run is repeatable and there is no dependency for it.
fn random(longest: usize, tries: u64) {
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut input = String::new();
    for round in 0..tries {
        input.clear();
        let pieces = 2 + (next() as usize) % longest;
        for _ in 0..pieces {
            input.push_str(PIECES[(next() as usize) % PIECES.len()]);
        }
        try_one(&input);
        if round % 1_000_000 == 0 && round > 0 {
            println!("{round} tried, longest {longest} pieces, no panic yet");
        }
    }
    println!("{tries} random builds of up to {longest} pieces, no panic");
}
