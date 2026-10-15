//! Where models run, and which model each job uses.

use core_store::{NewAiProvider, Store, StoredAiTask};

fn fresh(name: &str) -> (Store, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("kuverta-ai-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    (Store::open(dir.join("test.db")).unwrap(), dir)
}

fn deepseek() -> NewAiProvider {
    NewAiProvider {
        kind: "openai".into(),
        label: "DeepSeek".into(),
        base_url: "https://api.deepseek.com/".into(),
    }
}

#[test]
fn a_new_store_runs_models_on_the_local_ollama_until_told_otherwise() {
    let (store, dir) = fresh("new");

    let providers = store.ai_providers().unwrap();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].kind, "ollama");
    assert_eq!(providers[0].base_url, "http://127.0.0.1:11434");
    assert_eq!(providers[0].key_name.len(), 32, "{}", providers[0].key_name);
    assert!(
        store.ai_tasks().unwrap().is_empty(),
        "every job on its default"
    );

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_hosted_service_is_added_and_edited_in_place_with_its_key_where_it_was() {
    let (store, dir) = fresh("edit");

    let id = store.add_ai_provider(&deepseek()).unwrap();
    let added = store.ai_provider(id).unwrap().unwrap();
    assert_eq!(added.base_url, "https://api.deepseek.com");
    assert_ne!(added.key_name, store.ai_providers().unwrap()[0].key_name);

    let changed = store
        .update_ai_provider(
            id,
            &NewAiProvider {
                label: "DeepSeek (work)".into(),
                base_url: " https://api.deepseek.com/v1/ ".into(),
                ..deepseek()
            },
        )
        .unwrap();
    assert!(changed);
    let after = store.ai_provider(id).unwrap().unwrap();
    assert_eq!(after.label, "DeepSeek (work)");
    assert_eq!(after.base_url, "https://api.deepseek.com/v1");
    assert_eq!(after.key_name, added.key_name, "the key stays findable");

    assert!(!store.update_ai_provider(9999, &deepseek()).unwrap());

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_job_has_one_model_and_can_go_back_to_its_default() {
    let (store, dir) = fresh("tasks");
    let hosted = store.add_ai_provider(&deepseek()).unwrap();

    store.set_ai_task("vision", 1, "qwen2.5vl:3b").unwrap();
    store
        .set_ai_task("vision", hosted, "some-vl-model")
        .unwrap();
    assert_eq!(
        store.ai_tasks().unwrap(),
        vec![StoredAiTask {
            task: "vision".into(),
            provider_id: hosted,
            model: "some-vl-model".into(),
        }]
    );

    store.clear_ai_task("vision").unwrap();
    assert!(store.ai_tasks().unwrap().is_empty());

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn removing_a_provider_sends_its_jobs_back_to_their_defaults_and_leaves_the_rest() {
    let (store, dir) = fresh("delete");
    let hosted = store.add_ai_provider(&deepseek()).unwrap();
    store
        .set_ai_task("vision", hosted, "some-vl-model")
        .unwrap();
    store.set_ai_task("chat", 1, "llama3.2:3b").unwrap();

    store.delete_ai_provider(hosted).unwrap();

    assert!(store.ai_provider(hosted).unwrap().is_none());
    let tasks = store.ai_tasks().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].task, "chat");

    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn claude_is_a_kind_of_its_own_and_making_room_for_it_keeps_every_job() {
    let (store, dir) = fresh("claude");
    let hosted = store.add_ai_provider(&deepseek()).unwrap();
    store.set_ai_task("chat", hosted, "deepseek-chat").unwrap();
    // Back to before the provider table was rebuilt, so that opening the
    // store rebuilds it with a job pointing into it.
    store
        .connection()
        .execute_batch("PRAGMA user_version = 23;")
        .unwrap();
    drop(store);
    let store = Store::open(dir.join("test.db")).unwrap();

    assert_eq!(store.ai_providers().unwrap().len(), 2);
    assert_eq!(store.ai_tasks().unwrap()[0].provider_id, hosted);
    let claude = store
        .add_ai_provider(&NewAiProvider {
            kind: "anthropic".into(),
            label: "Claude".into(),
            base_url: "https://api.anthropic.com/v1".into(),
        })
        .unwrap();
    assert_eq!(
        store.ai_provider(claude).unwrap().unwrap().kind,
        "anthropic"
    );
    let fk: i64 = store
        .connection()
        .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
        .unwrap();
    assert_eq!(fk, 1, "foreign keys are back on");

    let _ = std::fs::remove_dir_all(dir);
}
