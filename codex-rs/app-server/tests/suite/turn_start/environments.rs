use super::*;

#[tokio::test]
async fn turn_start_resolves_sticky_thread_local_environment_and_turn_overrides() -> Result<()> {
    let tmp = TempDir::new()?;
    let codex_home = tmp.path().join("codex_home");
    std::fs::create_dir(&codex_home)?;
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir(&workspace)?;

    let server = create_mock_responses_server_repeating_assistant("done").await;
    create_config_toml(&codex_home, &server.uri(), "never", &BTreeMap::default())?;
    std::fs::write(
        codex_home.join("environments.toml"),
        r#"
[[environments]]
id = "remote"
url = "ws://127.0.0.1:1"
"#,
    )?;

    let mut mcp = McpProcess::new(&codex_home).await?;
    timeout(DEFAULT_READ_TIMEOUT, mcp.initialize()).await??;

    for case in [
        EnvironmentSelectionCase {
            name: "sticky_unset_turn_unset",
            sticky: None,
            turn: None,
        },
        EnvironmentSelectionCase {
            name: "sticky_empty_turn_unset",
            sticky: Some(&[]),
            turn: None,
        },
        EnvironmentSelectionCase {
            name: "sticky_local_turn_unset",
            sticky: Some(&["local"]),
            turn: None,
        },
        EnvironmentSelectionCase {
            name: "sticky_local_turn_empty",
            sticky: Some(&["local"]),
            turn: Some(&[]),
        },
        EnvironmentSelectionCase {
            name: "sticky_empty_turn_local",
            sticky: Some(&[]),
            turn: Some(&["local"]),
        },
    ] {
        run_environment_selection_case(&mut mcp, &workspace, case).await?;
    }

    Ok(())
}

struct EnvironmentSelectionCase {
    name: &'static str,
    sticky: Option<&'static [&'static str]>,
    turn: Option<&'static [&'static str]>,
}

async fn run_environment_selection_case(
    mcp: &mut McpProcess,
    workspace: &Path,
    case: EnvironmentSelectionCase,
) -> Result<()> {
    let thread_req = mcp
        .send_thread_start_request(ThreadStartParams {
            model: Some("mock-model".to_string()),
            cwd: Some(workspace.to_string_lossy().into_owned()),
            environments: environment_params(case.sticky, workspace)?,
            ..Default::default()
        })
        .await?;
    let thread_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(thread_req)),
    )
    .await??;
    let ThreadStartResponse { thread, .. } = to_response::<ThreadStartResponse>(thread_resp)?;

    let turn_req = mcp
        .send_turn_start_request(TurnStartParams {
            thread_id: thread.id.clone(),
            input: vec![V2UserInput::Text {
                text: format!("run {}", case.name),
                text_elements: Vec::new(),
            }],
            environments: environment_params(case.turn, workspace)?,
            cwd: Some(workspace.to_path_buf()),
            model: Some("mock-model".to_string()),
            ..Default::default()
        })
        .await?;
    let turn_resp: JSONRPCResponse = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_response_message(RequestId::Integer(turn_req)),
    )
    .await??;
    let TurnStartResponse { turn } = to_response::<TurnStartResponse>(turn_resp)?;

    let started_notification = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/started"),
    )
    .await??;
    let started: TurnStartedNotification = serde_json::from_value(
        started_notification
            .params
            .ok_or_else(|| anyhow::anyhow!("turn/started notification should include params"))?,
    )?;
    assert_eq!(started.turn.id, turn.id, "{}", case.name);

    let completed_notification = timeout(
        DEFAULT_READ_TIMEOUT,
        mcp.read_stream_until_notification_message("turn/completed"),
    )
    .await??;
    let completed: TurnCompletedNotification =
        serde_json::from_value(completed_notification.params.ok_or_else(|| {
            anyhow::anyhow!("turn/completed notification should include params")
        })?)?;
    assert_eq!(completed.turn.id, turn.id, "{}", case.name);
    assert_eq!(
        completed.turn.status,
        TurnStatus::Completed,
        "{}",
        case.name
    );

    mcp.clear_message_buffer();

    Ok(())
}

fn environment_params(
    ids: Option<&[&str]>,
    cwd: &Path,
) -> Result<Option<Vec<TurnEnvironmentParams>>> {
    ids.map(|ids| {
        ids.iter()
            .map(|id| {
                Ok(TurnEnvironmentParams {
                    environment_id: (*id).to_string(),
                    cwd: cwd.to_path_buf().try_into()?,
                })
            })
            .collect()
    })
    .transpose()
}
