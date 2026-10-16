use super::*;
use serde_json::json;

async fn call(server: &Server, method: &'static str, params: Value) -> Value {
    let response = server
        .handle_request(
            "wire-test",
            JsonRpcRequest::new(json!(1), method, Some(params)),
        )
        .await
        .unwrap();
    assert!(response.error.is_none(), "{response:?}");
    response.result.unwrap()
}

#[tokio::test]
async fn skills_prompts_and_resources_work_on_both_protocol_versions() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join(".skills")).unwrap();
    std::fs::write(
        directory.path().join("AGENTS.md.j2"),
        "Instructions fixture",
    )
    .unwrap();
    std::fs::write(
        directory.path().join(".skills/migration_probe.j2"),
        "---\nname: Migration probe\n---\nSkill fixture",
    )
    .unwrap();

    for version in ["2025-06-18", "2025-11-25"] {
        let helmsman = HelmsmanServer::new(
            &Config::default(),
            TemplateEngine::new(directory.path()).unwrap(),
        );
        let server = Server::builder("helmsman-test", "1")
            .with_prompts(true)
            .with_resources(true, false)
            .build();
        server
            .register_router(HelmsmanServer::router().unwrap(), Arc::new(helmsman))
            .unwrap();
        let initialized = call(
            &server,
            "initialize",
            json!({
                "protocolVersion": version,
                "capabilities": {},
                "clientInfo": {"name": "wire-test", "version": "1"}
            }),
        )
        .await;
        assert_eq!(initialized["protocolVersion"], version);
        server
            .handle_request(
                "wire-test",
                JsonRpcRequest::notification("notifications/initialized", None),
            )
            .await;

        let resources = call(&server, "resources/list", json!({})).await;
        assert!(
            resources["resources"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["uri"] == "skill:///")
        );
        let index = call(&server, "resources/read", json!({"uri": "skill:///"})).await;
        assert_eq!(index["contents"][0]["mimeType"], "application/json");
        let templates = call(&server, "resources/templates/list", json!({})).await;
        assert!(
            templates["resourceTemplates"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["uriTemplate"] == "skill:///{name}")
        );
        let skill = call(
            &server,
            "resources/read",
            json!({"uri": "skill:///migration_probe"}),
        )
        .await;
        assert!(
            skill["contents"][0]["text"]
                .as_str()
                .unwrap()
                .contains("Skill fixture")
        );
        let prompt = call(
            &server,
            "prompts/get",
            json!({"name": "skill", "arguments": {"name": "migration_probe"}}),
        )
        .await;
        assert!(
            prompt["messages"][0]["content"]["text"]
                .as_str()
                .unwrap()
                .contains("Skill fixture")
        );
        let instructions = call(
            &server,
            "prompts/get",
            json!({"name": "instructions", "arguments": {"model_id": DEFAULT_MODEL_ID}}),
        )
        .await;
        assert_eq!(
            instructions["messages"][0]["content"]["text"],
            "Instructions fixture"
        );

        for arguments in [
            json!({}),
            json!({"model_id": DEFAULT_MODEL_ID, "unknown": "value"}),
        ] {
            let response = server
                .handle_request(
                    "wire-test",
                    JsonRpcRequest::new(
                        json!(2),
                        "prompts/get",
                        Some(json!({"name": "instructions", "arguments": arguments})),
                    ),
                )
                .await
                .unwrap();
            assert_eq!(response.error.unwrap().code, ErrorCode::INVALID_PARAMS);
        }
    }
}
