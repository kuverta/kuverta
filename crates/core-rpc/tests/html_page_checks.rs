//! Exercise the fuzz target's assertions in the ordinary stable test suite.
#[path = "../../../fuzz/support/html_page.rs"]
mod checks;

#[test]
fn rendered_attribute_values_do_not_trigger_the_fuzz_assertions() {
    let html = r#"<img src="https://track.shop.example.srcdocen?id=99213&amp;u=dev%40kuverta.test" alt="srcdoc onload=example javascript:" width="1" height="1"><a href="https://example.test/javascript:guide">Notes onload=example srcdoc</a>"#;
    for remote in [false, true] {
        let view = core_rpc::html_view::View {
            remote,
            open: "kuverta-mail://localhost/open/test",
            parts: "kuverta-mail://localhost/part/1/2/",
            pictures: &[],
        };
        let page = core_rpc::html_view::to_page(html, &view).unwrap();
        assert_eq!(page.remote_blocked, usize::from(!remote));
        assert_eq!(page.links, 1);
        checks::assert_safe_page(&page.html);
    }
}
