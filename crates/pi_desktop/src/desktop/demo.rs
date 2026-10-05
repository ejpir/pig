use pi_core::{
    protocol::{Command, SavedSession, read_record},
    session::Session,
};
use serde_json::{Value, json};

pub fn load(model: &mut Session, saved: Option<&SavedSession>, first: bool) {
    let fixture: Option<&[u8]> = if first {
        model.cwd = "/demo/repos/pi".into();
        Some(include_bytes!("../../../../fixtures/thread.jsonl"))
    } else if saved.is_some_and(|saved| saved.id == "demo-mistral") {
        Some(include_bytes!("../../../../fixtures/markdown.jsonl"))
    } else if saved.is_none() {
        Some(include_bytes!("../../../../fixtures/landing.jsonl"))
    } else {
        None
    };
    if let Some(fixture) = fixture {
        let mut reader = std::io::Cursor::new(fixture);
        loop {
            match read_record(&mut reader) {
                Ok(Some(record)) => {
                    if let Err(error) = model.apply(&record) {
                        model.error = Some(error.to_string());
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    model.error = Some(error.to_string());
                    break;
                }
            }
        }
        if first {
            let mut reader =
                std::io::Cursor::new(include_bytes!("../../../../fixtures/views.jsonl").as_slice());
            while let Ok(Some(record)) = read_record(&mut reader) {
                if let Err(error) = model.apply(&record) {
                    model.error = Some(error.to_string());
                    break;
                }
            }
        }
        let extra: Option<&[u8]> = if std::env::var_os("PI_DESKTOP_DEMO_READABILITY").is_some() {
            Some(include_bytes!("../../../../fixtures/readability.jsonl"))
        } else if std::env::var_os("PI_DESKTOP_DEMO_WORKSPACE").is_some() {
            Some(include_bytes!("../../../../fixtures/workspace.jsonl"))
        } else {
            None
        };
        if first && let Some(extra) = extra {
            model
                .apply(&json!({"type":"queue_update","steering":[],"followUp":[]}))
                .expect("valid empty queues");
            let mut reader = std::io::Cursor::new(extra);
            while let Ok(Some(record)) = read_record(&mut reader) {
                model.apply(&record).expect("valid workspace fixture");
            }
        }
        if first && std::env::var_os("PI_DESKTOP_DEMO_WORKBENCH").is_some() {
            model
                .apply(&json!({"type":"queue_update","steering":[],"followUp":[]}))
                .expect("sample queue");
            let mut reader = std::io::Cursor::new(
                include_bytes!("../../../../fixtures/workbench.jsonl").as_slice(),
            );
            while let Ok(Some(record)) = read_record(&mut reader) {
                model.apply(&record).expect("valid workbench fixture");
            }
            if std::env::var_os("PI_DESKTOP_DEMO_WORKBENCH_FRESH").is_some() {
                // The moment the turn starts: the request, nothing reported yet.
                let request = model.messages.first().cloned();
                model
                    .apply(&json!({"type":"response","command":"get_messages","success":true,"data":{"messages":[request]}}))
                    .expect("fresh sample");
            }
            if std::env::var_os("PI_DESKTOP_DEMO_WORKBENCH_SETTLED").is_some() {
                model.apply(&json!({"type":"tool_execution_end","toolCallId":"vision-check","isError":false,"result":{"content":[{"type":"text","text":"✓ TypeScript\n✓ Provider regression tests · 12 passed\n  Workspace checks passed"}]}})).expect("finished sample tool");
                model
                    .apply(&json!({"type":"agent_settled"}))
                    .expect("settled sample");
                model
                    .apply(&json!({"type":"queue_update","steering":[],"followUp":[]}))
                    .expect("empty sample queue");
            }
        }
    } else if let Some(saved) = saved {
        model.preview_title = Some(saved.title().to_owned());
        model.notice = Some("Offline demo — this saved session has no sample transcript.".into());
    }
}

/// Explicit offline sample snapshots, never attached to a real repository.
pub fn workspace_history(model: &Session) -> super::jj::Jj {
    use pi_jj::{ChangeId, FileChange, FileStatus, Hunk, LineKind::*};
    if model.state.session_id.as_deref() != Some("demo-workspace") {
        return Default::default();
    }
    let first = FileChange {
        path: "packages/ai/src/providers/openai-completions.ts".into(),
        status: FileStatus::Modified,
        added: 3,
        removed: 1,
        binary: false,
        hunks: vec![Hunk {
            old_start: 205,
            new_start: 205,
            lines: vec![
                (Context, "  const blocks = message.content;".into()),
                (Context, "  for (const block of blocks) {".into()),
                (Context, "    if (block.type === \"thinking\") {".into()),
                (
                    Removed,
                    "      if (!block.signature) throw new MissingSignature(model.id);".into(),
                ),
                (
                    Added,
                    "      if (block.signature === undefined && isAnthropic(model)) {".into(),
                ),
                (
                    Added,
                    "        throw new MissingSignature(model.id);".into(),
                ),
                (Added, "      }".into()),
                (
                    Context,
                    "      signatures.push(block.signature ?? \"\");".into(),
                ),
                (Context, "    }".into()),
                (Context, "  }".into()),
            ],
        }],
    };
    let lines = model.tools.iter().find(|t| t.id == "edit-2").unwrap().args["newText"]
        .as_str()
        .unwrap()
        .lines()
        .map(|l| (Added, l.to_owned()))
        .collect::<Vec<_>>();
    let second = FileChange {
        path: "packages/ai/test/openai-completions.test.ts".into(),
        status: FileStatus::Modified,
        added: lines.len(),
        removed: 0,
        binary: false,
        hunks: vec![Hunk {
            old_start: 80,
            new_start: 80,
            lines,
        }],
    };
    let mut records = vec![
        super::jj::TurnRecord::new(
            1,
            ChangeId::try_from_reverse_hex("kqxlmwsv").unwrap(),
            &[first],
        ),
        super::jj::TurnRecord::new(
            3,
            ChangeId::try_from_reverse_hex("zvtmrpyq").unwrap(),
            &[second],
        ),
    ];
    for (record, (description, ids)) in records.iter_mut().zip([
        ("Empty signatures", vec!["read-1", "edit-1", "bash-1"]),
        ("Faux-provider test", vec!["edit-2", "bash-2"]),
    ]) {
        record.description = description.into();
        record.tool_ids = ids.into_iter().map(str::to_owned).collect();
    }
    super::jj::Jj {
        records,
        ..Default::default()
    }
}

pub fn command(model: &Session, command: Command) -> Vec<Value> {
    match command {
        Command::GetBackendInfo | Command::GetAvailableModels | Command::GetAvailableThinkingLevels | Command::GetCommands | Command::GetState | Command::GetSessionStats | Command::GetSettings | Command::GetAuthProviders | Command::GetProjectTrust | Command::ListPackages => vec![],
        Command::GetEntries => model.history.as_ref().map(|h|vec![json!({"type":"response","command":"get_entries","success":true,"data":{"entries":h.entries,"leafId":h.leaf}})]).unwrap_or_else(||vec![json!({"type":"response","command":"get_entries","success":true,"data":{"entries":[],"leafId":null}})]),
        Command::GetMessages => vec![],
        Command::SetAutoCompaction {enabled} => vec![json!({"type":"response","command":"get_state","success":true,"data":{"sessionId":model.state.session_id,"sessionName":model.state.session_name,"sessionFile":model.state.session_file,"thinkingLevel":model.state.thinking_level,"model":model.state.model.as_ref().map(|m|json!({"id":m.id,"provider":m.provider,"contextWindow":m.context_window})),"autoCompactionEnabled":enabled}})],
        Command::SetLabel {target_id,label} => {
            let Some(h)=&model.history else {return vec![];}; let mut entries=h.entries.clone();entries.push(json!({"id":format!("demo-label-{}",entries.len()),"parentId":h.leaf,"type":"label","targetId":target_id,"label":label}));
            vec![json!({"type":"response","command":"get_entries","success":true,"data":{"entries":entries,"leafId":h.leaf}})]
        },
        Command::NavigateTree {target_id,summarize} => {
            if summarize {return vec![json!({"type":"extension_ui_request","method":"notify","message":"Demo does not make model calls. Uncheck summarize to explore the sample history."})];}
            let Some(h)=&model.history else {return vec![];};let Some(entry)=h.entry(&target_id) else {return vec![];};
            let user=entry["message"]["role"]=="user";let leaf=if user{entry["parentId"].clone()}else{json!(target_id)};let data=json!({"entries":h.entries,"leafId":leaf});
            let path=pi_core::history::History::parse(&data).unwrap();let messages:Vec<_>=path.path().filter(|e|e["type"]=="message").map(|e|e["message"].clone()).collect();
            vec![json!({"type":"response","command":"get_entries","success":true,"data":data}),json!({"type":"response","command":"get_messages","success":true,"data":{"messages":messages}}),json!({"type":"response","command":"navigate_tree","success":true,"data":{"cancelled":false,"editorText":if user {pi_core::history::entry_text(entry)}else{String::new()}}})]
        },
        Command::SetModel { provider, model_id, .. } => model.available_models.iter().find(|m| m.provider == provider && m.id == model_id).map(|model| vec![json!({"type":"response","command":"set_model","success":true,"data":{"id":model.id,"provider":model.provider,"contextWindow":model.context_window}})]).unwrap_or_default(),
        Command::SetThinkingLevel { level } => vec![json!({"type":"thinking_level_changed","level":level})],
        Command::SetSessionName { name, .. } => vec![json!({"type":"session_info_changed","name":name})],
        _ => vec![json!({"type":"extension_ui_request","method":"notify","message":"This needs a connected pi session."})],
    }
}
