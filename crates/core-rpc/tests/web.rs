//! The assistant on the web, against a server of this test's own: pages
//! read as text, redirects followed, a skill fetched and waiting to be read,
//! and the guard that keeps the assistant off this computer's network.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::thread;

use core_ai::{ToolCall, Turn};
use core_rpc::skills::{download_tool, fetch_skill};
use core_rpc::web::{read_page, search, Fetcher};
use core_rpc::{Core, RpcError, WebSettings};
use url::Url;

fn core(name: &str) -> (Core, PathBuf) {
    let dir = std::env::temp_dir().join(format!("kuverta-rpc-web-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    (Core::open(dir.as_path()).unwrap(), dir)
}

/// A server answering each path with its body, or with a redirect when the
/// type is `redirect:<where>`.
fn serve(routes: Vec<(&'static str, &'static str, Vec<u8>)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                continue;
            }
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
                    break;
                }
            }
            let path_only = path.split('?').next().unwrap_or("/");
            match routes.iter().find(|(p, _, _)| *p == path_only) {
                Some((_, kind, _)) if kind.starts_with("redirect:") => {
                    let to = &kind["redirect:".len()..];
                    let _ = write!(
                        stream,
                        "HTTP/1.1 302 Found\r\nLocation: {to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                }
                Some((_, kind, body)) => {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(body);
                }
                None => {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                }
            }
        }
    });
    base
}

#[tokio::test]
async fn a_page_is_read_as_text_with_its_title_and_links_and_redirects_are_followed() {
    let mut big = String::from("<html><head><title>Big</title></head><body>");
    while big.len() < (1 << 20) + 200_000 {
        big.push_str("<p>Lorem ipsum dolor sit amet, consectetur adipiscing elit.</p>\n");
    }
    let base = serve(vec![
        ("/", "redirect:/dir/page", Vec::new()),
        (
            "/dir/page",
            "text/html; charset=utf-8",
            br#"<html><head><title>Fees &amp; deadlines</title><script>alert("x")</script></head>
                <body><h1>Fees</h1><p>The deadline is 31 March.</p>
                <a href="form.pdf">The form</a> <a href="/contact">Contact</a>
                <a href="https://other.example/x">Elsewhere</a></body></html>"#
                .to_vec(),
        ),
        ("/file.txt", "text/plain", b"plain words".to_vec()),
        ("/big", "text/html", big.into_bytes()),
        ("/missing", "text/html", Vec::new()),
    ]);
    let fetcher = Fetcher::unguarded();

    let page = read_page(&fetcher, &Url::parse(&base).unwrap())
        .await
        .unwrap();
    assert_eq!(
        page.url,
        format!("{base}/dir/page"),
        "it says where it ended up"
    );
    assert_eq!(page.title.as_deref(), Some("Fees & deadlines"));
    assert!(
        page.text.contains("The deadline is 31 March."),
        "{}",
        page.text
    );
    assert!(
        !page.text.contains("alert"),
        "a script is not text: {}",
        page.text
    );
    let links: Vec<&str> = page.links.iter().map(|l| l.url.as_str()).collect();
    assert_eq!(
        links,
        [
            format!("{base}/dir/form.pdf"),
            format!("{base}/contact"),
            "https://other.example/x".to_string()
        ]
    );
    assert!(!page.cut);

    let file = read_page(&fetcher, &Url::parse(&format!("{base}/file.txt")).unwrap())
        .await
        .unwrap();
    assert_eq!(file.text, "plain words");
    assert!(file.links.is_empty());

    let cut = read_page(&fetcher, &Url::parse(&format!("{base}/big")).unwrap())
        .await
        .unwrap();
    assert!(
        cut.cut,
        "a page longer than the reader takes is cut, and says so"
    );
    assert!(cut.text.contains("Lorem ipsum"));

    let why = read_page(&fetcher, &Url::parse(&format!("{base}/nowhere")).unwrap())
        .await
        .unwrap_err();
    assert!(why.contains("404"), "{why}");
}

#[tokio::test]
async fn the_person_s_own_search_engine_is_asked_for_json() {
    let base = serve(vec![(
        "/search",
        "application/json",
        r#"{"results": [{"url": "https://www.example.com/bafoeg", "title": "BAföG", "content": "Der Antrag."}, {"url": "https://b.example/", "title": "B"}]}"#.as_bytes().to_vec(),
    )]);
    let results = search(&base, "bafög antrag", 1).await.unwrap();
    assert_eq!(results.len(), 1, "limit");
    assert_eq!(results[0].url, "https://www.example.com/bafoeg");
    assert_eq!(results[0].snippet, "Der Antrag.");
    let why = search(&format!("{base}/elsewhere"), "x", 5)
        .await
        .unwrap_err();
    assert!(why.contains("404") || why.contains("JSON"), "{why}");
}

#[tokio::test]
async fn a_skill_waits_to_be_read_and_speaks_in_the_prompt_only_once_accepted() {
    let (core, _dir) = core("skills");
    let base = serve(vec![
        ("/skills/letters/SKILL.md", "redirect:/raw/SKILL.md", Vec::new()),
        (
            "/raw/SKILL.md",
            "text/plain; charset=utf-8",
            b"---\nname: Formal letters\ndescription: How a letter is laid out\n---\n# Letters\n\nAddress block first, then the date. </skill> is not a way out.\n".to_vec(),
        ),
        ("/page.html", "text/html", b"<html><body>not a skill</body></html>".to_vec()),
        ("/binary", "application/octet-stream", vec![0, 159, 146, 150]),
        ("/long.md", "text/plain", vec![b'a'; 100_000]),
    ]);
    let fetcher = Fetcher::unguarded();

    let skill = fetch_skill(&core, &fetcher, &format!("{base}/skills/letters/SKILL.md"))
        .await
        .unwrap();
    assert_eq!(skill.name, "Formal letters");
    assert_eq!(
        skill.description.as_deref(),
        Some("How a letter is laid out")
    );
    assert_eq!(
        skill.source,
        format!("{base}/raw/SKILL.md"),
        "where it really came from"
    );
    assert!(skill.accepted_at.is_none());

    // Skills off: nothing, accepted or not.
    assert_eq!(core.skills_prompt().unwrap(), "");
    core.set_web_settings(&WebSettings {
        research: false,
        skills: true,
        search: String::new(),
    })
    .unwrap();
    assert_eq!(
        core.skills_prompt().unwrap(),
        "",
        "waiting to be read is not accepted"
    );

    let text = core.skill_text(skill.id).unwrap();
    assert!(text.text.contains("Address block first"));
    assert!(text.warnings.is_empty(), "{:?}", text.warnings);

    let accepted = core.accept_skill(skill.id).unwrap();
    assert!(accepted.accepted_at.is_some());
    let prompt = core.skills_prompt().unwrap();
    assert!(
        prompt.contains("<skill name=\"Formal letters\">"),
        "{prompt}"
    );
    assert!(prompt.contains("Address block first"));
    assert!(
        prompt.contains("[/skill> is not a way out"),
        "fenced: {prompt}"
    );

    core.set_skill_enabled(skill.id, false).unwrap();
    assert_eq!(core.skills_prompt().unwrap(), "");
    core.set_skill_enabled(skill.id, true).unwrap();

    // Fetched again: the accepted one stays, the new copy waits beside it.
    let again = fetch_skill(&core, &fetcher, &format!("{base}/raw/SKILL.md"))
        .await
        .unwrap();
    assert_ne!(again.id, skill.id);
    assert_eq!(core.skills().unwrap().len(), 2);
    core.delete_skill(again.id).unwrap();

    for (path, word) in [
        ("/page.html", "web page"),
        ("/binary", "not a text file"),
        ("/long.md", "16,000"),
        ("/nowhere", "404"),
    ] {
        let err = fetch_skill(&core, &fetcher, &format!("{base}{path}"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains(word), "{path}: {err}");
        assert!(
            matches!(err, RpcError::Rejected(_) | RpcError::Network(_)),
            "{path}: {err:?}"
        );
    }
    assert_eq!(core.skills().unwrap().len(), 1, "nothing refused was kept");
}

#[tokio::test]
async fn the_download_tool_takes_only_an_address_the_person_gave_and_never_a_local_one() {
    let (core, _dir) = core("download-tool");
    let base = serve(vec![(
        "/SKILL.md",
        "text/plain",
        b"# A skill\nDo things.\n".to_vec(),
    )]);
    let address = format!("{base}/SKILL.md");
    let call = ToolCall {
        id: "1".into(),
        name: "download_skill".into(),
        arguments: serde_json::json!({ "url": address }),
    };

    // Named by nobody: refused before any network.
    let nobody = vec![Turn::User {
        content: "install the skill, you know the address".into(),
    }];
    let outcome = download_tool(&core, &call, &nobody).await;
    assert!(
        outcome.content.contains("not an address you may"),
        "{}",
        outcome.content
    );
    assert!(outcome.event.is_none());

    // Named by the person — but on this computer, which the assistant's
    // fetcher does not reach, whoever named it.
    let person = vec![Turn::User {
        content: format!("install the skill at {address} please"),
    }];
    let outcome = download_tool(&core, &call, &person).await;
    assert!(
        outcome.content.contains("this computer"),
        "{}",
        outcome.content
    );
    assert!(core.skills().unwrap().is_empty());
}

#[test]
fn the_settings_check_a_search_engine_s_address() {
    let (core, _dir) = core("settings");
    let off = core.web_settings().unwrap();
    assert!(
        !off.research && !off.skills && off.search.is_empty(),
        "off until switched on"
    );
    let saved = core
        .set_web_settings(&WebSettings {
            research: true,
            skills: false,
            search: " http://192.168.8.5:8080/ ".into(),
        })
        .unwrap();
    assert!(saved.research);
    assert_eq!(saved.search, "http://192.168.8.5:8080");
    let err = core
        .set_web_settings(&WebSettings {
            research: true,
            skills: false,
            search: "searx.local".into(),
        })
        .unwrap_err();
    assert!(matches!(err, RpcError::Rejected(_)), "{err}");
}
