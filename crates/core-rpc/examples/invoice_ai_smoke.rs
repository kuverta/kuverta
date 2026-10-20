//! Opt-in live test: only three synthetic documents go to the configured
//! assistant. SOURCE_DB is opened read-only; no real mail or profile is copied.
//! cargo run -p core-rpc --release --example invoice_ai_smoke -- SOURCE_DB OUTPUT_JSON
use core_rpc::{
    invoices::{
        analysis::{analyze, InvoiceAnalysisJob},
        digest,
    },
    Attachment, AttachmentView, Core, Task,
};
use core_store::{Blobs, Store};
use rusqlite::{params, Connection, OpenFlags};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let source = Connection::open_with_flags(
        args.get(1).ok_or("SOURCE_DB required")?,
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let output = args.get(2).ok_or("OUTPUT_JSON required")?;
    let core = Core::new(
        Store::open_in_memory()?,
        Blobs::new(std::env::temp_dir().join("unused-invoice-ai-smoke")),
    );
    let mut stmt =
        source.prepare("SELECT id,kind,label,base_url,key_name,created_at FROM ai_provider")?;
    for provider in stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, i64>(5)?,
        ))
    })? {
        let (id, kind, label, url, key, created) = provider?;
        core.store().connection().execute("INSERT INTO ai_provider(id,kind,label,base_url,key_name,created_at) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(id) DO UPDATE SET kind=excluded.kind,label=excluded.label,base_url=excluded.base_url,key_name=excluded.key_name",params![id,kind,label,url,key,created])?;
    }
    let (provider, model): (i64, String) = source.query_row(
        "SELECT provider_id,model FROM ai_task WHERE task='assistant'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    core.store().connection().execute(
        "INSERT INTO ai_task(task,provider_id,model) VALUES('assistant',?1,?2)",
        params![provider, model],
    )?;
    let choice = core.ai_for(Task::Assistant)?;
    let mut results = Vec::new();
    for (kind, number, amount, extra) in [
        ("Rechnung", "R-2026-42", "119,00", ""),
        (
            "Rechnung",
            "R-2026-42",
            "129,00",
            "Korrigierte Fassung 2; ersetzt die vorherige Fassung.",
        ),
        ("Gutschrift", "G-2026-7", "-10,00", ""),
    ] {
        let text=format!("Aussteller: Beispiel Handel GmbH\nEmpfänger: Muster Werkstatt GmbH\n{kind}\nBelegnummer: {number}\nRechnungsdatum: 15.09.2026\nZahlbar bis: 15.10.2026\nBruttobetrag gesamt: {amount} EUR\n{extra}\nVielen Dank.");
        let job = InvoiceAnalysisJob {
            digest: digest(text.as_bytes()),
            cached: None,
            attachment: Attachment {
                view: AttachmentView {
                    index: 0,
                    name: "beleg.txt".into(),
                    content_type: "text/plain".into(),
                    size: text.len(),
                    inline: false,
                    risky: false,
                    preview: "text".into(),
                },
                bytes: text.into_bytes(),
            },
        };
        let result = analyze(&job, &choice).await?;
        assert_eq!(
            result.invoice_date, "2026-09-15",
            "Must use document date, not payment date"
        );
        assert_eq!(result.number, number);
        assert_eq!(result.total, amount);
        assert_eq!(
            result.kind,
            if kind == "Gutschrift" {
                "credit"
            } else {
                "invoice"
            }
        );
        assert!(
            result.supported,
            "Synthetic facts should have document evidence: {result:?}"
        );
        results.push(result);
    }
    assert_eq!(results[0].identity(), results[1].identity());
    assert_ne!(results[0].identity(), results[2].identity());
    assert!(!results[1].version.is_empty());
    std::fs::write(output, serde_json::to_vec_pretty(&results)?)?;
    println!("PASS: 3 synthetic documents; dates, amounts, invoice/credit types and correction identity. Provider: {}; model: {}. No real mail read or changed.",choice.provider_label,choice.model);
    Ok(())
}
